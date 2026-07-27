#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Input,
    Display,
    Block,
    Network,
    Entropy,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Interface {
    class: Class,
    version: u16,
}

#[derive(Clone, Copy)]
pub struct DriverManifest {
    pub interface: Interface,
    pub vendor_id: u16,
    pub device_id: u16,
    pub capabilities: &'static [CapabilityKind],
}

pub fn bind(
    manifests: &[DriverManifest],
    vendor_id: u16,
    device_id: u16,
) -> Option<DriverManifest> {
    manifests
        .iter()
        .copied()
        .find(|manifest| manifest.vendor_id == vendor_id && manifest.device_id == device_id)
}

impl Interface {
    pub const fn new(class: Class) -> Self {
        Self { class, version: 1 }
    }

    pub const fn class(self) -> Class {
        self.class
    }

    pub fn compatible(self, other: Self) -> bool {
        self.class == other.class && self.version == other.version
    }
}

pub fn self_check() -> bool {
    let input = Interface::new(Class::Input);
    let display = Interface::new(Class::Display);
    let block = Interface::new(Class::Block);
    let network = Interface::new(Class::Network);
    let entropy = Interface::new(Class::Entropy);
    input.compatible(input)
        && !input.compatible(display)
        && block.class() == Class::Block
        && network.class() == Class::Network
        && entropy.class() == Class::Entropy
        && bind(
            &[DriverManifest {
                interface: Interface::new(Class::Block),
                vendor_id: 1,
                device_id: 2,
                capabilities: &[CapabilityKind::Service],
            }],
            1,
            2,
        )
        .is_some_and(|manifest| {
            manifest.interface == block && manifest.capabilities == [CapabilityKind::Service]
        })
}
use crate::capabilities::CapabilityKind;
