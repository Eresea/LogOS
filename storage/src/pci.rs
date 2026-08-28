pub const VIRTIO_PCI_VENDOR_ID: u16 = 0x1af4;
pub const VIRTIO_BLOCK_MODERN_DEVICE_ID: u16 = 0x1042;
pub const VIRTIO_GPU_MODERN_DEVICE_ID: u16 = 0x1050;
pub const PCI_CONFIG_BYTES: usize = 256;
const PCI_CAP_PTR: usize = 0x34;
const PCI_CAP_VENDOR_SPECIFIC: u8 = 0x09;
const VIRTIO_PCI_CAP_PCI_CFG: usize = 5;
const MAX_CAPABILITIES: usize = 48;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PciAddress {
    bus: u8,
    device: u8,
    function: u8,
}

impl PciAddress {
    pub const fn new(bus: u8, device: u8, function: u8) -> Option<Self> {
        if device >= 32 || function >= 8 { None } else { Some(Self { bus, device, function }) }
    }

    pub const fn bus(self) -> u8 {
        self.bus
    }

    pub const fn device(self) -> u8 {
        self.device
    }

    pub const fn function(self) -> u8 {
        self.function
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
    NotVirtioBlock,
    NotRequestedDevice,
    MissingCapability,
    MalformedCapability,
    CapabilityLoop,
    InvalidAddress,
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
        Self::from_config_for_device(address, config, VIRTIO_BLOCK_MODERN_DEVICE_ID).map_err(
            |error| {
                if error == PciError::NotRequestedDevice { PciError::NotVirtioBlock } else { error }
            },
        )
    }

    pub fn from_config_for_device(
        address: PciAddress,
        config: &[u8; PCI_CONFIG_BYTES],
        device_id: u16,
    ) -> Result<Self, PciError> {
        let vendor = u16::from_le_bytes([config[0], config[1]]);
        let device = u16::from_le_bytes([config[2], config[3]]);
        if vendor != VIRTIO_PCI_VENDOR_ID || device != device_id {
            return Err(PciError::NotRequestedDevice);
        }

        let mut cursor = config[PCI_CAP_PTR] as usize;
        let mut seen = [false; PCI_CONFIG_BYTES];
        let mut capabilities = [None; 5];
        let mut notify_multiplier = 0;
        for _ in 0..MAX_CAPABILITIES {
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
            let offset = u32::from_le_bytes([
                config[cursor + 8],
                config[cursor + 9],
                config[cursor + 10],
                config[cursor + 11],
            ]);
            let length = u32::from_le_bytes([
                config[cursor + 12],
                config[cursor + 13],
                config[cursor + 14],
                config[cursor + 15],
            ]);
            if bar >= 6 || length == 0 || offset.checked_add(length).is_none() {
                return Err(PciError::MalformedCapability);
            }
            if cfg_type == 2 && length >= 20 {
                notify_multiplier = u32::from_le_bytes([
                    config[cursor + 16],
                    config[cursor + 17],
                    config[cursor + 18],
                    config[cursor + 19],
                ]);
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
        config[0..2].copy_from_slice(&VIRTIO_PCI_VENDOR_ID.to_le_bytes());
        config[2..4].copy_from_slice(&VIRTIO_BLOCK_MODERN_DEVICE_ID.to_le_bytes());
        config[PCI_CAP_PTR] = 0x40;
        for (index, cfg_type) in [(0x40, 1u8), (0x60, 2), (0x80, 3), (0xa0, 4)] {
            config[index] = PCI_CAP_VENDOR_SPECIFIC;
            config[index + 1] = if index == 0xa0 { 0 } else { (index + 0x20) as u8 };
            config[index + 2] = 16;
            config[index + 3] = cfg_type;
            config[index + 4] = 0;
            config[index + 8..index + 12].copy_from_slice(&(index as u32 * 0x100).to_le_bytes());
            config[index + 12..index + 16].copy_from_slice(&0x100u32.to_le_bytes());
            if cfg_type == 2 {
                config[index + 2] = 20;
                config[index + 16..index + 20].copy_from_slice(&4u32.to_le_bytes());
            }
        }
        config
    }

    #[test]
    fn parses_modern_virtio_block_capabilities() {
        let address = PciAddress::new(0, 3, 0).unwrap();
        let device = VirtioPciDevice::from_config(address, &config()).unwrap();
        assert_eq!(device.address, address);
        assert_eq!(device.capabilities.common.offset, 0x4000);
        assert_eq!(device.capabilities.device.offset, 0xa000);
        assert_eq!(device.capabilities.notify_multiplier, 4);
    }

    #[test]
    fn rejects_non_virtio_devices_and_missing_capabilities() {
        let address = PciAddress::new(0, 3, 0).unwrap();
        let mut wrong = config();
        wrong[0] = 0;
        assert_eq!(VirtioPciDevice::from_config(address, &wrong), Err(PciError::NotVirtioBlock));

        let mut missing = config();
        missing[0xa0] = 0;
        missing[0x80 + 1] = 0;
        assert_eq!(
            VirtioPciDevice::from_config(address, &missing),
            Err(PciError::MissingCapability)
        );
    }

    #[test]
    fn rejects_capability_loops() {
        let address = PciAddress::new(0, 3, 0).unwrap();
        let mut looped = config();
        looped[0xa0 + 1] = 0x40;
        assert_eq!(VirtioPciDevice::from_config(address, &looped), Err(PciError::CapabilityLoop));
    }

    #[test]
    fn rejects_invalid_capability_bar_and_range() {
        let address = PciAddress::new(0, 3, 0).unwrap();
        let mut invalid_bar = config();
        invalid_bar[0x40 + 4] = 6;
        assert_eq!(
            VirtioPciDevice::from_config(address, &invalid_bar),
            Err(PciError::MalformedCapability)
        );

        let mut invalid_range = config();
        invalid_range[0x40 + 8..0x40 + 12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            VirtioPciDevice::from_config(address, &invalid_range),
            Err(PciError::MalformedCapability)
        );
    }

    #[test]
    fn accepts_optional_pci_cfg_capability() {
        let address = PciAddress::new(0, 3, 0).unwrap();
        let mut config = config();
        config[0xa0 + 1] = 0xc0;
        config[0xc0] = PCI_CAP_VENDOR_SPECIFIC;
        config[0xc0 + 2] = 16;
        config[0xc0 + 3] = VIRTIO_PCI_CAP_PCI_CFG as u8;
        assert!(VirtioPciDevice::from_config(address, &config).is_ok());
    }

    #[test]
    fn parses_the_same_capabilities_for_a_requested_gpu_device() {
        let address = PciAddress::new(0, 3, 0).unwrap();
        let mut config = config();
        config[2..4].copy_from_slice(&VIRTIO_GPU_MODERN_DEVICE_ID.to_le_bytes());
        assert_eq!(
            VirtioPciDevice::from_config_for_device(address, &config, VIRTIO_GPU_MODERN_DEVICE_ID,)
                .unwrap()
                .capabilities
                .notify_multiplier,
            4
        );
    }
}
