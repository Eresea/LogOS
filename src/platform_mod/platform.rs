pub use crate::drivers::virtio::{
    ServiceTask as Task, VirtioService as Service, completion_pending, interrupt,
};

pub const NAME: &[u8] = crate::drivers::supervisor::VIRTIO_BALLOON;
pub const SERVICE: crate::platform_mod::services::Service =
    crate::platform_mod::services::Service::VirtioBalloon;

pub fn matches(name: &[u8]) -> bool {
    name == NAME
}

const BALLOON: crate::device::DriverManifest = crate::device::DriverManifest {
    interface: crate::device::Interface::new(crate::device::Class::Memory),
    vendor_id: 0x1af4,
    device_id: 0x1002,
    capabilities: &[logos_core::capabilities::CapabilityKind::Service],
};

pub fn discover(devices: &crate::pci::PciDevices) -> Option<crate::pci::PciDevice> {
    crate::device::bind(&[BALLOON], BALLOON.vendor_id, BALLOON.device_id)
        .and_then(|manifest| devices.find(manifest.vendor_id, manifest.device_id))
}
