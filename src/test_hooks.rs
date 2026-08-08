use crate::debug;
use core::arch::asm;
use logos_core::test_protocol::{self, Request};

const COM2: u16 = 0x2f8;
const DEBUG_EXIT: u16 = 0xf4;

pub enum Action<'a> {
    Input(&'a str),
    Advance(u64),
    Query(&'a str),
    Poll,
    Run(&'a str),
}

pub fn serve(storage: u32, mut handle: impl FnMut(Action<'_>) -> bool) -> ! {
    init();
    line(match storage {
        logos_core::native_service::STORAGE_FORMATTED => {
            b"LOGOS/1 BOOT session=1 storage=formatted"
        }
        logos_core::native_service::STORAGE_RECOVERED => {
            b"LOGOS/1 BOOT session=1 storage=recovered"
        }
        logos_core::native_service::STORAGE_RECOVERED_INCOMPLETE => {
            b"LOGOS/1 BOOT session=1 storage=recovered-incomplete"
        }
        logos_core::native_service::STORAGE_CORRUPT => b"LOGOS/1 BOOT session=1 storage=corrupt",
        logos_core::native_service::STORAGE_UNAVAILABLE => {
            b"LOGOS/1 BOOT session=1 storage=unavailable"
        }
        _ => b"LOGOS/1 BOOT session=1 storage=io-failed",
    });
    line(b"LOGOS/1 READY stage=session-ready");
    let mut frame = [0u8; test_protocol::MAX_FRAME];
    loop {
        let length = read_frame(&mut frame, &mut || {
            let _ = handle(Action::Poll);
        });
        match test_protocol::parse(&frame[..length]) {
            Ok(Request::Hello) => line(b"LOGOS/1 RESULT hello=ok"),
            Ok(Request::Run(id)) if handle(Action::Run(id)) => {
                debug::write(b"LOGOS/1 EVENT id=");
                debug::write(id.as_bytes());
                debug::write_line(b" state=passed");
                write(b"LOGOS/1 EVENT id=");
                write(id.as_bytes());
                line(b" state=passed");
                debug::write(b"LOGOS/1 RESULT scenario=");
                debug::write(id.as_bytes());
                debug::write_line(b" status=passed");
                write(b"LOGOS/1 RESULT scenario=");
                write(id.as_bytes());
                line(b" status=passed");
            }
            Ok(Request::Run(id)) => {
                write(b"LOGOS/1 ERROR scenario=");
                write(id.as_bytes());
                line(b" reason=unavailable");
            }
            Ok(Request::Inject { point, action }) if fault_point(point) && valid_action(action) => {
                line(b"LOGOS/1 RESULT inject=accepted")
            }
            Ok(Request::Advance(ticks)) if handle(Action::Advance(ticks)) => {
                line(b"LOGOS/1 RESULT advance=accepted")
            }
            Ok(Request::Advance(_)) => line(b"LOGOS/1 ERROR advance=rejected"),
            Ok(Request::Query(query)) if handle(Action::Query(query)) => {
                write(b"LOGOS/1 RESULT query=");
                write(query.as_bytes());
                line(b" status=ready")
            }
            Ok(Request::Query(query)) => {
                write(b"LOGOS/1 RESULT query=");
                write(query.as_bytes());
                line(b" status=pending")
            }
            Ok(Request::Input(value)) if handle(Action::Input(value)) => {
                line(b"LOGOS/1 RESULT input=accepted")
            }
            Ok(Request::Input(_)) => line(b"LOGOS/1 ERROR input=rejected"),
            Ok(Request::Reset(_)) if handle(Action::Input("__reset")) => {
                line(b"LOGOS/1 RESULT reset=accepted")
            }
            Ok(Request::Reset(_)) => line(b"LOGOS/1 ERROR reset=failed"),
            Ok(Request::Shutdown) => exit(0),
            Ok(_) => line(b"LOGOS/1 ERROR reason=unavailable"),
            Err(_) => line(b"LOGOS/1 ERROR reason=invalid-frame"),
        }
    }
}

fn fault_point(point: &str) -> bool {
    matches!(
        point,
        "supervisor.before_service_start" | "ipc.after_enqueue" | "driver.before_completion"
    )
}
fn valid_action(action: &str) -> bool {
    matches!(action, "fail-once" | "fail-always" | "delay" | "drop" | "crash-owner")
}
fn init() {
    out(COM2 + 1, 0);
    out(COM2 + 3, 0x80);
    out(COM2, 1);
    out(COM2 + 1, 0);
    out(COM2 + 3, 3);
    out(COM2 + 2, 0xc7);
    out(COM2 + 4, 0x0b);
}
fn read_frame(frame: &mut [u8], poll: &mut impl FnMut()) -> usize {
    let mut length = 0;
    loop {
        while input(COM2 + 5) & 1 == 0 {
            poll();
            core::hint::spin_loop();
        }
        let byte = input(COM2);
        if byte == b'\n' {
            return length;
        }
        if byte != b'\r' && length < frame.len() {
            frame[length] = byte;
            length += 1;
        }
    }
}
fn line(value: &[u8]) {
    write(value);
    write(b"\r\n");
    debug::write_line(value);
}
fn write(value: &[u8]) {
    for &byte in value {
        while input(COM2 + 5) & 0x20 == 0 {
            core::hint::spin_loop();
        }
        out(COM2, byte);
    }
}
fn input(port: u16) -> u8 {
    let value;
    unsafe { asm!("in al, dx", in("dx") port, out("al") value) };
    value
}
fn out(port: u16, value: u8) {
    unsafe { asm!("out dx, al", in("dx") port, in("al") value) };
}
fn exit(code: u8) -> ! {
    out(DEBUG_EXIT, code);
    loop {
        unsafe { asm!("cli", "hlt") };
    }
}
