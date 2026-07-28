#![no_main]
#![no_std]

use core::arch::asm;
use logos_core::native_service::{
    ACKNOWLEDGED, CLEAR_DISPLAY, COMPLETE, Context, Header, PRESENT_TEXT, READ_INPUT, READY,
    SUBMIT_COMMAND,
};
use logos_terminal::terminal::Model;
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
            if let Ok(input) = u8::try_from((*context).input) {
                if input == b'\n' {
                    let submission = terminal.submit();
                    if submission.as_bytes().len() <= 32 {
                        (*context).text = [0; 32];
                        (&mut (*context).text)[..submission.as_bytes().len()]
                            .copy_from_slice(submission.as_bytes());
                        (*context).text_length = submission.as_bytes().len() as u32;
                        (*context).operation = SUBMIT_COMMAND;
                        asm!("int 0x80");
                        let length = usize::try_from((*context).text_length).unwrap_or(0).min(32);
                        let _ = terminal.write_output(&(*context).text[..length]);
                    } else {
                        let _ = terminal.write_output(b"command too long");
                    }
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

unsafe fn render(terminal: &Model, context: *mut Context) {
    unsafe {
        (*context).operation = CLEAR_DISPLAY;
        asm!("int 0x80");
        let mut row = 0u32;
        while let Some(line) = terminal.output_line(row as usize) {
            present(context, 32, 32 + row * 8, line.as_bytes());
            row += 1;
        }
        present(context, 32, 32 + row * 8, b">");
        present(context, 38, 32 + row * 8, terminal.input_line());
    }
}

unsafe fn present(context: *mut Context, x: u32, y: u32, bytes: &[u8]) {
    unsafe {
        for (chunk, bytes) in bytes.chunks(32).enumerate() {
            (*context).text = [0; 32];
            (&mut (*context).text)[..bytes.len()].copy_from_slice(bytes);
            (*context).x = x + u32::try_from(chunk * 32 * 6).unwrap_or(u32::MAX);
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
