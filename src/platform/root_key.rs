use uefi::{
    cstr16,
    runtime::{self, VariableAttributes, VariableVendor},
};

const NAME: &uefi::CStr16 = cstr16!("LogOSSecretRoot");

pub struct RootKey([u8; 32]);

impl RootKey {
    pub const fn bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn wipe(&mut self) {
        self.0.fill(0);
    }

    pub fn is_wiped(&self) -> bool {
        self.0 == [0; 32]
    }
}

pub fn announce(key: Option<&RootKey>) {
    crate::debug::write_line(if key.is_some() {
        b"LogOS: secret root firmware"
    } else {
        b"LogOS: durable secrets unavailable"
    });
}

pub fn load(entropy: Option<&crate::platform::entropy::Seed>) -> Option<RootKey> {
    let mut bytes = [0; 32];
    if let Ok((stored, _)) =
        runtime::get_variable(NAME, &VariableVendor::GLOBAL_VARIABLE, &mut bytes)
        && stored.len() == bytes.len()
    {
        return Some(RootKey(bytes));
    }
    bytes.copy_from_slice(entropy?.bytes());
    let attributes = VariableAttributes::NON_VOLATILE | VariableAttributes::BOOTSERVICE_ACCESS;
    runtime::set_variable(NAME, &VariableVendor::GLOBAL_VARIABLE, attributes, &bytes)
        .ok()
        .map(|()| RootKey(bytes))
}
