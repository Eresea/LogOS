//! Modern VirtIO-net PCI capability parsing, independent of MMIO access.

pub const VIRTIO_PCI_VENDOR_ID: u16 = 0x1af4;
pub const VIRTIO_NETWORK_MODERN_DEVICE_ID: u16 = 0x1041;
pub const PCI_CONFIG_BYTES: usize = 256;

const PCI_CAP_PTR: usize = 0x34;
const PCI_CAP_VENDOR_SPECIFIC: u8 = 0x09;
const VIRTIO_PCI_CAP_PCI_CFG: usize = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PciAddress {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

impl PciAddress {
    pub const fn new(bus: u8, device: u8, function: u8) -> Option<Self> {
        if device >= 32 || function >= 8 { None } else { Some(Self { bus, device, function }) }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtioPciCapability {
    pub bar: u8,
    pub offset: u32,
    pub length: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtioPciCapabilities {
    pub common: VirtioPciCapability,
    pub notify: VirtioPciCapability,
    pub notify_multiplier: u32,
    pub isr: VirtioPciCapability,
    pub device: VirtioPciCapability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PciError {
    NotVirtioNetwork,
    MissingCapability,
    MalformedCapability,
    CapabilityLoop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtioPciDevice {
    pub address: PciAddress,
    pub capabilities: VirtioPciCapabilities,
}

impl VirtioPciDevice {
    pub fn from_config(
        address: PciAddress,
        config: &[u8; PCI_CONFIG_BYTES],
    ) -> Result<Self, PciError> {
        let vendor = u16::from_le_bytes([config[0], config[1]]);
        let device = u16::from_le_bytes([config[2], config[3]]);
        if vendor != VIRTIO_PCI_VENDOR_ID || device != VIRTIO_NETWORK_MODERN_DEVICE_ID {
            return Err(PciError::NotVirtioNetwork);
        }
        let mut cursor = config[PCI_CAP_PTR] as usize;
        let mut seen = [false; PCI_CONFIG_BYTES];
        let mut capabilities = [None; 5];
        let mut notify_multiplier = 0;
        for _ in 0..48 {
            if cursor == 0 {
                break;
            }
            if cursor + 3 >= PCI_CONFIG_BYTES || seen[cursor] {
                return Err(PciError::CapabilityLoop);
            }
            seen[cursor] = true;
            if config[cursor] != PCI_CAP_VENDOR_SPECIFIC {
                cursor = config[cursor + 1] as usize;
                continue;
            }
            let next = config[cursor + 1] as usize;
            let length = config[cursor + 2] as usize;
            if length < 16 || cursor + length > PCI_CONFIG_BYTES {
                return Err(PciError::MalformedCapability);
            }
            let cfg_type = config[cursor + 3] as usize;
            if cfg_type == VIRTIO_PCI_CAP_PCI_CFG {
                cursor = next;
                continue;
            }
            if cfg_type >= capabilities.len() {
                return Err(PciError::MalformedCapability);
            }
            let bar = config[cursor + 4];
            let offset = u32::from_le_bytes(config[cursor + 8..cursor + 12].try_into().unwrap());
            let length = u32::from_le_bytes(config[cursor + 12..cursor + 16].try_into().unwrap());
            if bar >= 6 || length == 0 || offset.checked_add(length).is_none() {
                return Err(PciError::MalformedCapability);
            }
            if cfg_type == 2 && cursor + 20 <= PCI_CONFIG_BYTES {
                notify_multiplier =
                    u32::from_le_bytes(config[cursor + 16..cursor + 20].try_into().unwrap());
            }
            capabilities[cfg_type] = Some(VirtioPciCapability { bar, offset, length });
            cursor = next;
        }
        if cursor != 0 {
            return Err(PciError::CapabilityLoop);
        }
        Ok(Self {
            address,
            capabilities: VirtioPciCapabilities {
                common: capabilities[1].ok_or(PciError::MissingCapability)?,
                notify: capabilities[2].ok_or(PciError::MissingCapability)?,
                notify_multiplier,
                isr: capabilities[3].ok_or(PciError::MissingCapability)?,
                device: capabilities[4].ok_or(PciError::MissingCapability)?,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> [u8; PCI_CONFIG_BYTES] {
        let mut config = [0; PCI_CONFIG_BYTES];
        config[..2].copy_from_slice(&VIRTIO_PCI_VENDOR_ID.to_le_bytes());
        config[2..4].copy_from_slice(&VIRTIO_NETWORK_MODERN_DEVICE_ID.to_le_bytes());
        config[PCI_CAP_PTR] = 0x40;
        for (offset, kind) in [(0x40, 1u8), (0x60, 2), (0x80, 3), (0xa0, 4)] {
            config[offset] = PCI_CAP_VENDOR_SPECIFIC;
            config[offset + 1] = if offset == 0xa0 { 0 } else { (offset + 0x20) as u8 };
            config[offset + 2] = if kind == 2 { 20 } else { 16 };
            config[offset + 3] = kind;
            config[offset + 8..offset + 12].copy_from_slice(&(offset as u32 * 0x100).to_le_bytes());
            config[offset + 12..offset + 16].copy_from_slice(&0x100u32.to_le_bytes());
            if kind == 2 {
                config[offset + 16..offset + 20].copy_from_slice(&4u32.to_le_bytes());
            }
        }
        config
    }

    #[test]
    fn parses_modern_network_capabilities() {
        let address = PciAddress::new(0, 3, 0).unwrap();
        let device = VirtioPciDevice::from_config(address, &config()).unwrap();
        assert_eq!(device.address, address);
        assert_eq!(device.capabilities.notify_multiplier, 4);
    }

    #[test]
    fn rejects_wrong_device_and_capability_loops() {
        let address = PciAddress::new(0, 3, 0).unwrap();
        let mut wrong = config();
        wrong[2..4].copy_from_slice(&0x1042u16.to_le_bytes());
        assert_eq!(VirtioPciDevice::from_config(address, &wrong), Err(PciError::NotVirtioNetwork));
        let mut looped = config();
        looped[0x60 + 1] = 0x40;
        assert_eq!(VirtioPciDevice::from_config(address, &looped), Err(PciError::CapabilityLoop));
    }
}
