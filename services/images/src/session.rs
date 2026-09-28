#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    CompletionRequest, CompletionResponse, FetchPhase, FetchResponse, FetchStatus, FlowControl,
    IPC_FLAG_MORE, IpcBytes, IpcStatus, MAX_IPC_BYTES, MAX_SHELL_SESSIONS, MessageKind,
};
use logos_session::MAX_LINE_BYTES;

const INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"terminal",
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);
const OUTPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"terminal",
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const FLOW_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"flow",
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const FLOW_OUTPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"flow",
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);

struct PendingOutput {
    bytes: [u8; logos_session::MAX_OUTPUT_BYTES],
    len: usize,
    offset: usize,
}

impl PendingOutput {
    const fn new() -> Self {
        Self { bytes: [0; logos_session::MAX_OUTPUT_BYTES], len: 0, offset: 0 }
    }

    fn is_empty(&self) -> bool {
        self.offset >= self.len
    }

    fn stage(&mut self, bytes: &[u8]) {
        let count = bytes.len().min(self.bytes.len());
        self.bytes[..count].copy_from_slice(&bytes[..count]);
        self.len = count;
        self.offset = 0;
    }

    /// Tags every chunk with `session` (T3c, #98) so Terminal routes it to
    /// the right tab regardless of which tab is currently focused.
    fn flush(&mut self, capability: logos_abi::CapabilityHandle, session: u8) -> bool {
        let mut progressed = false;
        while self.offset < self.len {
            let end = (self.offset + MAX_IPC_BYTES).min(self.len);
            let Some(message) =
                IpcBytes::from_bytes(MessageKind::SessionOutput, &self.bytes[self.offset..end])
            else {
                break;
            };
            let message = message.with_session(session);
            if common::ipc_send_handle(capability, &message) != IpcStatus::Ok {
                break;
            }
            self.offset = end;
            progressed = true;
        }
        if self.is_empty() {
            self.len = 0;
            self.offset = 0;
        }
        progressed
    }
}

struct PendingFlowInput {
    bytes: [u8; MAX_LINE_BYTES],
    len: usize,
    pending: bool,
}

impl PendingFlowInput {
    const fn new() -> Self {
        Self { bytes: [0; MAX_LINE_BYTES], len: 0, pending: false }
    }

    fn is_empty(&self) -> bool {
        !self.pending
    }

    fn stage(&mut self, bytes: &[u8]) {
        self.bytes[..bytes.len()].copy_from_slice(bytes);
        self.len = bytes.len();
        self.pending = true;
    }

    fn clear(&mut self) {
        self.len = 0;
        self.pending = false;
    }

    fn take(&mut self, session: u8) -> Option<IpcBytes> {
        let message = IpcBytes::from_bytes(MessageKind::SessionInput, &self.bytes[..self.len])
            .map(|message| message.with_session(session));
        if message.is_some() {
            self.clear();
        }
        message
    }
}

/// One Terminal tab's line-editor, output queue and Flow request queue
/// (T3c, #98). The seven Flow-side clients (storage, network, ...) stay
/// singletons shared across sessions (`services/images/src/flow.rs`):
/// only one slot's request is ever in flight with Flow at a time (see
/// `OWNER` below), so per-session state here is limited to what actually
/// differs per tab — the line editor and the queues feeding it.
struct SessionSlot {
    session: logos_session::SessionService,
    pending_output: PendingOutput,
    flow_input: PendingFlowInput,
    pending_completion: Option<IpcBytes>,
    waiting_for_command: bool,
    waiting_for_completion: bool,
    command_response: [u8; logos_session::MAX_OUTPUT_BYTES],
    command_response_len: usize,
}

impl SessionSlot {
    const fn new() -> Self {
        Self {
            session: logos_session::SessionService::new(),
            pending_output: PendingOutput::new(),
            flow_input: PendingFlowInput::new(),
            pending_completion: None,
            waiting_for_command: false,
            waiting_for_completion: false,
            command_response: [0; logos_session::MAX_OUTPUT_BYTES],
            command_response_len: 0,
        }
    }

