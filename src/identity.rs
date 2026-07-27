use core::arch::asm;

use uefi::{
    cstr16,
    runtime::{self, VariableAttributes, VariableVendor},
};

const NAME: &uefi::CStr16 = cstr16!("LogOSMachineId");

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MachineId([u8; 16]);

impl MachineId {
    fn from_seed(mut seed: u64) -> Self {
        let mut bytes = [0; 16];
        for byte in &mut bytes {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            *byte = seed as u8;
        }
        Self(bytes)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Firmware,
    Volatile,
}

pub struct Machine {
    id: MachineId,
    source: Source,
}

impl Machine {
    pub const fn id(&self) -> MachineId {
        self.id
    }

    pub const fn source(&self) -> Source {
        self.source
    }
}

pub fn load() -> Machine {
    let mut bytes = [0; 16];
    if let Ok((stored, _)) =
        runtime::get_variable(NAME, &VariableVendor::GLOBAL_VARIABLE, &mut bytes)
        && stored.len() == bytes.len()
    {
        return Machine { id: MachineId(bytes), source: Source::Firmware };
    }
    let id = MachineId::from_seed(timestamp());
    let attributes = VariableAttributes::NON_VOLATILE | VariableAttributes::BOOTSERVICE_ACCESS;
    let source = if runtime::set_variable(NAME, &VariableVendor::GLOBAL_VARIABLE, attributes, &id.0)
        .is_ok()
    {
        Source::Firmware
    } else {
        Source::Volatile
    };
    Machine { id, source }
}

pub fn announce(machine: &Machine) {
    crate::debug::write_line(match machine.source() {
        Source::Firmware => b"LogOS: machine identity firmware",
        Source::Volatile => b"LogOS: machine identity volatile",
    });
}

pub fn self_check() -> bool {
    let one = MachineId::from_seed(1);
    one == MachineId::from_seed(1) && one != MachineId::from_seed(2)
}

fn timestamp() -> u64 {
    let high: u32;
    let low: u32;
    unsafe { asm!("rdtsc", out("edx") high, out("eax") low) };
    (u64::from(high) << 32) | u64::from(low)
}
