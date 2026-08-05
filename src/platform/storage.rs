use crate::debug;
use crate::{
    arch::interrupts,
    platform::{block, secrets, session},
    sched::native_task,
};
use logos_core::capabilities;

pub const NAME: &[u8] = b"storage";
pub const SERVICE: crate::platform::services::Service = crate::platform::services::Service::Storage;

pub const TERMINAL_NAMESPACE: logos_abi::NamespaceId = logos_abi::TERMINAL_NAMESPACE;
pub const TEXT_NAMESPACE: logos_abi::NamespaceId = logos_abi::NamespaceId(2);
pub const AUDIT_NAMESPACE: logos_abi::NamespaceId = logos_abi::NamespaceId(3);
pub const SECRETS_NAMESPACE: logos_abi::NamespaceId = logos_abi::NamespaceId(4);

pub struct StorageRuntime {
    store_client: crate::sched::native_task::StoreClientEndpoint,
    store_server: crate::sched::native_task::StoreServerEndpoint,
    block_client: crate::sched::native_task::BlockClientEndpoint,
    handle: crate::sched::native_task::Handle,
    block_dispatch: crate::platform::block::Dispatch,
    relay: RelayState,
}

impl StorageRuntime {
    pub const fn new(
        store_client: crate::sched::native_task::StoreClientEndpoint,
        store_server: crate::sched::native_task::StoreServerEndpoint,
        block_client: crate::sched::native_task::BlockClientEndpoint,
        handle: crate::sched::native_task::Handle,
    ) -> Self {
        Self {
            store_client,
            store_server,
            block_client,
            handle,
            block_dispatch: crate::platform::block::Dispatch::new(),
            relay: RelayState::new(),
        }
    }

    pub fn rebind(
        &mut self,
        store_server: crate::sched::native_task::StoreServerEndpoint,
        block_client: crate::sched::native_task::BlockClientEndpoint,
        handle: crate::sched::native_task::Handle,
    ) {
        self.store_server = store_server;
        self.block_client = block_client;
        self.handle = handle;
        self.relay.clear();
    }

    pub fn rebind_client(&mut self, store_client: crate::sched::native_task::StoreClientEndpoint) {
        self.store_client = store_client;
        self.relay.clear();
    }

    fn bind_block_context(&self, context: &mut block::DispatchContext<'_>) {
        context.endpoint = self.block_client;
    }

    pub fn poll_block(
        &mut self,
        context: &mut block::DispatchContext<'_>,
        scheduler: &mut native_task::Scheduler<'_>,
        tick: u64,
    ) -> bool {
        if !self.block_client.available() || !self.handle.available() {
            return true;
        }
        let Some(reply) = self.block_reply(context, tick) else { return true };
        self.block_client.reply(reply) && scheduler.wake(self.handle) && scheduler.run(self.handle)
    }

    pub fn block_reply(
        &mut self,
        context: &mut block::DispatchContext<'_>,
        tick: u64,
    ) -> Option<logos_abi::BlockReply> {
        self.bind_block_context(context);
        self.block_dispatch.poll(context, tick)
    }

    pub fn cancel_block(&mut self, context: &mut block::DispatchContext<'_>) {
        self.bind_block_context(context);
        self.block_dispatch.cancel_on_exit(context);
    }

