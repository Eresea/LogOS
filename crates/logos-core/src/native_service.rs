pub const MAGIC: [u8; 4] = *b"LGSV";
pub const ABI: u16 = 1;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Header {
    pub magic: [u8; 4],
    pub abi: u16,
    pub reserved: u16,
    pub name: [u8; 16],
}

impl Header {
    pub const fn new(name: [u8; 16]) -> Self {
        Self { magic: MAGIC, abi: ABI, reserved: 0, name }
    }

    pub fn valid_for(&self, name: &[u8]) -> bool {
        self.magic == MAGIC && self.abi == ABI && self.reserved == 0 && self.name_starts_with(name)
    }

    fn name_starts_with(&self, name: &[u8]) -> bool {
        if name.len() > self.name.len() {
            return false;
        }
        let mut index = 0;
        while index < name.len() {
            if self.name[index] != name[index] {
                return false;
            }
            index += 1;
        }
        index == self.name.len() || self.name[index] == 0
    }
}

pub fn self_check() -> bool {
    Header::new(*b"terminal\0\0\0\0\0\0\0\0").valid_for(b"terminal")
        && !Header::new(*b"terminal\0\0\0\0\0\0\0\0").valid_for(b"other")
}
