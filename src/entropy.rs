use uefi::{boot, proto::rng::Rng};

#[derive(Clone, Copy)]
pub struct Seed([u8; 32]);

impl Seed {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

pub fn load() -> Option<Seed> {
    let handle = boot::get_handle_for_protocol::<Rng>().ok()?;
    let mut rng = boot::open_protocol_exclusive::<Rng>(handle).ok()?;
    let mut bytes = [0; 32];
    rng.get_rng(None, &mut bytes).ok()?;
    Some(Seed(bytes))
}

pub fn announce(seed: Option<Seed>) {
    crate::debug::write_line(if seed.is_some() {
        b"LogOS: entropy firmware"
    } else {
        b"LogOS: entropy unavailable"
    });
}

pub fn self_check() -> bool {
    Seed::from_bytes([1; 32]).bytes() == &[1; 32]
}