    pub fn startup(
        &mut self,
        context: &mut block::DispatchContext<'_>,
        scheduler: &mut native_task::Scheduler<'_>,
    ) -> bool {
        self.bind_block_context(context);
        run_startup(self.store_server, &mut self.block_dispatch, context, scheduler, self.handle)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn protected_store_read(
        &mut self,
        context: &mut block::DispatchContext<'_>,
        scheduler: &mut native_task::Scheduler<'_>,
        page: logos_abi::PageHandle,
        page_address: u64,
        namespace: logos_abi::NamespaceId,
        name: &[u8],
        output: &mut [u8],
        tick: u64,
    ) -> logos_abi::PersistenceStatus {
        self.bind_block_context(context);
        protected_store_read(
            self.store_server,
            &mut self.block_dispatch,
            context,
            scheduler,
            self.handle,
            page,
            page_address,
            namespace,
            name,
            output,
            tick,
        )
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    #[allow(clippy::too_many_arguments)]
    pub fn relay_store_request(
        &mut self,
        terminal: native_task::StoreClientEndpoint,
        context: &mut block::DispatchContext<'_>,
        terminal_owner: u64,
        storage_owner: u64,
        history_page: logos_abi::PageHandle,
        scheduler: &mut native_task::Scheduler<'_>,
        session: &session::Context,
        capabilities: &capabilities::CapabilityManager,
        tick: u64,
    ) -> session::Relay {
        self.bind_block_context(context);
        relay_store_request(
            terminal,
            self.store_server,
            &mut self.block_dispatch,
            context,
            terminal_owner,
            storage_owner,
            history_page,
            scheduler,
            self.handle,
            session,
            capabilities,
            &mut self.relay,
            tick,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn relay_terminal_store_requests(
        &mut self,
        terminal: native_task::StoreClientEndpoint,
        context: &mut block::DispatchContext<'_>,
        terminal_owner: u64,
        storage_owner: u64,
        history_page: logos_abi::PageHandle,
        scheduler: &mut native_task::Scheduler<'_>,
        terminal_handle: native_task::Handle,
        session: &session::Context,
        capabilities: &capabilities::CapabilityManager,
        tick: u64,
    ) -> bool {
        self.bind_block_context(context);
        relay_terminal_store_requests(
            terminal,
            self.store_server,
            &mut self.block_dispatch,
            context,
            terminal_owner,
            storage_owner,
            history_page,
            scheduler,
            terminal_handle,
            self.handle,
            session,
            capabilities,
            &mut self.relay,
            tick,
        )
    }

    pub fn cancel_store_transaction(&self, scheduler: &mut native_task::Scheduler<'_>) -> bool {
        cancel_store_transaction(self.store_server, scheduler, self.handle)
    }

    pub fn persist_remote_control(
        &mut self,
        state: &mut secrets::RemoteState,
        context: &mut block::DispatchContext<'_>,
        scheduler: &mut native_task::Scheduler<'_>,
        page: logos_abi::PageHandle,
        owner: u64,
        tick: u64,
    ) -> bool {
        self.bind_block_context(context);
        persist_remote_control(
            state,
            self.store_server,
            &mut self.block_dispatch,
            context,
            scheduler,
            self.handle,
            page,
            owner,
            tick,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn persist_remote_enrollment(
        &mut self,
        state: &mut secrets::RemoteState,
        bootstrap: Option<logos_remote::Bootstrap>,
        context: &mut block::DispatchContext<'_>,
        scheduler: &mut native_task::Scheduler<'_>,
        page: logos_abi::PageHandle,
        terminal_owner: u64,
        tick: u64,
    ) -> bool {
        self.bind_block_context(context);
        persist_remote_enrollment(
            state,
            bootstrap,
            self.store_server,
            &mut self.block_dispatch,
            context,
            scheduler,
            self.handle,
            page,
            terminal_owner,
            tick,
        )
    }

    pub fn reset_relay(&mut self) {
        self.relay.clear();
    }
}

pub struct RelayState {
    pub read_namespace: Option<logos_abi::NamespaceId>,
    pub replace_namespace: Option<logos_abi::NamespaceId>,
}

impl RelayState {
    pub const fn new() -> Self {
        Self { read_namespace: None, replace_namespace: None }
    }

    pub fn clear(&mut self) {
        self.read_namespace = None;
        self.replace_namespace = None;
    }
}

#[allow(clippy::too_many_arguments)]
pub fn persist_remote_control(
    state: &mut secrets::RemoteState,
    storage: native_task::StoreServerEndpoint,
    dispatch: &mut block::Dispatch,
    context: &mut block::DispatchContext<'_>,
    scheduler: &mut native_task::Scheduler<'_>,
    storage_handle: native_task::Handle,
    page: logos_abi::PageHandle,
    owner: u64,
    tick: u64,
) -> bool {
    let Some(address) = context.pages.address(owner, page) else { return false };
    let mut blob = [0; logos_remote::REMOTE_CONTROL_BLOB_BYTES];
    if !state.seal_control_random(&mut blob) {
        return false;
    }
    let status = protected_store_replace(
        storage,
        dispatch,
        context,
        scheduler,
        storage_handle,
        page,
        address,
        logos_abi::TRUST_NAMESPACE,
        logos_abi::TRUST_SESSION_NAME,
        &blob,
        tick,
    );
    if status == logos_abi::PersistenceStatus::Complete {
        true
    } else {
        state.disable();
        false
    }
}

fn store_namespace(
    request: logos_abi::StoreRequest,
    state: &RelayState,
) -> Option<logos_abi::NamespaceId> {
    match request.operation {
        logos_abi::StoreOperation::OpenRead | logos_abi::StoreOperation::BeginReplace => {
            Some(request.namespace)
        }
        logos_abi::StoreOperation::ReadChunk => state.read_namespace,
        logos_abi::StoreOperation::WriteChunk
        | logos_abi::StoreOperation::Commit
        | logos_abi::StoreOperation::Abort
        | logos_abi::StoreOperation::Cancel => state.replace_namespace,
    }
}

fn store_capability(operation: logos_abi::StoreOperation) -> capabilities::CapabilityKind {
    match operation {
        logos_abi::StoreOperation::OpenRead | logos_abi::StoreOperation::ReadChunk => {
            capabilities::CapabilityKind::StoreRead
        }
        logos_abi::StoreOperation::BeginReplace
        | logos_abi::StoreOperation::WriteChunk
        | logos_abi::StoreOperation::Commit
        | logos_abi::StoreOperation::Abort
        | logos_abi::StoreOperation::Cancel => capabilities::CapabilityKind::StoreWrite,
    }
}

fn update_store_state(
    state: &mut RelayState,
    request: logos_abi::StoreRequest,
    status: logos_abi::PersistenceStatus,
) {
    if status != logos_abi::PersistenceStatus::Complete {
        return;
    }
    match request.operation {
        logos_abi::StoreOperation::OpenRead => state.read_namespace = Some(request.namespace),
        logos_abi::StoreOperation::BeginReplace => {
            state.replace_namespace = Some(request.namespace)
        }
        logos_abi::StoreOperation::Commit
        | logos_abi::StoreOperation::Abort
        | logos_abi::StoreOperation::Cancel => {
            state.replace_namespace = None;
            if request.operation == logos_abi::StoreOperation::Cancel {
                state.read_namespace = None;
            }
        }
        logos_abi::StoreOperation::ReadChunk | logos_abi::StoreOperation::WriteChunk => {}
    }
}

#[allow(clippy::too_many_arguments)]
pub fn relay_store_request(
    terminal: native_task::StoreClientEndpoint,
    storage: native_task::StoreServerEndpoint,
    dispatch: &mut block::Dispatch,
    block_context: &mut block::DispatchContext<'_>,
    terminal_owner: u64,
    storage_owner: u64,
    history_page: logos_abi::PageHandle,
    scheduler: &mut native_task::Scheduler<'_>,
    storage_handle: native_task::Handle,
    session: &session::Context,
    capabilities: &capabilities::CapabilityManager,
    state: &mut RelayState,
    tick: u64,
) -> session::Relay {
    let Some(request) = terminal.request() else {
        return session::Relay::Handled(true);
    };
    if !storage.available() || !storage_handle.available() {
        return session::Relay::Handled(terminal.reply(logos_abi::StoreReply {
            id: request.id,
            status: logos_abi::PersistenceStatus::Unavailable,
            version: 0,
            length: 0,
        }));
    }
    let Some(namespace) = store_namespace(request, state) else {
        let _ = terminal.reply(logos_abi::StoreReply {
            id: request.id,
            status: logos_abi::PersistenceStatus::Denied,
            version: 0,
            length: 0,
        });
        return session::Relay::Handled(true);
    };
    if !session.allows_scoped(capabilities, store_capability(request.operation), namespace.0) {
        let replied = terminal.reply(logos_abi::StoreReply {
            id: request.id,
            status: logos_abi::PersistenceStatus::Denied,
            version: 0,
            length: 0,
        });
        return session::Relay::Handled(replied);
    }
    let needs_page = matches!(
        request.operation,
        logos_abi::StoreOperation::ReadChunk | logos_abi::StoreOperation::WriteChunk
    );
    let mut loaned = false;
    if needs_page {
        let Some(page) = terminal.transfer_page() else {
            let _ = terminal.reply(logos_abi::StoreReply {
                id: request.id,
                status: logos_abi::PersistenceStatus::Denied,
                version: 0,
                length: 0,
            });
            return session::Relay::Handled(true);
        };
        if page != history_page
            || block_context.pages.address(storage_owner, page).is_some()
            || block_context.pages.address(terminal_owner, page).is_none()
            || !block_context.pages.lend(terminal_owner, page, storage_owner)
        {
            let _ = terminal.reply(logos_abi::StoreReply {
                id: request.id,
                status: logos_abi::PersistenceStatus::Denied,
                version: 0,
                length: 0,
            });
            return session::Relay::Handled(true);
        }
        loaned = true;
    }
    if !storage.deliver(request, terminal_owner) {
        if loaned {
            let _ = block_context.pages.return_loan(storage_owner, request.page);
        }
        return session::Relay::Handled(false);
    }
    if !scheduler.wake(storage_handle) || !scheduler.run(storage_handle) {
        if loaned {
            let _ = block_context.pages.return_loan(storage_owner, request.page);
        }
        return session::Relay::Handled(false);
    }
    let mut current_tick = tick;
    loop {
        if let Some(reply) = storage.response(request.id) {
            if loaned {
                let _ = block_context.pages.return_loan(storage_owner, request.page);
            }
            update_store_state(state, request, reply.status);
            return session::Relay::Handled(
                scheduler.wake(storage_handle)
                    && scheduler.run(storage_handle)
                    && terminal.reply(reply),
            );
        }
        if scheduler.run_next() {
            continue;
        }
        if scheduler.failed(storage_handle) {
            if loaned {
                let _ = block_context.pages.return_loan(storage_owner, request.page);
            }
            return session::Relay::Handled(false);
        }
        if let Some(reply) = dispatch.poll(block_context, current_tick) {
            if !block_context.endpoint.reply(reply)
                || !scheduler.wake(storage_handle)
                || !scheduler.run(storage_handle)
            {
                if loaned {
                    let _ = block_context.pages.return_loan(storage_owner, request.page);
                }
                return session::Relay::Handled(false);
            }
        } else if dispatch.accepts_new_request() {
            interrupts::wait_for_tick();
        } else {
            interrupts::wait_for_virtio();
        }
        current_tick = interrupts::ticks();
    }
}

#[allow(clippy::too_many_arguments)]
fn protected_store_request(
    storage: native_task::StoreServerEndpoint,
    dispatch: &mut block::Dispatch,
    block_context: &mut block::DispatchContext<'_>,
    scheduler: &mut native_task::Scheduler<'_>,
    storage_handle: native_task::Handle,
    request: logos_abi::StoreRequest,
    tick: u64,
) -> Option<logos_abi::StoreReply> {
    if !storage.available() || !storage.deliver(request, 0) || !scheduler.wake(storage_handle) {
        return None;
    }
    let mut current_tick = tick.max(1);
    if !scheduler.run(storage_handle) {
        return None;
    }
    loop {
        if let Some(reply) = storage.response(request.id) {
            return Some(reply);
        }
        if scheduler.run_next() {
            continue;
        }
        if scheduler.failed(storage_handle) {
            return None;
        }
        if let Some(reply) = dispatch.poll(block_context, current_tick) {
            if !block_context.endpoint.reply(reply)
                || !scheduler.wake(storage_handle)
                || !scheduler.run(storage_handle)
            {
                return None;
            }
        } else if dispatch.accepts_new_request() {
            interrupts::wait_for_tick();
        } else {
            interrupts::wait_for_virtio();
        }
        current_tick = interrupts::ticks();
    }
}

#[allow(clippy::too_many_arguments)]
pub fn protected_store_replace(
    storage: native_task::StoreServerEndpoint,
    dispatch: &mut block::Dispatch,
    block_context: &mut block::DispatchContext<'_>,
    scheduler: &mut native_task::Scheduler<'_>,
    storage_handle: native_task::Handle,
    page: logos_abi::PageHandle,
    page_address: u64,
    namespace: logos_abi::NamespaceId,
    name: &[u8],
    bytes: &[u8],
    tick: u64,
) -> logos_abi::PersistenceStatus {
    if name.is_empty()
        || name.len() > logos_abi::MAX_OBJECT_NAME
        || bytes.is_empty()
        || bytes.len() > logos_abi::PAGE_SIZE
        || core::str::from_utf8(name).is_err()
    {
        return logos_abi::PersistenceStatus::Invalid;
    }
    let mut identity = [0; logos_abi::MAX_OBJECT_NAME];
    identity[..name.len()].copy_from_slice(name);
    let begin = logos_abi::StoreRequest {
        id: u32::MAX - 3,
        operation: logos_abi::StoreOperation::BeginReplace,
        namespace,
        name: identity,
        name_length: name.len() as u8,
        version: logos_abi::VersionSelector::None,
        offset: 0,
        length: bytes.len() as u32,
        page: logos_abi::PageHandle(0),
        deadline: tick.max(1).saturating_add(100),
    };
    if protected_store_request(
        storage,
        dispatch,
        block_context,
        scheduler,
        storage_handle,
        begin,
        tick,
    )
    .is_none_or(|reply| reply.status != logos_abi::PersistenceStatus::Complete)
    {
        return logos_abi::PersistenceStatus::Unavailable;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), page_address as *mut u8, bytes.len());
    }
    let write = logos_abi::StoreRequest {
        id: u32::MAX - 2,
        operation: logos_abi::StoreOperation::WriteChunk,
        namespace: logos_abi::NamespaceId(0),
        name: [0; logos_abi::MAX_OBJECT_NAME],
        name_length: 0,
        version: logos_abi::VersionSelector::None,
        offset: 0,
        length: bytes.len() as u32,
        page,
        deadline: tick.max(1).saturating_add(100),
    };
    if protected_store_request(
        storage,
        dispatch,
        block_context,
        scheduler,
        storage_handle,
        write,
        tick,
    )
    .is_none_or(|reply| reply.status != logos_abi::PersistenceStatus::Complete)
    {
        let _ = cancel_store_transaction(storage, scheduler, storage_handle);
        return logos_abi::PersistenceStatus::Unavailable;
    }
    let commit = logos_abi::StoreRequest {
        id: u32::MAX - 1,
        operation: logos_abi::StoreOperation::Commit,
        namespace: logos_abi::NamespaceId(0),
        name: [0; logos_abi::MAX_OBJECT_NAME],
        name_length: 0,
        version: logos_abi::VersionSelector::None,
        offset: 0,
        length: 0,
        page: logos_abi::PageHandle(0),
        deadline: tick.max(1).saturating_add(100),
    };
    protected_store_request(
        storage,
        dispatch,
        block_context,
        scheduler,
        storage_handle,
        commit,
        tick,
    )
    .map_or(logos_abi::PersistenceStatus::Unavailable, |reply| reply.status)
}

#[allow(clippy::too_many_arguments)]
pub fn protected_store_read(
    storage: native_task::StoreServerEndpoint,
    dispatch: &mut block::Dispatch,
    block_context: &mut block::DispatchContext<'_>,
    scheduler: &mut native_task::Scheduler<'_>,
    storage_handle: native_task::Handle,
    page: logos_abi::PageHandle,
    page_address: u64,
    namespace: logos_abi::NamespaceId,
    name: &[u8],
    output: &mut [u8],
    tick: u64,
) -> logos_abi::PersistenceStatus {
    if name.is_empty()
        || name.len() > logos_abi::MAX_OBJECT_NAME
        || output.is_empty()
        || output.len() > logos_abi::PAGE_SIZE
        || core::str::from_utf8(name).is_err()
    {
        return logos_abi::PersistenceStatus::Invalid;
    }
    let mut identity = [0; logos_abi::MAX_OBJECT_NAME];
    identity[..name.len()].copy_from_slice(name);
    let open = logos_abi::StoreRequest {
        id: u32::MAX - 6,
        operation: logos_abi::StoreOperation::OpenRead,
        namespace,
        name: identity,
        name_length: name.len() as u8,
        version: logos_abi::VersionSelector::Current,
        offset: 0,
        length: 0,
        page: logos_abi::PageHandle(0),
        deadline: tick.max(1).saturating_add(100),
    };
    let Some(reply) = protected_store_request(
        storage,
        dispatch,
        block_context,
        scheduler,
        storage_handle,
        open,
        tick,
    ) else {
        return logos_abi::PersistenceStatus::Unavailable;
    };
    if reply.status != logos_abi::PersistenceStatus::Complete {
        return reply.status;
    }
    let length = reply.length as usize;
    if length == 0 || length > output.len() {
        return logos_abi::PersistenceStatus::Invalid;
    }
    let read = logos_abi::StoreRequest {
        id: u32::MAX - 5,
        operation: logos_abi::StoreOperation::ReadChunk,
        namespace: logos_abi::NamespaceId(0),
        name: [0; logos_abi::MAX_OBJECT_NAME],
        name_length: 0,
        version: logos_abi::VersionSelector::None,
        offset: 0,
        length: length as u32,
        page,
        deadline: tick.max(1).saturating_add(100),
    };
    let Some(reply) = protected_store_request(
        storage,
        dispatch,
        block_context,
        scheduler,
        storage_handle,
        read,
        tick,
    ) else {
        return logos_abi::PersistenceStatus::Unavailable;
    };
    if reply.status == logos_abi::PersistenceStatus::Complete {
        unsafe {
            core::ptr::copy_nonoverlapping(page_address as *const u8, output.as_mut_ptr(), length);
        }
    }
    reply.status
}

#[allow(clippy::too_many_arguments)]
pub fn persist_remote_enrollment(
    state: &mut secrets::RemoteState,
    bootstrap: Option<logos_remote::Bootstrap>,
    storage: native_task::StoreServerEndpoint,
    dispatch: &mut block::Dispatch,
    block_context: &mut block::DispatchContext<'_>,
    scheduler: &mut native_task::Scheduler<'_>,
    storage_handle: native_task::Handle,
    page: logos_abi::PageHandle,
    terminal_owner: u64,
    tick: u64,
) -> bool {
    let Some(bootstrap) = bootstrap else { return false };
    let Some(page_address) = block_context.pages.address(terminal_owner, page) else {
        return false;
    };
    let mut blob = [0; logos_remote::ENROLLMENT_BLOB_BYTES];
    if !state.seal_enrollment_random(&mut blob) {
        return false;
    }
    let status = protected_store_replace(
        storage,
        dispatch,
        block_context,
        scheduler,
        storage_handle,
        page,
        page_address,
        logos_abi::TRUST_NAMESPACE,
        logos_abi::TRUST_ENROLLMENT_NAME,
        &blob,
        tick,
    );
    if status == logos_abi::PersistenceStatus::Complete {
        true
    } else {
        *state = secrets::RemoteState::unavailable(bootstrap);
        false
    }
}

#[allow(clippy::too_many_arguments)]
pub fn relay_terminal_store_requests(
    terminal: native_task::StoreClientEndpoint,
    storage: native_task::StoreServerEndpoint,
    dispatch: &mut block::Dispatch,
    block_context: &mut block::DispatchContext<'_>,
    terminal_owner: u64,
    storage_owner: u64,
    history_page: logos_abi::PageHandle,
    scheduler: &mut native_task::Scheduler<'_>,
    terminal_handle: native_task::Handle,
    storage_handle: native_task::Handle,
    session: &session::Context,
    capabilities: &capabilities::CapabilityManager,
    state: &mut RelayState,
    tick: u64,
) -> bool {
    while terminal.request().is_some() {
        if !relay_store_request(
            terminal,
            storage,
            dispatch,
            block_context,
            terminal_owner,
            storage_owner,
            history_page,
            scheduler,
            storage_handle,
            session,
            capabilities,
            state,
            tick,
        )
        .ok()
        {
            let Some(request) = terminal.request() else { return false };
            if !terminal.reply(logos_abi::StoreReply {
                id: request.id,
                status: logos_abi::PersistenceStatus::Unavailable,
                version: 0,
                length: 0,
            }) {
                return false;
            }
        }
        if !scheduler.wake(terminal_handle) || !scheduler.run(terminal_handle) {
            return false;
        }
    }
    true
}

pub fn cancel_store_transaction(
    storage: native_task::StoreServerEndpoint,
    scheduler: &mut native_task::Scheduler<'_>,
    storage_handle: native_task::Handle,
) -> bool {
    if !storage.available() || !storage_handle.available() {
        return true;
    }
    let request = logos_abi::StoreRequest {
        id: u32::MAX,
        operation: logos_abi::StoreOperation::Cancel,
        namespace: logos_abi::NamespaceId(0),
        name: [0; logos_abi::MAX_OBJECT_NAME],
        name_length: 0,
        version: logos_abi::VersionSelector::None,
        offset: 0,
        length: 0,
        page: logos_abi::PageHandle(0),
        deadline: 0,
    };
    storage.deliver(request, 0)
        && scheduler.wake(storage_handle)
        && scheduler.run(storage_handle)
        && storage
            .response(request.id)
            .is_some_and(|reply| reply.status == logos_abi::PersistenceStatus::Complete)
}

pub fn run_startup(
    storage: native_task::StoreServerEndpoint,
    dispatch: &mut block::Dispatch,
    context: &mut block::DispatchContext<'_>,
    scheduler: &mut native_task::Scheduler<'_>,
    handle: native_task::Handle,
) -> bool {
    interrupts::enable();
    loop {
        if scheduler.failed(handle) {
            return false;
        }
        let waiting = storage.waiting();
        if waiting {
            debug::write_line(b"LogOS: startup waiting");
            let marker: &[u8] = match storage.status() {
                Some(logos_core::native_service::STORAGE_FORMATTED) => b"LogOS: storage formatted",
                Some(logos_core::native_service::STORAGE_RECOVERED) => b"LogOS: storage recovered",
                Some(logos_core::native_service::STORAGE_RECOVERED_INCOMPLETE) => {
                    b"LogOS: storage recovered-incomplete"
                }
                Some(logos_core::native_service::STORAGE_CORRUPT) => b"LogOS: storage corrupt",
                Some(logos_core::native_service::STORAGE_IO_FAILED) => b"LogOS: storage io-failed",
                _ => {
                    return false;
                }
            };
            debug::write_line(marker);
            return true;
        }
        let Some(reply) = dispatch.poll(context, interrupts::ticks()) else {
            debug::write_line(if dispatch.accepts_new_request() {
                b"LogOS: storage startup no request"
            } else {
                b"LogOS: storage startup block pending"
            });
            if dispatch.accepts_new_request() {
                interrupts::wait_for_tick();
            } else {
                interrupts::wait_for_virtio();
            }
            continue;
        };
        if !context.endpoint.reply(reply) {
            return false;
        }
        if !scheduler.wake(handle) {
            return false;
        }
        if !scheduler.run(handle) {
            return false;
        }
    }
}
