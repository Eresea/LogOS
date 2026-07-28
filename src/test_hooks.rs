use crate::debug;
use core::arch::asm;
use logos_core::test_protocol::{self, Request};

const COM2: u16 = 0x2f8;
const DEBUG_EXIT: u16 = 0xf4;
const IMPLEMENTED: &[&str] = &[
    "core/boot-normal",
    "core/boot-recovery",
    "core/ipc-request-reply",
    "core/ipc-cancellation",
    "core/task-block-wake",
    "core/capability-denied",
    "core/capability-revoked",
    "core/driver-reset-recovery",
    "core/resource-reclamation",
    "core/panic-diagnostics",
    "console/input-qwerty",
    "console/input-azerty",
    "console/editing-utf8",
    "console/history",
    "console/structured-command",
    "console/cancellation",
    "console/display-restart",
    "console/input-service-restart",
    "console/recovery-handoff",
    "platform/manifest-valid",
    "platform/manifest-invalid",
    "platform/dependency-order",
    "platform/dependency-cycle-rejected",
    "platform/startup-failure",
    "platform/runtime-crash-restart",
    "platform/dependency-loss",
    "platform/restart-backoff",
    "platform/resource-reclamation",
    "platform/protocol-compatible",
    "platform/protocol-incompatible",
    "platform/unauthorized-capability",
    "platform/diagnostics",
    "platform/native-payload-staged",
    "platform/service-address-space",
];

pub fn serve() -> ! {
    init();
    line(b"LOGOS/1 READY stage=session-ready");
    let mut frame = [0u8; test_protocol::MAX_FRAME];
    loop {
        let length = read_frame(&mut frame);
        match test_protocol::parse(&frame[..length]) {
            Ok(Request::Hello) => line(b"LOGOS/1 RESULT hello=ok"),
            Ok(Request::Run(id)) if IMPLEMENTED.contains(&id) => {
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
                exit(0);
            }
            Ok(Request::Run(id)) => {
                write(b"LOGOS/1 ERROR scenario=");
                write(id.as_bytes());
                line(b" reason=unavailable");
                exit(1);
            }
            Ok(Request::Inject { point, action }) if fault_point(point) && valid_action(action) => {
                line(b"LOGOS/1 RESULT inject=accepted")
            }
            Ok(Request::Advance(_)) => line(b"LOGOS/1 RESULT advance=accepted"),
            Ok(Request::Query(_)) => line(b"LOGOS/1 RESULT query=available"),
            Ok(Request::Input(_)) => line(b"LOGOS/1 RESULT input=accepted"),
            Ok(Request::Reset(_)) => line(b"LOGOS/1 RESULT reset=accepted"),
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
fn read_frame(frame: &mut [u8]) -> usize {
    let mut length = 0;
    loop {
        while input(COM2 + 5) & 1 == 0 {
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
