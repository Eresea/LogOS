#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    INPUT_KEYBOARD_RING_BASE, INPUT_POINTER_RING_BASE, IPC_CONTRACT_GUI_INPUT, InputMessage,
    IpcStatus, KeyboardByteRing, PointerByteRing,
};

static mut PENDING: Option<InputMessage> = None;
static mut SENT_MASK: u8 = 0;
static mut PENDING_POINTER: Option<InputMessage> = None;
static mut POINTER_SENT_MASK: u8 = 0;

#[cfg(feature = "qemu-proof")]
fn proof_line(message: &[u8]) {
    common::proof_line(message);
}

#[cfg(not(feature = "qemu-proof"))]
fn proof_line(_message: &[u8]) {}

fn flush_pointer(
    capability: logos_abi::CapabilityHandle,
    pending: &mut Option<InputMessage>,
    sent_mask: &mut u8,
) -> bool {
    let Some(message) = *pending else { return false };
    if *sent_mask == 0 {
        match common::ipc_send_handle(capability, &message) {
            IpcStatus::Ok
            | IpcStatus::Stale
            | IpcStatus::Disconnected
            | IpcStatus::Unauthorized
            | IpcStatus::Malformed
            | IpcStatus::Empty => *sent_mask = 1,
            IpcStatus::Full => return false,
        }
    }
    if *sent_mask == 1 {
        *pending = None;
        *sent_mask = 0;
    }
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let shell_output = match common::discover_capability_contract_named(
        b"shell",
        logos_abi::IpcRights::Send,
        IPC_CONTRACT_GUI_INPUT,
        core::mem::size_of::<InputMessage>(),
    ) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let atrium_output = match common::discover_capability_contract_named(
        b"atrium",
        logos_abi::IpcRights::Send,
        IPC_CONTRACT_GUI_INPUT,
        core::mem::size_of::<InputMessage>(),
    ) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let keyboard = unsafe { &*(INPUT_KEYBOARD_RING_BASE as *const KeyboardByteRing) };
    let pointer = unsafe { &*(INPUT_POINTER_RING_BASE as *const PointerByteRing) };
    let pending = unsafe { &mut *core::ptr::addr_of_mut!(PENDING) };
    let sent_mask = unsafe { &mut *core::ptr::addr_of_mut!(SENT_MASK) };
    let pending_pointer = unsafe { &mut *core::ptr::addr_of_mut!(PENDING_POINTER) };
    let pointer_sent_mask = unsafe { &mut *core::ptr::addr_of_mut!(POINTER_SENT_MASK) };
    let mut decoder = logos_input::InputDecoder::new();
    let mut pointer_decoder = logos_input::PointerDecoder::new();
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks);
        if pending_pointer.is_some() {
            let _ = flush_pointer(atrium_output, pending_pointer, pointer_sent_mask);
        }
        if let Some(message) = *pending {
            for (index, capability) in [shell_output, atrium_output].into_iter().enumerate() {
                let bit = 1 << index;
                if *sent_mask & bit != 0 {
                    continue;
                }
                match common::ipc_send_handle(capability, &message) {
                    IpcStatus::Ok => *sent_mask |= bit,
                    IpcStatus::Full => {
                        common::wait_on_capability_or_input(capability);
                        continue;
                    }
                    IpcStatus::Stale
                    | IpcStatus::Disconnected
                    | IpcStatus::Unauthorized
                    | IpcStatus::Malformed
                    | IpcStatus::Empty => *sent_mask |= bit,
                }
            }
            if *sent_mask == 0b11 {
                *pending = None;
                *sent_mask = 0;
            }
            continue;
        }
        if let Some(byte) = keyboard.pop() {
            if let Some(event) = decoder.feed(byte) {
                proof_line(b"LogOS vNext: Input decoded keyboard event");
                *pending = Some(event.terminal_message());
                *sent_mask = 0;
            }
            continue;
        }
        if pending_pointer.is_none() {
            if let Some(byte) = pointer.pop() {
                if let Some(event) = pointer_decoder.feed(byte) {
                    *pending_pointer = Some(event);
                    *pointer_sent_mask = 0;
                    let _ = flush_pointer(atrium_output, pending_pointer, pointer_sent_mask);
                }
                continue;
            }
        }
        if pending_pointer.is_some() {
            common::wait_on_capability_or_input(atrium_output);
        } else {
            common::sleep_on_input();
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
