#![no_main]
#![no_std]

use core::arch::asm;
use logos_core::native_service::{
    ACKNOWLEDGED, CLEAR_DISPLAY, COMPLETE, Context, Header, MAX_TEXT, PRESENT_TEXT, READ_INPUT,
    READY, SYSCALL, Syscall,
};
use logos_terminal::{
    command::{self, Call, Local, Resolution},
    terminal::Model,
};
use uefi::{Status, prelude::*};

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header = Header::new(*b"terminal\0\0\0\0\0\0\0\0", logos_service_entry);

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: *mut Context) -> ! {
    unsafe {
        (*context).operation = READY;
        asm!("int 0x80");
        let mut terminal = Model::new();
        let _ = terminal.write_output(b"LOGOS RING3 TERMINAL");
        while (*context).status == ACKNOWLEDGED {
            render(&terminal, context);
            (*context).operation = READ_INPUT;
            asm!("int 0x80");
            if (*context).input == 0x1b {
                (*context).operation = COMPLETE;
                asm!("int 0x80");
            }
            if let Ok(input) = u8::try_from((*context).input)
                && let Some(input) = logos_abi::InputEvent::from_byte(input)
            {
                let input = input.byte();
                if input == b'\n' {
                    let submission = terminal.submit();
                    let _ = terminal.write_output(submission.as_bytes());
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
                        Resolution::Local(Local::Layout(layout)) => submit_with_argument(
                            context,
                            Syscall::SetInputLayout,
                            &[match layout {
                                logos_terminal::input::Layout::Qwerty => {
                                    logos_abi::InputLayout::Qwerty.wire()
                                }
                                logos_terminal::input::Layout::Azerty => {
                                    logos_abi::InputLayout::Azerty.wire()
                                }
                            }],
                            &mut terminal,
                        ),
                        Resolution::Call(call) => match call_command(call) {
                            Some(command) => submit_call(context, command, call, &mut terminal),
                            None => {
                                let _ = terminal.write_output(b"unknown command");
                            }
                        },
                        Resolution::Error(_) => {
                            let _ = terminal.write_output(b"unknown command");
                        }
                    }
                } else if input == 0x08 {
                    let _ = terminal.backspace();
                } else if input != 0x1b {
                    let _ = terminal.insert_utf8(&[input]);
                }
            }
        }
    }
    loop {
        core::hint::spin_loop();
    }
}

unsafe fn submit_call(context: *mut Context, syscall: Syscall, call: Call, terminal: &mut Model) {
    unsafe {
        if let Some(value) = call.argument {
            submit_with_argument(context, syscall, value.as_bytes(), terminal)
        } else {
            submit_with_argument(context, syscall, &[], terminal)
        }
    }
}

unsafe fn submit_with_argument(
    context: *mut Context,
    syscall: Syscall,
    argument: &[u8],
    terminal: &mut Model,
) {
    unsafe {
        (*context).x = syscall as u32;
        (*context).text = [0; MAX_TEXT];
        (&mut (*context).text)[..argument.len()].copy_from_slice(argument);
        (*context).text_length = argument.len() as u32;
        (*context).operation = SYSCALL;
        asm!("int 0x80");
        let length = usize::try_from((*context).text_length).unwrap_or(0).min(MAX_TEXT);
        for line in (&(*context).text)[..length].split(|byte| *byte == b'\n') {
            if !line.is_empty() {
                let _ = terminal.write_output(line);
            }
        }
    }
}

fn call_command(call: Call) -> Option<Syscall> {
    match call.name {
        b"recovery" => Some(Syscall::Recovery),
        b"reboot" => Some(Syscall::Reboot),
        b"poweroff" => Some(Syscall::PowerOff),
        b"ping" => Some(Syscall::Ping),
        b"tasks" => Some(Syscall::Tasks),
        b"services" => Some(Syscall::Services),
        b"drivers" => Some(Syscall::Drivers),
        b"trace" => Some(Syscall::Trace),
        b"inspect" => Some(Syscall::Inspect),
        b"restart" => Some(Syscall::Restart),
        b"cancel" => Some(Syscall::Cancel),
        _ => None,
    }
}

unsafe fn render(terminal: &Model, context: *mut Context) {
    unsafe {
        (*context).operation = CLEAR_DISPLAY;
        asm!("int 0x80");
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
}

unsafe fn present(context: *mut Context, x: u32, y: u32, bytes: &[u8]) {
    unsafe {
        for (chunk, bytes) in bytes.chunks(MAX_TEXT).enumerate() {
            (*context).text = [0; MAX_TEXT];
            (&mut (*context).text)[..bytes.len()].copy_from_slice(bytes);
            (*context).x = x + u32::try_from(chunk * MAX_TEXT * 8).unwrap_or(u32::MAX);
            (*context).y = y;
            (*context).text_length = bytes.len() as u32;
            (*context).color = 0x0000_ff00;
            (*context).operation = PRESENT_TEXT;
            asm!("int 0x80");
        }
    }
}

#[entry]
fn main() -> Status {
    let _ = logos_terminal::terminal::Model::new();
    Status::SUCCESS
}
