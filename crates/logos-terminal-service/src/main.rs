#![no_main]
#![no_std]

use logos_abi::Syscall;
use logos_service_rt::{Context, Header, MAX_TEXT};
use logos_terminal::{
    command::{self, Local, Resolution},
    terminal::Model,
};

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header = Header::new(*b"terminal\0\0\0\0\0\0\0\0", logos_service_entry);

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: logos_service_rt::EntryContext) -> ! {
    logos_service_rt::entry(context, run)
}

fn run(context: &mut Context) -> ! {
    if !context.ready() {
        spin();
    }
    let mut terminal = Model::new();
    let _ = terminal.write_output(b"LOGOS RING3 TERMINAL");
    while context.acknowledged() {
        render(&mut terminal, context);
        if !context.wait_for_input() {
            spin();
        }
        if context.input() == 0x1b {
            let _ = context.complete();
            spin();
        }
        let Some(input) = context.input_byte().and_then(logos_abi::InputEvent::from_byte) else {
            continue;
        };
        match input.byte() {
            b'\n' => submit_line(&mut terminal, context),
            0x08 => {
                let _ = terminal.backspace();
            }
            0x1b => {}
            byte => {
                let _ = terminal.insert_utf8(&[byte]);
            }
        }
    }
    spin()
}

fn submit_line(terminal: &mut Model, context: &mut Context) {
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

fn submit_call(terminal: &mut Model, context: &mut Context, syscall: Syscall, argument: &[u8]) {
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

fn render(terminal: &mut Model, context: &mut Context) {
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

fn present(context: &mut Context, x: u32, y: u32, bytes: &[u8]) {
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
