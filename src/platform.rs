#[path = "virtio.rs"]
mod driver;

pub use driver::{ServiceTask as Task, VirtioService as Service, completion_pending, interrupt};

const BALLOON: crate::device::DriverManifest = crate::device::DriverManifest {
    interface: crate::device::Interface::new(crate::device::Class::Memory),
    vendor_id: 0x1af4,
    device_id: 0x1002,
    capabilities: &[crate::capabilities::CapabilityKind::Service],
};

pub fn discover(devices: &crate::pci::PciDevices) -> Option<crate::pci::PciDevice> {
    crate::device::bind(&[BALLOON], BALLOON.vendor_id, BALLOON.device_id)
        .and_then(|manifest| devices.find(manifest.vendor_id, manifest.device_id))
}
