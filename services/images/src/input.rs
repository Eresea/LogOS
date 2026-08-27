#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    INPUT_KEYBOARD_RING_BASE, IPC_CONTRACT_GUI_INPUT, InputMessage, IpcStatus, KeyboardByteRing,
};

static mut PENDING: Option<InputMessage> = None;
static mut SENT_MASK: u8 = 0;

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
    let pending = unsafe { &mut *core::ptr::addr_of_mut!(PENDING) };
    let sent_mask = unsafe { &mut *core::ptr::addr_of_mut!(SENT_MASK) };
    let mut decoder = logos_input::InputDecoder::new();
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks);
        if let Some(message) = *pending {
            for (index, capability) in [shell_output, atrium_output].into_iter().enumerate() {
                let bit = 1 << index;
                if *sent_mask & bit != 0 {
                    continue;
                }
                match common::ipc_send_handle(capability, &message) {
                    IpcStatus::Ok => *sent_mask |= bit,
                    IpcStatus::Full => {
                        common::wait_on_capability(capability);
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
        let Some(byte) = keyboard.pop() else {
            common::sleep_on_keyboard();
            continue;
        };
        let Some(event) = decoder.feed(byte) else {
            continue;
        };
        *pending = Some(event.terminal_message());
        *sent_mask = 0;
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
