pub const NAME: &[u8] = b"virtio-block";
pub const SERVICE: crate::platform::services::Service =
    crate::platform::services::Service::VirtioBlock;

const BLOCK: crate::device::DriverManifest = crate::device::DriverManifest {
    interface: crate::device::Interface::new(crate::device::Class::Block),
    vendor_id: 0x1af4,
    device_id: 0x1001,
    capabilities: &[
        logos_core::capabilities::CapabilityKind::Block,
        logos_core::capabilities::CapabilityKind::Memory,
    ],
};

pub fn discover(devices: &crate::pci::PciDevices) -> Option<crate::pci::PciDevice> {
    crate::device::bind(&[BLOCK], BLOCK.vendor_id, BLOCK.device_id)
        .and_then(|manifest| devices.find(manifest.vendor_id, manifest.device_id))
}
