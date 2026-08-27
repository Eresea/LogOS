#![no_std]

use core::{mem, ptr};

use logos_abi::{
    AtriumApp, AtriumSurfaceInput, AtriumSurfaceRequest, AtriumSurfaceResponse, GUI_DRAW_FLAG_MORE,
    GuiDrawBatch, GuiSceneOp, GuiStatus, IPC_PAGE_BYTES, IPC_STAGING_BASE, IPC_SYSCALL_RECEIVE,
    IPC_SYSCALL_SEND, IpcStatus, MAX_RENDER_CELLS, MessageKind, PROGRAM_BOOTSTRAP_BASE,
    ProgramBootstrapPage, RENDER_FLAG_MORE, RenderMessage, SurfaceHandle,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgramClientError {
    InvalidBootstrap,
    Busy,
    NoPendingRequest,
    NoSurface,
    SurfaceMismatch,
    InvalidPayload,
    Protocol,
    Surface(GuiStatus),
    Ipc(IpcStatus),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceEvent {
    Created(SurfaceHandle),
    Revoked(SurfaceHandle),
}

pub struct ProgramClient {
    client: logos_abi::ServiceHandle,
    surface_request: logos_abi::CapabilityHandle,
    surface_response: logos_abi::CapabilityHandle,
    surface_input: logos_abi::CapabilityHandle,
    surface_render: logos_abi::CapabilityHandle,
    surface_draw: logos_abi::CapabilityHandle,
    next_request_id: u32,
    pending_request: Option<AtriumSurfaceRequest>,
    request_sent: bool,
    surface: SurfaceHandle,
    draw_frame: u32,
}

impl ProgramClient {
    pub fn from_bootstrap(bootstrap: ProgramBootstrapPage) -> Result<Self, ProgramClientError> {
        if !bootstrap.is_valid() {
            return Err(ProgramClientError::InvalidBootstrap);
        }
        Ok(Self {
            client: bootstrap.client,
            surface_request: bootstrap.surface_request,
            surface_response: bootstrap.surface_response,
            surface_input: bootstrap.surface_input,
            surface_render: bootstrap.surface_render,
            surface_draw: bootstrap.surface_draw,
            next_request_id: 1,
            pending_request: None,
            request_sent: false,
            surface: SurfaceHandle::EMPTY,
            draw_frame: 0,
        })
    }

    /// Read the read-only bootstrap page mapped into the program address space.
    ///
    /// # Safety
    ///
    /// The caller must be running as a LogOS program with the fixed bootstrap
    /// mapping installed at [`logos_abi::PROGRAM_BOOTSTRAP_BASE`].
    pub unsafe fn from_fixed_bootstrap() -> Result<Self, ProgramClientError> {
        let bootstrap = unsafe { ptr::read_volatile(PROGRAM_BOOTSTRAP_BASE as *const _) };
        Self::from_bootstrap(bootstrap)
    }

    pub const fn surface(&self) -> SurfaceHandle {
        self.surface
    }

    pub const fn has_surface(&self) -> bool {
        self.surface.is_valid()
    }

    pub fn request_surface(&mut self, app: AtriumApp) -> Result<(), ProgramClientError> {
        if self.pending_request.is_some() || self.surface.is_valid() {
            return Err(ProgramClientError::Busy);
        }
        let request = AtriumSurfaceRequest::new(app, self.client, self.next_request_id);
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        self.pending_request = Some(request);
        self.request_sent = false;
        self.flush_surface_request()
    }

    pub fn retry_surface_request(&mut self) -> Result<(), ProgramClientError> {
        self.flush_surface_request()
    }

    pub fn poll_surface(&mut self) -> Result<Option<SurfaceEvent>, ProgramClientError> {
        let mut response = AtriumSurfaceResponse {
            operation: logos_abi::AtriumSurfaceOperation::Request,
            status: GuiStatus::Malformed,
            reserved: 0,
            request_id: 0,
            surface: SurfaceHandle::EMPTY,
        };
        match receive(self.surface_response, &mut response)? {
            Receive::Empty => return Ok(None),
            Receive::Message => {}
        }
        self.accept_surface_response(response).map(Some)
    }

    fn accept_surface_response(
        &mut self,
        response: AtriumSurfaceResponse,
    ) -> Result<SurfaceEvent, ProgramClientError> {
        if response.is_revoke() {
            if response.surface != self.surface {
                return Err(ProgramClientError::Protocol);
            }
            self.surface = SurfaceHandle::EMPTY;
            self.request_sent = false;
            self.draw_frame = 0;
            return Ok(SurfaceEvent::Revoked(response.surface));
        }
        let Some(request) = self.pending_request else {
            return Err(ProgramClientError::Protocol);
        };
        if !response.is_valid_for(request) || GuiStatus::from_raw(response.status as u8).is_none() {
            return Err(ProgramClientError::Protocol);
        }
        self.pending_request = None;
        self.request_sent = false;
        if response.status != GuiStatus::Ok {
            return Err(ProgramClientError::Surface(response.status));
        }
        if !response.surface.is_valid() {
            return Err(ProgramClientError::Protocol);
        }
        self.surface = response.surface;
        self.draw_frame = 0;
        Ok(SurfaceEvent::Created(response.surface))
    }

    pub fn receive_input(&self, input: &mut AtriumSurfaceInput) -> Result<(), ProgramClientError> {
        if !self.surface.is_valid() {
            return Err(ProgramClientError::NoSurface);
        }
        let mut received = AtriumSurfaceInput::new(
            self.surface,
            logos_abi::InputMessage::key(
                logos_abi::KeyCode::ESCAPE,
                logos_abi::KeyState::Pressed,
                0,
            ),
        );
        match receive(self.surface_input, &mut received)? {
            Receive::Empty => return Err(ProgramClientError::Ipc(IpcStatus::Empty)),
            Receive::Message => {}
        }
        if !received.is_valid() || received.surface != self.surface {
            return Err(ProgramClientError::Protocol);
        }
        *input = received;
        Ok(())
    }

    pub fn send_draw(&mut self, batch: GuiDrawBatch) -> Result<(), ProgramClientError> {
        self.require_surface(batch.surface)?;
        if !batch.is_valid() || batch.flags & !GUI_DRAW_FLAG_MORE != 0 {
            return Err(ProgramClientError::InvalidPayload);
        }
        if batch.sequence != self.draw_frame {
            let mut clear = GuiSceneOp::clear(batch.surface, batch.sequence);
            clear.flags = if batch.command_count == 0 { batch.flags } else { GUI_DRAW_FLAG_MORE };
            send(self.surface_draw, &clear)?;
            self.draw_frame = batch.sequence;
        }
        for index in 0..usize::from(batch.command_count) {
            let mut op = GuiSceneOp::upsert(
                batch.surface,
                batch.sequence,
                1 + index as u32,
                batch.commands[index],
            );
            if batch.flags & GUI_DRAW_FLAG_MORE != 0 || index + 1 < usize::from(batch.command_count)
            {
                op.flags = GUI_DRAW_FLAG_MORE;
            }
            send(self.surface_draw, &op)?;
        }
        if batch.command_count == 0 && batch.flags & GUI_DRAW_FLAG_MORE == 0 {
            send(self.surface_draw, &GuiSceneOp::commit(batch.surface, batch.sequence))?;
        }
        Ok(())
    }

    pub fn send_render(&self, message: RenderMessage) -> Result<(), ProgramClientError> {
        self.require_surface(message.surface)?;
        if !matches!(message.kind, MessageKind::RenderCells | MessageKind::FullRedraw)
            || message.flags & !RENDER_FLAG_MORE != 0
            || message.count as usize > MAX_RENDER_CELLS
        {
            return Err(ProgramClientError::InvalidPayload);
        }
        send(self.surface_render, &message)
    }

    fn require_surface(&self, surface: SurfaceHandle) -> Result<(), ProgramClientError> {
        if !self.surface.is_valid() {
            return Err(ProgramClientError::NoSurface);
        }
        if surface != self.surface {
            return Err(ProgramClientError::SurfaceMismatch);
        }
        Ok(())
    }

    fn flush_surface_request(&mut self) -> Result<(), ProgramClientError> {
        let Some(request) = self.pending_request else {
            return Err(ProgramClientError::NoPendingRequest);
        };
        if self.request_sent {
            return Ok(());
        }
        match send(self.surface_request, &request) {
            Ok(()) => {
                self.request_sent = true;
                Ok(())
            }
            Err(ProgramClientError::Ipc(IpcStatus::Full)) => {
                Err(ProgramClientError::Ipc(IpcStatus::Full))
            }
            Err(error) => {
                self.pending_request = None;
                self.request_sent = false;
                Err(error)
            }
        }
    }
}

enum Receive {
    Empty,
    Message,
}

fn send<T: Copy>(
    capability: logos_abi::CapabilityHandle,
    message: &T,
) -> Result<(), ProgramClientError> {
    if !capability.is_valid() || mem::size_of::<T>() == 0 || mem::size_of::<T>() > IPC_PAGE_BYTES {
        return Err(ProgramClientError::InvalidPayload);
    }
    unsafe { ptr::write_unaligned(IPC_STAGING_BASE as *mut T, *message) };
    let status = ipc_syscall(IPC_SYSCALL_SEND, capability.raw(), mem::size_of::<T>());
    if status == IpcStatus::Ok { Ok(()) } else { Err(ProgramClientError::Ipc(status)) }
}

fn receive<T: Copy>(
    capability: logos_abi::CapabilityHandle,
    message: &mut T,
) -> Result<Receive, ProgramClientError> {
    if !capability.is_valid() || mem::size_of::<T>() == 0 || mem::size_of::<T>() > IPC_PAGE_BYTES {
        return Err(ProgramClientError::InvalidPayload);
    }
    let status = ipc_syscall(IPC_SYSCALL_RECEIVE, capability.raw(), 0);
    match status {
        IpcStatus::Ok => {
            *message = unsafe { ptr::read_unaligned(IPC_STAGING_BASE as *const T) };
            Ok(Receive::Message)
        }
        IpcStatus::Empty => Ok(Receive::Empty),
        status => Err(ProgramClientError::Ipc(status)),
    }
}

#[inline(always)]
fn ipc_syscall(number: usize, capability: u64, length: usize) -> IpcStatus {
    #[cfg(target_os = "none")]
    {
        let mut raw = number;
        unsafe {
            core::arch::asm!(
                "int 49",
                inout("rax") raw,
                in("rdi") capability as usize,
                in("rsi") length,
                options(preserves_flags),
            );
        }
        IpcStatus::from_raw(raw).unwrap_or(IpcStatus::Malformed)
    }
    #[cfg(not(target_os = "none"))]
    {
        let _ = (number, capability, length);
        IpcStatus::Unauthorized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bootstrap() -> ProgramBootstrapPage {
        let cap = |index| logos_abi::CapabilityHandle::new(index, 1).unwrap();
        ProgramBootstrapPage {
            abi_version: logos_abi::RUNTIME_ABI_VERSION,
            flags: 0,
            ipc_generation: 1,
            reserved: 0,
            program_generation: 1,
            client: logos_abi::ServiceHandle::new(1, 1).unwrap(),
            surface_request: cap(1),
            surface_response: cap(2),
            surface_input: cap(3),
            surface_render: cap(4),
            surface_draw: cap(5),
        }
    }

    #[test]
    fn bootstrap_is_the_only_constructor_authority() {
        let client = ProgramClient::from_bootstrap(bootstrap()).unwrap();
        assert!(!client.has_surface());
        assert_eq!(client.surface(), SurfaceHandle::EMPTY);
        assert!(matches!(
            ProgramClient::from_bootstrap(ProgramBootstrapPage::empty()),
            Err(ProgramClientError::InvalidBootstrap)
        ));
    }

    #[test]
    fn surface_operations_require_the_admitted_reference() {
        let mut client = ProgramClient::from_bootstrap(bootstrap()).unwrap();
        let batch = GuiDrawBatch::new(SurfaceHandle::EMPTY, 1, logos_abi::GuiRect::SURFACE);
        assert_eq!(client.send_draw(batch), Err(ProgramClientError::NoSurface));
    }

    #[test]
    fn response_binds_surface_to_the_pending_request() {
        let mut client = ProgramClient::from_bootstrap(bootstrap()).unwrap();
        let request = AtriumSurfaceRequest::new(AtriumApp::Calculator, client.client, 1);
        client.pending_request = Some(request);
        let surface = SurfaceHandle::new(2, 1, 13).unwrap();
        let response = AtriumSurfaceResponse {
            operation: logos_abi::AtriumSurfaceOperation::Request,
            status: GuiStatus::Ok,
            reserved: 0,
            request_id: 1,
            surface,
        };
        assert!(response.is_valid_for(request));
        assert_eq!(client.accept_surface_response(response), Ok(SurfaceEvent::Created(surface)));
        assert_eq!(client.surface(), surface);
    }
}
