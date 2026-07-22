use core::arch::asm;

const DEVICES: usize = 8;

#[derive(Clone, Copy)]
pub struct PciDevice {
    bus: u8,
    device: u8,
    function: u8,
    vendor_id: u16,
    device_id: u16,
}

pub struct PciDevices {
    devices: [Option<PciDevice>; DEVICES],
    len: usize,
}

impl PciDevices {
    pub const fn len(&self) -> usize {
        self.len
    }

    pub fn first(&self) -> Option<PciDevice> {
        self.devices[0]
    }

    pub fn find(&self, vendor_id: u16, device_id: u16) -> Option<PciDevice> {
        self.devices[..self.len]
            .iter()
            .flatten()
            .copied()
            .find(|device| device.vendor_id == vendor_id && device.device_id == device_id)
    }

    fn push(&mut self, device: PciDevice) {
        if self.len < DEVICES {
            // ponytail: retain eight devices; add dynamic storage when drivers need more.
            self.devices[self.len] = Some(device);
            self.len += 1;
        }
    }
}

impl PciDevice {
    pub const fn location(self) -> (u8, u8, u8) {
        (self.bus, self.device, self.function)
    }

    pub const fn vendor_id(self) -> u16 {
        self.vendor_id
    }

    pub const fn device_id(self) -> u16 {
        self.device_id
    }

    pub fn bar(self, index: u8) -> u32 {
        config_read(self.bus, self.device, self.function, 0x10 + index * 4)
    }
}

pub fn scan() -> PciDevices {
    let mut devices = PciDevices { devices: [None; DEVICES], len: 0 };
    for bus in 0..=u8::MAX {
        for device in 0..32 {
            let Some(first) = probe(bus, device, 0) else {
                continue;
            };
            let header = config_read(bus, device, 0, 0x0c);
            devices.push(first);
            if header & (1 << 23) != 0 {
                for function in 1..8 {
                    if let Some(device) = probe(bus, device, function) {
                        devices.push(device);
                    }
                }
            }
        }
    }
    devices
}

fn probe(bus: u8, device: u8, function: u8) -> Option<PciDevice> {
    let id = config_read(bus, device, function, 0);
    let vendor_id = id as u16;
    (vendor_id != u16::MAX).then_some(PciDevice {
        bus,
        device,
        function,
        vendor_id,
        device_id: (id >> 16) as u16,
    })
}

fn config_read(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    let address = 0x8000_0000
        | (u32::from(bus) << 16)
        | (u32::from(device) << 11)
        | (u32::from(function) << 8)
        | u32::from(offset & 0xfc);
    unsafe {
        asm!("out dx, eax", in("dx") 0xcf8u16, in("eax") address);
        let value: u32;
        asm!("in eax, dx", in("dx") 0xcfcu16, out("eax") value);
        value
    }
}
