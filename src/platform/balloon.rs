pub const NAME: &[u8] = crate::drivers::supervisor::VIRTIO_BALLOON;
pub const SERVICE: crate::platform::services::Service =
    crate::platform::services::Service::VirtioBalloon;

pub fn matches(name: &[u8]) -> bool {
    name == NAME
}

const BALLOON: crate::drivers::device::DriverManifest = crate::drivers::device::DriverManifest {
    interface: crate::drivers::device::Interface::new(crate::drivers::device::Class::Memory),
    vendor_id: 0x1af4,
    device_id: 0x1002,
    capabilities: &[logos_core::capabilities::CapabilityKind::Service],
};

pub fn discover(devices: &crate::arch::pci::PciDevices) -> Option<crate::arch::pci::PciDevice> {
    crate::drivers::device::bind(&[BALLOON], BALLOON.vendor_id, BALLOON.device_id)
        .and_then(|manifest| devices.find(manifest.vendor_id, manifest.device_id))
}
