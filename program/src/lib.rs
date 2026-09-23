#![no_std]

#[cfg(test)]
extern crate std;

use core::{mem, ptr};

use logos_abi::{
    AtriumApp, AtriumSurfaceInput, AtriumSurfaceRequest, AtriumSurfaceResponse, GUI_DRAW_FLAG_MORE,
    GuiDrawBatch, GuiRect, GuiSceneOp, GuiStatus, IPC_PAGE_BYTES, IPC_STAGING_BASE,
    IPC_SYSCALL_RECEIVE, IPC_SYSCALL_SEND, IpcStatus, MAX_RENDER_CELLS, MessageKind,
    PROGRAM_BOOTSTRAP_BASE, ProgramBootstrapPage, RENDER_FLAG_MORE, RenderMessage, SurfaceHandle,
};
use logos_ui_graphics::{UiComponentTree, UiScenePublisher, UiSceneSink, UiSceneTheme};

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
            bounds: GuiRect::EMPTY,
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

    pub fn send_scene(
        &mut self,
        publisher: &mut UiScenePublisher,
        frame: u32,
        tree: &UiComponentTree,
    ) -> Result<(), ProgramClientError> {
        let mut sink = ProgramSceneSink(self.surface_draw);
        self.send_scene_with_sink(publisher, frame, tree, &mut sink)
    }

    fn send_scene_with_sink<S: UiSceneSink>(
        &mut self,
        publisher: &mut UiScenePublisher,
        frame: u32,
        tree: &UiComponentTree,
        sink: &mut S,
    ) -> Result<(), ProgramClientError> {
        self.require_surface(self.surface)?;
        match publisher.publish(self.surface, frame, tree, UiSceneTheme::DEFAULT, None, sink) {
            Ok((IpcStatus::Ok, _)) => {
                self.draw_frame = frame;
                Ok(())
            }
            Ok((status, _)) => Err(ProgramClientError::Ipc(status)),
            Err(_) => Err(ProgramClientError::InvalidPayload),
        }
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

struct ProgramSceneSink(logos_abi::CapabilityHandle);

impl UiSceneSink for ProgramSceneSink {
    fn send(&mut self, operation: &GuiSceneOp) -> IpcStatus {
        match send(self.0, operation) {
            Ok(()) => IpcStatus::Ok,
            Err(ProgramClientError::Ipc(status)) => status,
            Err(_) => IpcStatus::Malformed,
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
    use logos_ui_graphics::{UiBlueprint, UiNodeKind, UiRect, UiText};
    use std::vec::Vec;

    struct TestSink {
        operations: Vec<GuiSceneOp>,
        full_at: Option<usize>,
        calls: usize,
    }

    impl UiSceneSink for TestSink {
        fn send(&mut self, operation: &GuiSceneOp) -> IpcStatus {
            let call = self.calls;
            self.calls += 1;
            if self.full_at == Some(call) {
                self.full_at = None;
                return IpcStatus::Full;
            }
            self.operations.push(*operation);
            IpcStatus::Ok
        }
    }

    fn tree() -> UiComponentTree {
        let mut blueprint = UiBlueprint::new();
        let root = blueprint.push_root(UiNodeKind::Root, 1).unwrap();
        let label = blueprint.push_child(UiNodeKind::Label, root, 2).unwrap();
        blueprint.set_text(label, UiText::from_bytes(b"First").unwrap()).unwrap();
        let mut tree = UiComponentTree::from_blueprint(&blueprint).unwrap();
        let root = tree.tree().handle_at(usize::from(root)).unwrap();
        let label = tree.tree().handle_at(usize::from(label)).unwrap();
        tree.tree_mut().set_bounds(root, UiRect::new(0, 0, 100, 30)).unwrap();
        tree.tree_mut().set_bounds(label, UiRect::new(4, 4, 80, 20)).unwrap();
        tree
    }

    fn client() -> ProgramClient {
        let mut client = ProgramClient::from_bootstrap(bootstrap()).unwrap();
        client.surface = SurfaceHandle::new(2, 1, 13).unwrap();
        client
    }

    fn sink(full_at: Option<usize>) -> TestSink {
        TestSink { operations: Vec::new(), full_at, calls: 0 }
    }

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
            bounds: logos_abi::GuiRect::EMPTY,
        };
        assert!(response.is_valid_for(request));
        assert_eq!(client.accept_surface_response(response), Ok(SurfaceEvent::Created(surface)));
        assert_eq!(client.surface(), surface);
    }

    #[test]
    fn retained_scene_validation_rejects_cursor_shape_for_program_surfaces() {
        let mut client = ProgramClient::from_bootstrap(bootstrap()).unwrap();
        let surface = SurfaceHandle::new(2, 1, 13).unwrap();
        client.surface = surface;
        let mut blueprint = UiBlueprint::new();
        blueprint.push_root(UiNodeKind::Root, 1).unwrap();
        let mut tree = UiComponentTree::from_blueprint(&blueprint).unwrap();
        let root = tree.tree().handle_at(0).unwrap();
        tree.tree_mut().set_bounds(root, UiRect::new(0, 0, 3, 14)).unwrap();
        let mut publisher = UiScenePublisher::new();
        let mut sink = sink(None);

        assert_eq!(
            client.send_scene_with_sink(&mut publisher, 1, &tree, &mut sink),
            Err(ProgramClientError::InvalidPayload)
        );
        assert!(sink.operations.is_empty());
    }

    #[test]
    fn send_scene_resumes_a_full_frame_without_resending_sent_operations() {
        let mut client = client();
        let mut publisher = UiScenePublisher::new();
        let tree = tree();
        let expected =
            logos_ui_graphics::emit(client.surface(), 1, &tree, UiSceneTheme::DEFAULT).unwrap();
        let mut sink = sink(Some(2));

        assert_eq!(
            client.send_scene_with_sink(&mut publisher, 1, &tree, &mut sink),
            Err(ProgramClientError::Ipc(IpcStatus::Full))
        );
        assert!(publisher.is_pending());
        assert_eq!(client.send_scene_with_sink(&mut publisher, 1, &tree, &mut sink), Ok(()));
        assert!(!publisher.is_pending());
        assert_eq!(sink.operations, expected.as_slice());
    }

    #[test]
    fn send_scene_publishes_deltas_and_coalesces_after_full() {
        let mut client = client();
        let mut publisher = UiScenePublisher::new();
        let mut tree = tree();
        let mut sink = sink(None);
        client.send_scene_with_sink(&mut publisher, 1, &tree, &mut sink).unwrap();
        sink.operations.clear();
        sink.calls = 0;

        let label = tree.tree().handle_at(1).unwrap();
        tree.set_text(label, UiText::from_bytes(b"Intermediate").unwrap()).unwrap();
        sink.full_at = Some(1);
        assert_eq!(
            client.send_scene_with_sink(&mut publisher, 2, &tree, &mut sink),
            Err(ProgramClientError::Ipc(IpcStatus::Full))
        );
        tree.set_text(label, UiText::from_bytes(b"Latest").unwrap()).unwrap();
        sink.calls = 0;
        client.send_scene_with_sink(&mut publisher, 3, &tree, &mut sink).unwrap();

        let frame_two_commit = sink
            .operations
            .iter()
            .position(|op| op.operation == logos_abi::GuiNodeOperation::Commit && op.frame == 2)
            .unwrap();
        let frame_three = sink.operations.iter().position(|op| op.frame == 3).unwrap();
        assert!(frame_two_commit < frame_three);
        assert!(
            !sink
                .operations
                .iter()
                .any(|op| op.frame == 3 && op.operation == logos_abi::GuiNodeOperation::Clear)
        );
        assert!(sink.operations.iter().any(|op| {
            op.frame == 3 && &op.command.text[..usize::from(op.command.text_len)] == b"Latest"
        }));
    }

    #[test]
    fn send_scene_rebinds_with_a_full_frame() {
        let mut client = client();
        let mut publisher = UiScenePublisher::new();
        let tree = tree();
        let mut sink = sink(None);
        client.send_scene_with_sink(&mut publisher, 1, &tree, &mut sink).unwrap();
        let first_surface = client.surface();
        client.surface =
            SurfaceHandle::new(first_surface.slot, first_surface.generation + 1, 13).unwrap();
        sink.operations.clear();
        sink.calls = 0;

        client.send_scene_with_sink(&mut publisher, 2, &tree, &mut sink).unwrap();

        assert_eq!(sink.operations[0].operation, logos_abi::GuiNodeOperation::Clear);
        assert!(sink.operations.iter().all(|op| op.surface == client.surface()));
    }
}
