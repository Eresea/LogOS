use crate::debug;

pub struct Startup;

impl Startup {
    pub fn new() -> Self {
        debug::write_line(b"LogOS: startup self check");
        Self
    }

    pub fn check(&self, module: &[u8], passed: bool) {
        debug::write(b"LogOS: check ");
        debug::write(module);
        if passed {
            debug::write_line(b" passed");
        } else {
            debug::write_line(b" failed");
            self.halt();
        }
    }

    pub fn fail(&self, module: &[u8]) -> ! {
        self.check(module, false);
        unreachable!()
    }

    pub fn finish(&self) {
        debug::write_line(b"LogOS: startup self check passed");
    }

    fn halt(&self) -> ! {
        loop {
            unsafe { core::arch::asm!("cli", "hlt") };
        }
    }
}

pub fn driver_failure(driver: &[u8], recovered: bool) {
    debug::write(b"LogOS: driver ");
    debug::write(driver);
    debug::write_line(if recovered { b" recovered" } else { b" failed" });
}
