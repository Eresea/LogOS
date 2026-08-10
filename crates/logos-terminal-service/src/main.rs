#![cfg_attr(not(test), no_main)]
#![cfg_attr(not(test), no_std)]

use logos_abi::{NamespaceId, PageHandle, StoreOperation, StoreRequest, Syscall, VersionSelector};
use logos_service_rt::{Header, MAX_TEXT, ProtocolVersion, ServiceContext, SharedPage};
use logos_terminal::{
    command::{self, Local, Resolution},
    input::{self, LogicalKey},
    terminal::{HISTORY_BYTES, Model},
};

const HISTORY_NAME: &[u8] = b"history";

fn next_id(next: &mut u32) -> u32 {
    let id = (*next).max(1);
    *next = id.wrapping_add(1).max(1);
    id
}

fn request(
    id: u32,
    operation: StoreOperation,
    version: VersionSelector,
    offset: u64,
    length: u32,
    page: PageHandle,
) -> StoreRequest {
    let mut name = [0; logos_abi::MAX_OBJECT_NAME];
    name[..HISTORY_NAME.len()].copy_from_slice(HISTORY_NAME);
    let identifies = matches!(operation, StoreOperation::OpenRead | StoreOperation::BeginReplace);
    StoreRequest {
        id,
        operation,
        namespace: if identifies { logos_abi::TERMINAL_NAMESPACE } else { NamespaceId(0) },
        name: if identifies { name } else { [0; logos_abi::MAX_OBJECT_NAME] },
        name_length: if identifies { HISTORY_NAME.len() as u8 } else { 0 },
        version: if identifies { version } else { VersionSelector::None },
        offset,
        length,
        page,
        deadline: 0,
    }
}