    fn has_queued_flow_work(&self) -> bool {
        self.pending_completion.is_some() || !self.flow_input.is_empty()
    }

    /// Resets this slot back to a fresh prompt, for reuse by a tab opened
    /// after the one that used to own it was closed.
    fn reset(&mut self) {
        self.session = logos_session::SessionService::new();
        self.flow_input.clear();
        self.pending_completion = None;
        self.command_response_len = 0;
        let mut prompt = logos_session::ShellOutput::new();
        self.session.prompt(&mut prompt);
        self.pending_output.stage(prompt.as_bytes());
    }
}

const EMPTY_SLOT: SessionSlot = SessionSlot::new();
static mut SESSIONS: [SessionSlot; MAX_SHELL_SESSIONS] = [EMPTY_SLOT; MAX_SHELL_SESSIONS];
/// The one slot currently exchanging a command or completion request with
/// Flow; every other slot's line editing keeps working locally regardless
/// (T3c, #98's "one shared arbitration queue" approach).
static mut OWNER: Option<u8> = None;
/// Set when `OWNER`'s tab was closed while its exchange was still in
/// flight (`close_session`): `OWNER` itself stays put (Flow's shared
/// clients only ever serve one command at a time, so the slot cannot be
/// freed for a new owner until this exchange actually ends), but its
/// reply must not be staged into the slot any more — a new tab may
/// already have reused it. The reply handler drains and discards instead
/// until the exchange's terminal message, then clears both.
static mut ORPHANED: bool = false;
static mut PENDING_CONTROL: Option<IpcBytes> = None;

fn completion_request_message(request: CompletionRequest, session: u8) -> IpcBytes {
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&request as *const CompletionRequest).cast::<u8>(),
            core::mem::size_of::<CompletionRequest>(),
        )
    };
    IpcBytes::from_bytes(MessageKind::CompletionRequest, bytes)
        .unwrap_or_else(|| IpcBytes::empty(MessageKind::CompletionRequest))
        .with_session(session)
}

fn completion_response(message: &IpcBytes) -> Option<CompletionResponse> {
    (message.kind == MessageKind::CompletionResponse
        && message.len as usize == core::mem::size_of::<CompletionResponse>()
        && CompletionResponse::wire_enums_valid(
            &message.bytes[..core::mem::size_of::<CompletionResponse>()],
        ))
    .then(|| unsafe { core::ptr::read_unaligned(message.bytes.as_ptr().cast()) })
}

fn fetch_progress(message: &IpcBytes) -> Option<FetchResponse> {
    (message.kind == MessageKind::FlowProgress
        && message.len as usize == core::mem::size_of::<FetchResponse>()
        && FetchResponse::wire_enums_valid(&message.bytes[..core::mem::size_of::<FetchResponse>()]))
    .then(|| unsafe { core::ptr::read_unaligned(message.bytes.as_ptr().cast()) })
    .filter(|response: &FetchResponse| response.is_valid())
}

fn flow_control_message(request_id: u32, session: u8) -> IpcBytes {
    let control = FlowControl::cancel(request_id, session);
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&control as *const FlowControl).cast::<u8>(),
            core::mem::size_of::<FlowControl>(),
        )
    };
    IpcBytes::from_bytes(MessageKind::FlowControl, bytes)
        .unwrap_or_else(|| IpcBytes::empty(MessageKind::FlowControl))
        .with_session(session)
}

fn render_fetch_progress(response: FetchResponse, output: &mut logos_session::ShellOutput) {
    output.extend(b"\r\x1b[Kfetch: ");
    match response.phase {
        FetchPhase::Connect => output.extend(b"connecting"),
        FetchPhase::SendRequest => output.extend(b"request sent"),
        FetchPhase::ReadResponse => {
            output.push_decimal(response.downloaded_bytes as usize);
            output.extend(b" bytes");
        }
        FetchPhase::StageStorage => output.extend(b"staging"),
        FetchPhase::Commit => output.extend(b"committing"),
        _ => output.extend(match response.status {
            FetchStatus::Cancelled => &b"cancelled"[..],
            _ => &b"working"[..],
        }),
    }
}

