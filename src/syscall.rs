//! Pure syscall decode and saved-register view for vector 49 (ADR-0009).
//!
//! This module owns the only knowledge of the saved-GPR layout and the only
//! raw reads and writes of it. It is host-testable; the kernel dispatch in
//! `user_mode` consumes it.

/// Word index of each saved register in the frame built by `context_common`
/// in `arch/context.rs`. The stub pushes rax first and r15 last, so r15 sits
/// at the lowest address (word 0) and rax at word 14.
pub(crate) mod slot {
    pub(crate) const RDI: usize = 8;
    pub(crate) const RSI: usize = 9;
    pub(crate) const RAX: usize = 14;
}

/// Number of saved GPR words below the vector (r15..rax).
pub(crate) const GPR_WORDS: usize = 15;

/// Word index inside the fxsave area holding the saved-GPR pointer
/// (byte offset 512, written by `context_common`).
pub(crate) const FX_GPR_POINTER_WORD: usize = 64;

/// Calling convention: `rax` = number and return value, `rdi` = arg 0,
/// `rsi` = arg 1.
const ARG_SLOTS: [usize; 2] = [slot::RDI, slot::RSI];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Syscall {
    Yield,
    Heartbeat,
    Power,
    Manager,
    Exit,
    IpcSend,
    IpcReceive,
    HeapGrow,
    HeapShrink,
    Directory,
    Event,
    CurrentTicks,
    DebugLine,
    WallTime,
    SystemInfo,
}

impl TryFrom<usize> for Syscall {
    type Error = ();

    fn try_from(number: usize) -> Result<Self, ()> {
        Ok(match number {
            logos_abi::YIELD_SYSCALL => Self::Yield,
            logos_abi::SERVICE_HEARTBEAT_SYSCALL => Self::Heartbeat,
            logos_abi::POWER_SYSCALL => Self::Power,
            logos_abi::MANAGER_SYSCALL => Self::Manager,
            logos_abi::PROGRAM_EXIT_SYSCALL => Self::Exit,
            logos_abi::IPC_SYSCALL_SEND => Self::IpcSend,
            logos_abi::IPC_SYSCALL_RECEIVE => Self::IpcReceive,
            logos_abi::SERVICE_HEAP_GROW_SYSCALL => Self::HeapGrow,
            logos_abi::SERVICE_HEAP_SHRINK_SYSCALL => Self::HeapShrink,
            logos_abi::SERVICE_DIRECTORY_SYSCALL => Self::Directory,
            logos_abi::EVENT_SYSCALL => Self::Event,
            logos_abi::CURRENT_TICKS_SYSCALL => Self::CurrentTicks,
            logos_abi::DEBUG_LINE_SYSCALL => Self::DebugLine,
            logos_abi::WALL_TIME_SYSCALL => Self::WallTime,
            logos_abi::SYSTEM_INFO_SYSCALL => Self::SystemInfo,
            _ => return Err(()),
        })
    }
}

/// View over the saved user GPR block of one trapped syscall.
pub(crate) struct SyscallFrame {
    gpr: *mut usize,
}

impl SyscallFrame {
    /// # Safety
    /// `gpr` must point to a live, writable saved-GPR block of at least
    /// `GPR_WORDS` words in `context_common` push order, and stay valid and
    /// unaliased for the life of the frame.
    pub(crate) unsafe fn from_gpr(gpr: usize) -> Self {
        Self { gpr: gpr as *mut usize }
    }

    /// # Safety
    /// `fx_context` must point to the fxsave area built by `context_common`,
    /// whose word `FX_GPR_POINTER_WORD` holds the saved-GPR pointer.
    pub(crate) unsafe fn from_fx_context(fx_context: usize) -> Self {
        // SAFETY: the caller guarantees the fxsave area and its pointer word.
        let gpr = unsafe {
            core::ptr::read_unaligned((fx_context as *const usize).add(FX_GPR_POINTER_WORD))
        };
        // SAFETY: the pointer word names the saved-GPR block per the caller contract.
        unsafe { Self::from_gpr(gpr) }
    }

    fn read(&self, word: usize) -> usize {
        debug_assert!(word < GPR_WORDS);
        // SAFETY: `from_gpr` guarantees `GPR_WORDS` readable words; `word` is
        // one of the `slot` constants, all below `GPR_WORDS`.
        unsafe { core::ptr::read_unaligned(self.gpr.add(word)) }
    }

    pub(crate) fn raw_number(&self) -> usize {
        self.read(slot::RAX)
    }

    pub(crate) fn number(&self) -> Result<Syscall, ()> {
        Syscall::try_from(self.raw_number())
    }