fn page_bytes(page: SharedPage) -> &'static mut [u8; logos_abi::PAGE_SIZE] {
    unsafe { &mut *(page.address as *mut [u8; logos_abi::PAGE_SIZE]) }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HistoryPhase {
    Idle,
    LoadOpen,
    LoadRead,
    SaveBegin,
    SaveWrite,
    SaveCommit,
    SaveAbort,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HistoryPoll {
    Idle,
    Pending,
    Complete,
    Failed,
}

struct HistoryTask {
    phase: HistoryPhase,
    request_id: u32,
    page: Option<SharedPage>,
}

impl HistoryTask {
    const fn new() -> Self {
        Self { phase: HistoryPhase::Idle, request_id: 0, page: None }
    }

    const fn pending(&self) -> bool {
        !matches!(self.phase, HistoryPhase::Idle)
    }

    fn issue(
        &mut self,
        context: &mut ServiceContext,
        request: StoreRequest,
        phase: HistoryPhase,
    ) -> bool {
        if !context.request_store(request) {
            return false;
        }
        self.request_id = request.id;
        self.phase = phase;
        true
    }

    fn start_load(&mut self, context: &mut ServiceContext, next: &mut u32) -> bool {
        if self.pending() {
            return false;
        }
        let Some(page) = context.shared_page() else { return false };
        self.page = Some(page);
        let open = request(
            next_id(next),
            StoreOperation::OpenRead,
            VersionSelector::Current,
            0,
            0,
            PageHandle(0),
        );
        self.issue(context, open, HistoryPhase::LoadOpen)
    }

    fn start_save(
        &mut self,
        terminal: &Model,
        context: &mut ServiceContext,
        next: &mut u32,
    ) -> bool {
        if self.pending() {
            return false;
        }
        let Some(page) = context.shared_page() else { return false };
        page_bytes(page)[..HISTORY_BYTES].copy_from_slice(&terminal.export_history());
        self.page = Some(page);
        let begin = request(
            next_id(next),
            StoreOperation::BeginReplace,
            VersionSelector::None,
            0,
            HISTORY_BYTES as u32,
            PageHandle(0),
        );
        self.issue(context, begin, HistoryPhase::SaveBegin)
    }

    fn poll(
        &mut self,
        terminal: &mut Model,
        context: &mut ServiceContext,
        next: &mut u32,
    ) -> HistoryPoll {
        let phase = self.phase;
        if phase == HistoryPhase::Idle {
            return HistoryPoll::Idle;
        }
        let Some(reply) = context.store_response(self.request_id) else {
            return HistoryPoll::Pending;
        };
        let Some(page) = self.page else {
            self.phase = HistoryPhase::Idle;
            return HistoryPoll::Failed;
        };
        match phase {
            HistoryPhase::LoadOpen => match reply.status {
                logos_abi::PersistenceStatus::NotFound => {
                    self.phase = HistoryPhase::Idle;
                    HistoryPoll::Complete
                }
                logos_abi::PersistenceStatus::Complete
                    if reply.length as usize == HISTORY_BYTES =>
                {
                    let read = request(
                        next_id(next),
                        StoreOperation::ReadChunk,
                        VersionSelector::None,
                        0,
                        HISTORY_BYTES as u32,
                        page.handle,
                    );
                    if self.issue(context, read, HistoryPhase::LoadRead) {
                        HistoryPoll::Pending
                    } else {
                        self.phase = HistoryPhase::Idle;
                        HistoryPoll::Failed
                    }
                }
                logos_abi::PersistenceStatus::Complete | logos_abi::PersistenceStatus::Corrupt => {
                    self.phase = HistoryPhase::Idle;
                    let _ = terminal.write_output(b"history corrupt");
                    HistoryPoll::Complete
                }
                _ => {
                    self.phase = HistoryPhase::Idle;
                    let _ = terminal.write_output(b"history persistence failed");
                    HistoryPoll::Failed
                }
            },
            HistoryPhase::LoadRead => {
                self.phase = HistoryPhase::Idle;
                if reply.status != logos_abi::PersistenceStatus::Complete
                    || reply.length as usize != HISTORY_BYTES
                {
                    let _ = terminal.write_output(b"history corrupt");
                    return HistoryPoll::Complete;
                }
                if !terminal.restore_history_bytes(&page_bytes(page)[..HISTORY_BYTES]) {
                    let _ = terminal.write_output(b"history corrupt");
                }
                HistoryPoll::Complete
            }
            HistoryPhase::SaveBegin => {
                if reply.status != logos_abi::PersistenceStatus::Complete {
                    self.phase = HistoryPhase::Idle;
                    return HistoryPoll::Failed;
                }
                let write = request(
                    next_id(next),
                    StoreOperation::WriteChunk,
                    VersionSelector::None,
                    0,
                    HISTORY_BYTES as u32,
                    page.handle,
                );
                if self.issue(context, write, HistoryPhase::SaveWrite) {
                    HistoryPoll::Pending
                } else {
                    self.phase = HistoryPhase::Idle;
                    HistoryPoll::Failed
                }
            }
            HistoryPhase::SaveWrite => {
                if reply.status != logos_abi::PersistenceStatus::Complete {
                    let abort = request(
                        next_id(next),
                        StoreOperation::Abort,
                        VersionSelector::None,
                        0,
                        0,
                        PageHandle(0),
                    );
                    if self.issue(context, abort, HistoryPhase::SaveAbort) {
                        return HistoryPoll::Pending;
                    }
                    self.phase = HistoryPhase::Idle;
                    return HistoryPoll::Failed;
                }
                let commit = request(
                    next_id(next),
                    StoreOperation::Commit,
                    VersionSelector::None,
                    0,
                    0,
                    PageHandle(0),
                );
                if self.issue(context, commit, HistoryPhase::SaveCommit) {
                    HistoryPoll::Pending
                } else {
                    self.phase = HistoryPhase::Idle;
                    HistoryPoll::Failed
                }
            }
            HistoryPhase::SaveCommit => {
                self.phase = HistoryPhase::Idle;
                if reply.status == logos_abi::PersistenceStatus::Complete {
                    HistoryPoll::Complete
                } else {
                    HistoryPoll::Failed
                }
            }
            HistoryPhase::SaveAbort => {
                self.phase = HistoryPhase::Idle;
                HistoryPoll::Failed
            }
            HistoryPhase::Idle => HistoryPoll::Idle,
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn load_history_with(
    terminal: &mut Model,
    page: SharedPage,
    next: &mut u32,
    mut store: impl FnMut(StoreRequest) -> Option<logos_abi::StoreReply>,
) {
    let open = request(
        next_id(next),
        StoreOperation::OpenRead,
        VersionSelector::Current,
        0,
        0,
        PageHandle(0),
    );
    let Some(reply) = store(open) else {
        let _ = terminal.write_output(b"history persistence failed");
        return;
    };
    match reply.status {
        logos_abi::PersistenceStatus::NotFound => {}
        logos_abi::PersistenceStatus::Complete if reply.length as usize == HISTORY_BYTES => {
            let read = request(
                next_id(next),
                StoreOperation::ReadChunk,
                VersionSelector::None,
                0,
                HISTORY_BYTES as u32,
                page.handle,
            );
            let Some(read_reply) = store(read) else {
                let _ = terminal.write_output(b"history persistence failed");
                return;
            };
            if read_reply.status != logos_abi::PersistenceStatus::Complete
                || read_reply.length as usize != HISTORY_BYTES
            {
                let _ = terminal.write_output(b"history corrupt");
                return;
            }
            if !terminal.restore_history_bytes(&page_bytes(page)[..HISTORY_BYTES]) {
                let _ = terminal.write_output(b"history corrupt");
            }
        }
        logos_abi::PersistenceStatus::Complete | logos_abi::PersistenceStatus::Corrupt => {
            let _ = terminal.write_output(b"history corrupt");
        }
        _ => {
            let _ = terminal.write_output(b"history persistence failed");
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn abort_replace(
    next: &mut u32,
    store: &mut impl FnMut(StoreRequest) -> Option<logos_abi::StoreReply>,
) {
    let _ = store(request(
        next_id(next),
        StoreOperation::Abort,
        VersionSelector::None,
        0,
        0,
        PageHandle(0),
    ));
}

#[cfg_attr(not(test), allow(dead_code))]
fn save_history_with(
    terminal: &Model,
    page: SharedPage,
    next: &mut u32,
    mut store: impl FnMut(StoreRequest) -> Option<logos_abi::StoreReply>,
) -> bool {
    let bytes = terminal.export_history();
    page_bytes(page)[..HISTORY_BYTES].copy_from_slice(&bytes);
    let begin = request(
        next_id(next),
        StoreOperation::BeginReplace,
        VersionSelector::None,
        0,
        HISTORY_BYTES as u32,
        PageHandle(0),
    );
    if !store(begin).is_some_and(|reply| reply.status == logos_abi::PersistenceStatus::Complete) {
        return false;
    }
    let write = request(
        next_id(next),
        StoreOperation::WriteChunk,
        VersionSelector::None,
        0,
        HISTORY_BYTES as u32,
        page.handle,
    );
    if !store(write).is_some_and(|reply| reply.status == logos_abi::PersistenceStatus::Complete) {
        abort_replace(next, &mut store);
        return false;
    }
    let commit =
        request(next_id(next), StoreOperation::Commit, VersionSelector::None, 0, 0, PageHandle(0));
    if !store(commit).is_some_and(|reply| reply.status == logos_abi::PersistenceStatus::Complete) {
        abort_replace(next, &mut store);
        return false;
    }
    true
}

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header =
    Header::new(*b"terminal\0\0\0\0\0\0\0\0", ProtocolVersion::V2, logos_service_entry);

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: logos_service_rt::EntryControlPage) -> ! {
    logos_service_rt::entry(context, run)
}

fn run(context: &mut ServiceContext) -> ! {
    if !context.ready() {
        spin();
    }
    let mut terminal = Model::new();
    let _ = terminal.write_output(b"LOGOS RING3 TERMINAL");
    let mut next_store_id = 1;
    let mut history_started = false;
    let mut history = HistoryTask::new();
    let mut deferred_input = None;
    while context.acknowledged() {
        render(&mut terminal, context);
        match history.poll(&mut terminal, context, &mut next_store_id) {
            HistoryPoll::Failed => {
                let _ = terminal.write_output(b"history persistence failed");
            }
            HistoryPoll::Idle | HistoryPoll::Pending | HistoryPoll::Complete => {}
        }
        let byte = if let Some(byte) = deferred_input.take() {
            Some(byte)
        } else if let Some(byte) = context.input_byte() {
            // A store completion can wake us while the input endpoint already
            // contains a reply. Consume that reply before arming another wait.
            Some(byte)
        } else {
            if !context.wait_for_input() {
                spin();
            }
            context.input_byte()
        };
        let Some(byte) = byte else {
            continue;
        };
        #[cfg(feature = "test-hooks")]
        inject_failure(u32::from(byte));
        if byte == 0x1b {
            let _ = context.complete();
            spin();
        }
        if !history_started {
            history_started = true;
            if byte == logos_abi::InputEvent::STARTUP.byte() {
                if !history.start_load(context, &mut next_store_id) {
                    let _ = terminal.write_output(b"history persistence failed");
                }
                continue;
            }
            if !history.start_load(context, &mut next_store_id) {
                let _ = terminal.write_output(b"history persistence failed");
            }
            deferred_input = Some(byte);
            continue;
        }
        if history.pending() {
            deferred_input = Some(byte);
            continue;
        }
        let Some(input) = logos_abi::InputEvent::from_byte(byte) else {
            continue;
        };
        match input.byte() {
            b'\n' => submit_line(&mut terminal, context, &mut next_store_id, &mut history),
            0x08 => {
                let _ = terminal.backspace();
            }
            0x1b => {}
            byte if byte == logos_abi::InputEvent::UP.byte() => {
                let _ = terminal.apply(input::Event::Key {
                    physical: input::PhysicalKey(0),
                    logical: LogicalKey::Up,
                    state: input::State::Press,
                    modifiers: input::Modifiers::none(),
                });
            }
            byte if byte == logos_abi::InputEvent::DOWN.byte() => {
                let _ = terminal.apply(input::Event::Key {
                    physical: input::PhysicalKey(0),
                    logical: LogicalKey::Down,
                    state: input::State::Press,
                    modifiers: input::Modifiers::none(),
                });
            }
            byte => {
                let _ = terminal.insert_utf8(&[byte]);
            }
        }
    }
    spin()
}

#[cfg(feature = "test-hooks")]
fn inject_failure(control: u32) {
    if control == 0xfa {
        panic!("test panic");
    }
    if control == 0xfb {
        let address = core::hint::black_box(1usize);
        unsafe { (address as *mut u8).write_volatile(1) };
    }
}

fn submit_line(
    terminal: &mut Model,
    context: &mut ServiceContext,
    next: &mut u32,
    history: &mut HistoryTask,
) {
    let submission = terminal.submit();
    let _ = terminal.write_output(submission.as_bytes());
    if !submission.as_bytes().is_empty() && !history.start_save(terminal, context, next) {
        let _ = terminal.write_output(b"history persistence failed");
    }
    match command::pipeline(submission) {
        Resolution::Local(Local::Text(value)) => {
            let _ = terminal.write_output(value.as_bytes());
        }
        Resolution::Local(Local::Clear) => terminal.clear_output(),
        Resolution::Local(Local::CommandList) => {
            for line in command::COMMAND_LIST {
                let _ = terminal.write_output(line);
            }
        }
        Resolution::Local(Local::Layout(layout)) => submit_call(
            terminal,
            context,
            Syscall::SetInputLayout,
            &[match layout {
                logos_terminal::input::Layout::Qwerty => logos_abi::InputLayout::Qwerty.wire(),
                logos_terminal::input::Layout::Azerty => logos_abi::InputLayout::Azerty.wire(),
            }],
        ),
        Resolution::Call(call) => match Syscall::from_name(call.name) {
            Some(command) => {
                if let Some(argument) = call.argument {
                    submit_call(terminal, context, command, argument.as_bytes());
                } else {
                    submit_call(terminal, context, command, &[]);
                }
            }
            None => {
                let _ = terminal.write_output(b"unknown command");
            }
        },
        Resolution::Error(_) => {
            let _ = terminal.write_output(b"unknown command");
        }
    }
}

fn submit_call(
    terminal: &mut Model,
    context: &mut ServiceContext,
    syscall: Syscall,
    argument: &[u8],
) {
    let Some(reply) = context.syscall(syscall, argument) else {
        let _ = terminal.write_output(b"syscall failed");
        return;
    };
    for line in reply.text[..reply.length.min(MAX_TEXT)].split(|byte| *byte == b'\n') {
        if !line.is_empty() {
            let _ = terminal.write_output(line);
        }
    }
}

fn render(terminal: &mut Model, context: &mut ServiceContext) {
    let _ = context.clear_display();
    let mut row = 0u32;
    while let Some(line) = terminal.output_line(row as usize) {
        present(context, 32, 32 + row * 20, line.as_bytes());
        row += 1;
    }
    if row == 0 {
        row = 1;
    }
    present(context, 32, 32 + row * 20, b">");
    present(context, 40, 32 + row * 20, terminal.input_line());
}

fn present(context: &mut ServiceContext, x: u32, y: u32, bytes: &[u8]) {
    for (chunk, bytes) in bytes.chunks(MAX_TEXT).enumerate() {
        let offset = u32::try_from(chunk * MAX_TEXT * 8).unwrap_or(u32::MAX);
        let _ = context.present_text(
            x.saturating_add(offset),
            y,
            logos_abi::DisplayColor::GREEN,
            bytes,
        );
    }
}

fn spin() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logos_abi::StoreReply;

    fn page(bytes: &mut [u8; logos_abi::PAGE_SIZE]) -> SharedPage {
        SharedPage { handle: PageHandle(7), address: bytes.as_mut_ptr() as u64 }
    }

    fn reply(
        request: StoreRequest,
        status: logos_abi::PersistenceStatus,
        length: usize,
    ) -> StoreReply {
        StoreReply { id: request.id, status, version: 1, length: length as u32 }
    }

    fn key(logical: LogicalKey) -> input::Event {
        input::Event::Key {
            physical: input::PhysicalKey(0),
            logical,
            state: input::State::Press,
            modifiers: input::Modifiers::none(),
        }
    }

    #[test]
    fn missing_history_starts_empty() {
        let mut bytes = [0; logos_abi::PAGE_SIZE];
        let mut model = Model::new();
        let mut next = 1;
        load_history_with(&mut model, page(&mut bytes), &mut next, |request| {
            Some(reply(request, logos_abi::PersistenceStatus::NotFound, 0))
        });
        assert_eq!(model.scrollback_len(), 0);
    }

    #[test]
    fn valid_history_restores_navigation() {
        let mut source = Model::new();
        source.insert_utf8(b"azerty");
        source.submit();
        source.insert_utf8(b"qwerty");
        source.submit();
        let encoded = source.export_history();
        let mut bytes = [0; logos_abi::PAGE_SIZE];
        let page = page(&mut bytes);
        let mut model = Model::new();
        let mut next = 1;
        load_history_with(&mut model, page, &mut next, |request| {
            if request.operation == StoreOperation::ReadChunk {
                bytes[..HISTORY_BYTES].copy_from_slice(&encoded);
                Some(reply(request, logos_abi::PersistenceStatus::Complete, HISTORY_BYTES))
            } else {
                Some(reply(request, logos_abi::PersistenceStatus::Complete, HISTORY_BYTES))
            }
        });
        assert!(model.apply(key(LogicalKey::Up)));
        assert_eq!(model.input_line(), b"qwerty");
        assert!(model.apply(key(LogicalKey::Up)));
        assert_eq!(model.input_line(), b"azerty");
        assert!(model.apply(key(LogicalKey::Down)));
        assert_eq!(model.input_line(), b"qwerty");
    }

    #[test]
    fn invalid_history_preserves_live_history() {
        let mut bytes = [0; logos_abi::PAGE_SIZE];
        bytes[0] = 1;
        bytes[1] = 0xff;
        let mut model = Model::new();
        model.insert_utf8(b"live");
        model.submit();
        let mut next = 1;
        load_history_with(&mut model, page(&mut bytes), &mut next, |request| {
            Some(reply(request, logos_abi::PersistenceStatus::Complete, HISTORY_BYTES))
        });
        assert_eq!(model.history_entry(0).unwrap().as_bytes(), b"live");
    }

    #[test]
    fn failed_save_keeps_submitted_command_and_aborts() {
        let mut bytes = [0; logos_abi::PAGE_SIZE];
        let mut model = Model::new();
        model.insert_utf8(b"keep me");
        model.submit();
        let mut operations = [StoreOperation::Cancel; 4];
        let mut count = 0;
        let mut next = 1;
        let saved = save_history_with(&model, page(&mut bytes), &mut next, |request| {
            operations[count] = request.operation;
            count += 1;
            let status = if request.operation == StoreOperation::Commit {
                logos_abi::PersistenceStatus::Io
            } else {
                logos_abi::PersistenceStatus::Complete
            };
            Some(reply(request, status, 0))
        });
        assert!(!saved);
        assert_eq!(
            &operations[..count],
            &[
                StoreOperation::BeginReplace,
                StoreOperation::WriteChunk,
                StoreOperation::Commit,
                StoreOperation::Abort
            ]
        );
        assert!(model.apply(key(LogicalKey::Up)));
        assert_eq!(model.input_line(), b"keep me");
    }
}