/// Whether `message` is the last reply of a Flow exchange (a completion
/// response, or a `SessionOutput` chunk without `IPC_FLAG_MORE`) rather
/// than an intermediate fetch-progress or partial-output chunk. Shared by
/// the normal and orphaned-reply paths in `_start` so both agree on when
/// an exchange ends and `OWNER` can free up.
fn is_terminal_reply(message: &IpcBytes) -> bool {
    completion_response(message).is_some()
        || (fetch_progress(message).is_none()
            && message.kind == MessageKind::SessionOutput
            && message.flags & IPC_FLAG_MORE == 0)
}

/// Picks the next slot with queued Flow work, starting just after
/// `after` and wrapping once, so no tab is starved if several submit
/// commands back to back.
fn next_owner(sessions: &[SessionSlot; MAX_SHELL_SESSIONS], after: u8) -> Option<u8> {
    (1..=MAX_SHELL_SESSIONS)
        .map(|offset| (after as usize + offset) % MAX_SHELL_SESSIONS)
        .find(|&candidate| sessions[candidate].has_queued_flow_work())
        .map(|slot| slot as u8)
}

/// Cancels/drops a closed tab's queued or in-flight Flow work and resets
/// its slot for reuse (T3c, #98's close acceptance). A command already
/// sent to Flow keeps running to completion (Flow's shared clients can
/// only ever serve one command at a time; see `next_owner`): this only
/// requests its cancellation and marks `*orphaned` so the reply handler
/// drains and discards the eventual (possibly-cancelled) reply instead of
/// staging it into this slot, which a new tab may already have reused.
/// `OWNER` itself is left untouched here — forcing it free early would let
/// a second command start while Flow is still mid-command for the first.
fn close_session(
    sessions: &mut [SessionSlot; MAX_SHELL_SESSIONS],
    owner: Option<u8>,
    orphaned: &mut bool,
    pending_control: &mut Option<IpcBytes>,
    session: u8,
) {
    let Some(slot) = sessions.get_mut(session as usize) else { return };
    if owner == Some(session) && (slot.waiting_for_command || slot.waiting_for_completion) {
        *pending_control = Some(flow_control_message(0, session));
        *orphaned = true;
    }
    slot.reset();
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let sessions = unsafe { &mut *core::ptr::addr_of_mut!(SESSIONS) };
    let owner = unsafe { &mut *core::ptr::addr_of_mut!(OWNER) };
    let orphaned = unsafe { &mut *core::ptr::addr_of_mut!(ORPHANED) };
    let pending_control = unsafe { &mut *core::ptr::addr_of_mut!(PENDING_CONTROL) };
    let input_capability = match common::capability_handle(INPUT_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let output_capability = match common::capability_handle(OUTPUT_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let flow_capability = match common::capability_handle(FLOW_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let flow_output_capability = match common::capability_handle(FLOW_OUTPUT_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let mut command_bytes = [0; MAX_LINE_BYTES];
    for slot in sessions.iter_mut() {
        let mut prompt = logos_session::ShellOutput::new();
        slot.session.prompt(&mut prompt);
        slot.pending_output.stage(prompt.as_bytes());
    }
    let mut heartbeat_ticks = 0u16;
    loop {
        let busy = owner.is_some()
            || pending_control.is_some()
            || sessions.iter().any(|slot| !slot.pending_output.is_empty());
        if busy {
            common::heartbeat();
        } else {
            common::heartbeat_tick(&mut heartbeat_ticks);
        }
        let mut progressed = false;

        for (index, slot) in sessions.iter_mut().enumerate() {
            progressed |= slot.pending_output.flush(output_capability, index as u8);
        }

        if let Some(control) = *pending_control {
            match common::ipc_send_handle(flow_capability, &control) {
                IpcStatus::Ok => {
                    *pending_control = None;
                    progressed = true;
                }
                IpcStatus::Full => {}
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => *pending_control = None,
            }
        }

        if owner.is_none() {
            let start_after = sessions.len() as u8 - 1;
            if let Some(index) = next_owner(sessions, start_after) {
                let slot = &mut sessions[index as usize];
                if let Some(message) = slot.pending_completion.take() {
                    match common::ipc_send_handle(flow_capability, &message) {
                        IpcStatus::Ok => {
                            *owner = Some(index);
                            slot.waiting_for_completion = true;
                            progressed = true;
                        }
                        IpcStatus::Full => slot.pending_completion = Some(message),
                        IpcStatus::Stale
                        | IpcStatus::Disconnected
                        | IpcStatus::Unauthorized
                        | IpcStatus::Malformed
                        | IpcStatus::Empty => {
                            let mut failure = logos_session::ShellOutput::new();
                            slot.session.completion_failed(&mut failure);
                            slot.pending_output.stage(failure.as_bytes());
                        }
                    }
                } else if let Some(message) = slot.flow_input.take(index) {
                    match common::ipc_send_handle(flow_capability, &message) {
                        IpcStatus::Ok => {
                            *owner = Some(index);
                            slot.waiting_for_command = true;
                            progressed = true;
                        }
                        IpcStatus::Full => {
                            slot.flow_input.stage(message.as_bytes().unwrap_or_default());
                        }
                        IpcStatus::Stale
                        | IpcStatus::Disconnected
                        | IpcStatus::Unauthorized
                        | IpcStatus::Malformed
                        | IpcStatus::Empty => {}
                    }
                }
            }
        }

        if let Some(index) = *owner {
            let mut message = IpcBytes::empty(MessageKind::SessionOutput);
            if common::ipc_receive_handle(flow_output_capability, &mut message) == IpcStatus::Ok {
                progressed = true;
                if *orphaned {
                    // This slot's tab was closed while this exchange was
                    // still in flight and may already be reused by a new
                    // tab (`close_session`): discard the reply instead of
                    // staging it or touching the slot's line editor, and
                    // only track whether it was this exchange's terminal
                    // message so `OWNER` can free up for the next one.
                    if is_terminal_reply(&message) {
                        *owner = None;
                        *orphaned = false;
                    }
                } else {
                    let slot = &mut sessions[index as usize];
                    if let Some(response) = completion_response(&message) {
                        let mut edit_output = logos_session::ShellOutput::new();
                        slot.session.apply_completion_response(response, &mut edit_output);
                        if edit_output.len > 0 {
                            slot.pending_output.stage(edit_output.as_bytes());
                        }
                        slot.waiting_for_completion = false;
                        *owner = None;
                    } else if let Some(response) = fetch_progress(&message) {
                        let mut progress = logos_session::ShellOutput::new();
                        render_fetch_progress(response, &mut progress);
                        slot.pending_output.stage(progress.as_bytes());
                    } else if message.kind == MessageKind::SessionOutput {
                        if let Some(bytes) = message.as_bytes() {
                            let available = slot.command_response.len() - slot.command_response_len;
                            let count = bytes.len().min(available);
                            slot.command_response
                                [slot.command_response_len..slot.command_response_len + count]
                                .copy_from_slice(&bytes[..count]);
                            slot.command_response_len += count;
                            if message.flags & IPC_FLAG_MORE == 0 {
                                let mut result = logos_session::ShellOutput::new();
                                slot.session.command_output(
                                    &slot.command_response[..slot.command_response_len],
                                    &mut result,
                                );
                                slot.pending_output.stage(result.as_bytes());
                                slot.command_response_len = 0;
                                slot.waiting_for_command = false;
                                *owner = None;
                            }
                        }
                    }
                }
            }
        }

        let mut message = IpcBytes::empty(MessageKind::SessionInput);
        while common::ipc_receive_handle(input_capability, &mut message) == IpcStatus::Ok {
            progressed = true;
            let tag = message.session();
            if message.kind == MessageKind::SessionClose {
                close_session(sessions, *owner, orphaned, pending_control, tag);
                continue;
            }
            if message.kind != MessageKind::SessionInput || tag as usize >= sessions.len() {
                continue;
            }
            let slot = &mut sessions[tag as usize];
            let Some(bytes) = message.as_bytes() else { continue };
            if *owner == Some(tag) && slot.waiting_for_command && bytes == [0x03] {
                *pending_control = Some(flow_control_message(0, tag));
                continue;
            }
            let mut edit_output = logos_session::ShellOutput::new();
            if let Some(length) =
                slot.session.input_for_command(bytes, &mut command_bytes, &mut edit_output)
            {
                slot.flow_input.stage(&command_bytes[..length]);
            }
            if let Some(request) = slot.session.take_completion_request() {
                slot.pending_completion = Some(completion_request_message(request, tag));
            }
            if edit_output.len > 0 {
                slot.pending_output.stage(edit_output.as_bytes());
            }
        }

        for slot in sessions.iter_mut() {
            let mut completion_output = logos_session::ShellOutput::new();
            slot.session.completion_tick(&mut completion_output);
            if completion_output.len > 0 {
                slot.pending_output.stage(completion_output.as_bytes());
            }
        }

        if !progressed {
            common::wait_on_capabilities(&[input_capability, flow_output_capability]);
        }
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_cap_stays_four() {
        // T3c (#98) keeps T3's tab cap (#76); a fifth slot is not added.
        assert_eq!(MAX_SHELL_SESSIONS, 4);
        let sessions: [SessionSlot; MAX_SHELL_SESSIONS] = [EMPTY_SLOT; MAX_SHELL_SESSIONS];
        assert_eq!(sessions.len(), 4);
    }

    #[test]
    fn each_slot_line_edits_independently() {
        // Two tabs typing at once must not see each other's line buffer or
        // history (T3c, #98's "variables and cwd are isolated per session"
        // extends to the line editor itself).
        let mut sessions: [SessionSlot; MAX_SHELL_SESSIONS] = [EMPTY_SLOT; MAX_SHELL_SESSIONS];
        let mut command = [0u8; MAX_LINE_BYTES];
        let mut discard = logos_session::ShellOutput::new();
        sessions[0].session.input_for_command(b"one", &mut command, &mut discard);
        sessions[1].session.input_for_command(b"two", &mut command, &mut discard);
        let mut output_a = logos_session::ShellOutput::new();
        let length_a = sessions[0]
            .session
            .input_for_command(b"\r", &mut command, &mut output_a)
            .expect("session A's line commits on its own Enter");
        assert_eq!(&command[..length_a], b"one");
        let mut output_b = logos_session::ShellOutput::new();
        let length_b = sessions[1]
            .session
            .input_for_command(b"\r", &mut command, &mut output_b)
            .expect("session B's line commits on its own Enter");
        assert_eq!(&command[..length_b], b"two");
    }

    #[test]
    fn next_owner_round_robins_over_slots_with_queued_flow_work() {
        let mut sessions: [SessionSlot; MAX_SHELL_SESSIONS] = [EMPTY_SLOT; MAX_SHELL_SESSIONS];
        assert_eq!(next_owner(&sessions, 0), None, "nothing queued yet");
        sessions[3].flow_input.stage(b"sys.version()");
        sessions[1].flow_input.stage(b"sys.uname()");
        // Starting just after slot 0 finds slot 1 before wrapping to 3.
        assert_eq!(next_owner(&sessions, 0), Some(1));
        // Starting just after slot 1 skips straight to 3, wrapping past 2.
        assert_eq!(next_owner(&sessions, 1), Some(3));
    }

    #[test]
    fn closing_a_queued_but_not_yet_owned_session_drops_its_command_silently() {
        // Session B queued a command but Flow hasn't started it yet
        // (`OWNER` is still A, or nobody): closing B must drop the queued
        // line without ever touching Flow.
        let mut sessions: [SessionSlot; MAX_SHELL_SESSIONS] = [EMPTY_SLOT; MAX_SHELL_SESSIONS];
        sessions[1].flow_input.stage(b"sys.version()");
        let mut pending_control = None;
        let mut orphaned = false;
        close_session(&mut sessions, Some(0), &mut orphaned, &mut pending_control, 1);
        assert!(sessions[1].flow_input.is_empty());
        assert!(pending_control.is_none(), "a merely-queued command needs no cancel sent to Flow");
        assert!(!orphaned, "no exchange was in flight for the closed session");
    }

    #[test]
    fn closing_the_owning_session_sends_a_tagged_cancel_to_flow_and_marks_it_orphaned() {
        // Session A's command is already in flight with Flow (`OWNER` is
        // A): closing A must ask Flow to cancel it, tagged to A, rather
        // than silently forgetting about it (Flow's shared clients would
        // otherwise keep driving a command nobody can see the result of),
        // and mark the exchange orphaned so its eventual reply is drained
        // instead of staged into whatever tab reuses the slot.
        let mut sessions: [SessionSlot; MAX_SHELL_SESSIONS] = [EMPTY_SLOT; MAX_SHELL_SESSIONS];
        sessions[0].waiting_for_command = true;
        let mut pending_control = None;
        let mut orphaned = false;
        close_session(&mut sessions, Some(0), &mut orphaned, &mut pending_control, 0);
        let control = pending_control.expect("the owning session's cancel is queued to send");
        assert_eq!(control.kind, MessageKind::FlowControl);
        assert_eq!(control.session(), 0);
        assert!(orphaned);
    }

    #[test]
    fn closing_a_session_resets_it_for_reuse() {
        let mut sessions: [SessionSlot; MAX_SHELL_SESSIONS] = [EMPTY_SLOT; MAX_SHELL_SESSIONS];
        let mut command = [0u8; MAX_LINE_BYTES];
        let mut discard = logos_session::ShellOutput::new();
        sessions[2].session.input_for_command(b"leftover", &mut command, &mut discard);
        sessions[2].pending_completion = Some(IpcBytes::empty(MessageKind::CompletionRequest));
        let mut pending_control = None;
        let mut orphaned = false;
        close_session(&mut sessions, None, &mut orphaned, &mut pending_control, 2);
        assert!(sessions[2].pending_completion.is_none());
        assert!(sessions[2].flow_input.is_empty());
        // A fresh prompt is queued for whatever tab reuses this slot next.
        assert!(!sessions[2].pending_output.is_empty());
    }

    /// The exact race the coordinator's review flagged: A submits, A
    /// closes (so B can immediately reuse its slot), then the late reply
    /// for A's cancelled command arrives. It must not land in B's output,
    /// and `OWNER`/`ORPHANED` must both clear on that reply's terminal
    /// message so the slot is free for the next exchange.
    #[test]
    fn a_late_reply_for_a_closed_and_reused_slot_is_drained_not_staged() {
        let mut sessions: [SessionSlot; MAX_SHELL_SESSIONS] = [EMPTY_SLOT; MAX_SHELL_SESSIONS];
        sessions[0].waiting_for_command = true;
        let mut owner = Some(0u8);
        let mut orphaned = false;
        let mut pending_control = None;

        // A closes while its command is still in flight.
        close_session(&mut sessions, owner, &mut orphaned, &mut pending_control, 0);
        assert!(orphaned);
        assert_eq!(owner, Some(0), "OWNER stays put; Flow is still mid-command for slot 0");

        // A new tab reuses slot 0 (Terminal picks the lowest free slot)
        // and types into it before A's stale reply ever arrives.
        let mut command = [0u8; MAX_LINE_BYTES];
        let mut new_tab_output = logos_session::ShellOutput::new();
        sessions[0].session.input_for_command(b"fresh", &mut command, &mut new_tab_output);
        let new_tab_queued_before_reply = sessions[0].pending_output.len;

        // A's stale, cancelled command's terminal reply finally arrives,
        // tagged (by Flow) to slot 0 same as always. This is exactly the
        // `*orphaned` branch of `_start`'s `*owner` block.
        let final_chunk = IpcBytes::empty(MessageKind::SessionOutput);
        assert!(
            is_terminal_reply(&final_chunk),
            "an empty, non-MORE SessionOutput is this exchange's terminal message"
        );
        if orphaned && is_terminal_reply(&final_chunk) {
            owner = None;
            orphaned = false;
        }

        assert_eq!(owner, None, "OWNER clears once the orphaned exchange's reply is drained");
        assert!(!orphaned);
        assert_eq!(
            sessions[0].pending_output.len, new_tab_queued_before_reply,
            "the closed session's late reply must not add to the reused slot's output"
        );
    }
}