    /// Argument `n` (0 = rdi, 1 = rsi). Panics on `n >= 2`.
    pub(crate) fn arg(&self, n: usize) -> usize {
        self.read(ARG_SLOTS[n])
    }

    pub(crate) fn set_return(&mut self, value: usize) {
        // SAFETY: `from_gpr` guarantees `GPR_WORDS` writable words and
        // `slot::RAX` is below `GPR_WORDS`.
        unsafe { core::ptr::write_unaligned(self.gpr.add(slot::RAX), value) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [(usize, Syscall); 15] = [
        (1, Syscall::Yield),
        (4, Syscall::IpcSend),
        (5, Syscall::IpcReceive),
        (10, Syscall::Heartbeat),
        (11, Syscall::Power),
        (12, Syscall::Manager),
        (14, Syscall::Exit),
        (15, Syscall::HeapGrow),
        (16, Syscall::HeapShrink),
        (17, Syscall::Directory),
        (18, Syscall::Event),
        (19, Syscall::CurrentTicks),
        (20, Syscall::DebugLine),
        (21, Syscall::WallTime),
        (22, Syscall::SystemInfo),
    ];

    #[test]
    fn every_known_number_decodes() {
        for (number, syscall) in ALL {
            assert_eq!(Syscall::try_from(number), Ok(syscall), "number {number}");
        }
    }

    #[test]
    fn unknown_numbers_reject() {
        let known: std::vec::Vec<usize> = ALL.iter().map(|(n, _)| *n).collect();
        for number in 0..64usize {
            if !known.contains(&number) {
                assert!(Syscall::try_from(number).is_err(), "number {number}");
            }
        }
        for number in [0x1_0000, 0x8000_0000, u32::MAX as usize, usize::MAX, usize::MAX - 1] {
            assert!(Syscall::try_from(number).is_err());
        }
        // A number that only matches in the low byte must not alias.
        assert!(Syscall::try_from(0x100 | 1).is_err());
    }

    #[test]
    fn frame_layout_matches_push_order() {
        // r15 lowest ... rax highest; rdi is word 8, rsi 9, rax 14.
        assert_eq!(slot::RDI, 8);
        assert_eq!(slot::RSI, 9);
        assert_eq!(slot::RAX, GPR_WORDS - 1);
    }

    #[test]
    fn args_and_number_map_to_registers() {
        let mut words = [0usize; GPR_WORDS];
        words[slot::RAX] = 18;
        words[slot::RDI] = 0xaaaa;
        words[slot::RSI] = 0xbbbb;
        words[0] = 0xdead; // r15 must never leak into an argument
        // SAFETY: `words` is a live writable block of GPR_WORDS words.
        let frame = unsafe { SyscallFrame::from_gpr(words.as_mut_ptr() as usize) };
        assert_eq!(frame.raw_number(), 18);
        assert_eq!(frame.number(), Ok(Syscall::Event));
        assert_eq!(frame.arg(0), 0xaaaa);
        assert_eq!(frame.arg(1), 0xbbbb);
    }

    #[test]
    fn unknown_number_through_frame_rejects() {
        let mut words = [0usize; GPR_WORDS];
        words[slot::RAX] = usize::MAX;
        // SAFETY: as above.
        let frame = unsafe { SyscallFrame::from_gpr(words.as_mut_ptr() as usize) };
        assert!(frame.number().is_err());
    }

    #[test]
    fn return_lands_in_rax_only() {
        let mut words = [0x1111usize; GPR_WORDS];
        // SAFETY: as above.
        let mut frame = unsafe { SyscallFrame::from_gpr(words.as_mut_ptr() as usize) };
        frame.set_return(0x42);
        for (index, word) in words.iter().enumerate() {
            let expected = if index == slot::RAX { 0x42 } else { 0x1111 };
            assert_eq!(*word, expected, "word {index}");
        }
    }

    #[test]
    fn fx_context_pointer_resolves_gpr_block() {
        let mut words = [0usize; GPR_WORDS];
        words[slot::RAX] = 19;
        let mut fx = [0usize; FX_GPR_POINTER_WORD + 1];
        fx[FX_GPR_POINTER_WORD] = words.as_mut_ptr() as usize;
        // SAFETY: `fx` carries a valid pointer word to `words`.
        let mut frame = unsafe { SyscallFrame::from_fx_context(fx.as_ptr() as usize) };
        assert_eq!(frame.number(), Ok(Syscall::CurrentTicks));
        frame.set_return(7);
        assert_eq!(words[slot::RAX], 7);
    }
}
