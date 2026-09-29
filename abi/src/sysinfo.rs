//! Bounded system-info wire shape (ADR-0093).
//!
//! `SystemInfo` is the fixed, read-only record the kernel hands to services
//! through `SYSTEM_INFO_SYSCALL`: online CPU count and physical memory in
//! whole MiB. It is packed into the single `u64` the bounded syscall ABI
//! returns in `rax`, so no user pointer is involved.

/// Largest value each memory field can carry (28 bits of MiB, 256 TiB).
pub const SYSTEM_INFO_MAX_MIB: u32 = (1 << 28) - 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct SystemInfo {
    pub cpus: u8,
    pub mem_total_mib: u32,
    pub mem_used_mib: u32,
}

impl SystemInfo {
    /// Pack as `cpus:8 | total_mib:28 | used_mib:28`, clamping each memory
    /// field to [`SYSTEM_INFO_MAX_MIB`].
    pub fn pack(self) -> u64 {
        (u64::from(self.cpus) << 56)
            | (u64::from(self.mem_total_mib.min(SYSTEM_INFO_MAX_MIB)) << 28)
            | u64::from(self.mem_used_mib.min(SYSTEM_INFO_MAX_MIB))
    }

    /// Inverse of [`SystemInfo::pack`].
    pub fn unpack(raw: u64) -> Self {
        SystemInfo {
            cpus: (raw >> 56) as u8,
            mem_total_mib: ((raw >> 28) & u64::from(SYSTEM_INFO_MAX_MIB)) as u32,
            mem_used_mib: (raw & u64::from(SYSTEM_INFO_MAX_MIB)) as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_round_trips_and_clamps() {
        let info = SystemInfo { cpus: 4, mem_total_mib: 512, mem_used_mib: 38 };
        assert_eq!(SystemInfo::unpack(info.pack()), info);
        let max = SystemInfo { cpus: 255, mem_total_mib: SYSTEM_INFO_MAX_MIB, mem_used_mib: 0 };
        assert_eq!(SystemInfo::unpack(max.pack()), max);
        let over = SystemInfo { cpus: 1, mem_total_mib: u32::MAX, mem_used_mib: u32::MAX };
        let back = SystemInfo::unpack(over.pack());
        assert_eq!(
            (back.mem_total_mib, back.mem_used_mib),
            (SYSTEM_INFO_MAX_MIB, SYSTEM_INFO_MAX_MIB)
        );
        assert_eq!(back.cpus, 1);
    }
}
