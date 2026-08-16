//! Fixed manifest for the ring-3 service images.

use logos_abi::{MAX_SERVICE_IMAGE_BYTES, MAX_SERVICE_NAME_BYTES, ServiceId};

use crate::process::{ElfLoadPlan, ProcessError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceImageSpec {
    service: ServiceId,
    name: &'static [u8],
    path: &'static [u8],
    dependencies: u8,
}

impl ServiceImageSpec {
    const fn new(
        service: ServiceId,
        name: &'static [u8],
        path: &'static [u8],
        dependencies: u8,
    ) -> Self {
        Self { service, name, path, dependencies }
    }

    pub const fn service(self) -> ServiceId {
        self.service
    }

    pub const fn name(self) -> &'static [u8] {
        self.name
    }

    pub const fn path(self) -> &'static [u8] {
        self.path
    }

    pub const fn dependencies(self) -> u8 {
        self.dependencies
    }

    pub fn validate_image(self, image: &[u8]) -> Result<ElfLoadPlan, ServiceImageError> {
        validate_image_bytes(image.len())?;
        ElfLoadPlan::parse(image).map_err(ServiceImageError::InvalidElf)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceImageError {
    Empty,
    TooLarge,
    InvalidElf(ProcessError),
}

pub const SERVICE_IMAGES: [ServiceImageSpec; 6] = [
    ServiceImageSpec::new(ServiceId::Input, b"input", b"\\EFI\\LOGOS\\INPUT.ELF", 0),
    ServiceImageSpec::new(ServiceId::Display, b"display", b"\\EFI\\LOGOS\\DISPLAY.ELF", 0),
    ServiceImageSpec::new(
        ServiceId::Terminal,
        b"terminal",
        b"\\EFI\\LOGOS\\TERMINAL.ELF",
        (1 << ServiceId::Input.index()) | (1 << ServiceId::Display.index()),
    ),
    ServiceImageSpec::new(
        ServiceId::Session,
        b"session",
        b"\\EFI\\LOGOS\\SESSION.ELF",
        1 << ServiceId::Terminal.index(),
    ),
    ServiceImageSpec::new(
        ServiceId::Commands,
        b"commands",
        b"\\EFI\\LOGOS\\COMMANDS.ELF",
        1 << ServiceId::Session.index(),
    ),
    ServiceImageSpec::new(
        ServiceId::Storage,
        b"storage",
        b"\\EFI\\LOGOS\\STORAGE.ELF",
        1 << ServiceId::Commands.index(),
    ),
];

pub const fn service_image(service: ServiceId) -> ServiceImageSpec {
    SERVICE_IMAGES[service.index()]
}

const fn manifest_is_indexed() -> bool {
    let mut index = 0;
    while index < SERVICE_IMAGES.len() {
        let spec = SERVICE_IMAGES[index];
        if spec.service().index() != index
            || spec.name().is_empty()
            || spec.name().len() > MAX_SERVICE_NAME_BYTES
            || spec.dependencies() & !((1 << SERVICE_IMAGES.len()) - 1) != 0
        {
            return false;
        }
        index += 1;
    }
    true
}

const _: () = assert!(manifest_is_indexed());

fn validate_image_bytes(bytes: usize) -> Result<(), ServiceImageError> {
    if bytes == 0 {
        Err(ServiceImageError::Empty)
    } else if bytes > MAX_SERVICE_IMAGE_BYTES {
        Err(ServiceImageError::TooLarge)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> [u8; 128] {
        let mut image = [0; 128];
        image[..4].copy_from_slice(b"\x7fELF");
        image[4] = 2;
        image[5] = 1;
        image[16..18].copy_from_slice(&2u16.to_le_bytes());
        image[18..20].copy_from_slice(&0x3eu16.to_le_bytes());
        image[24..32].copy_from_slice(&0x1000u64.to_le_bytes());
        image[32..40].copy_from_slice(&64u64.to_le_bytes());
        image[54..56].copy_from_slice(&56u16.to_le_bytes());
        image[56..58].copy_from_slice(&1u16.to_le_bytes());
        image[64..68].copy_from_slice(&1u32.to_le_bytes());
        image[68..72].copy_from_slice(&5u32.to_le_bytes());
        image[80..88].copy_from_slice(&0x1000u64.to_le_bytes());
        image[96..104].copy_from_slice(&1u64.to_le_bytes());
        image[104..112].copy_from_slice(&0x1000u64.to_le_bytes());
        image
    }

    #[test]
    fn manifest_is_fixed_and_dependency_ordered() {
        assert_eq!(SERVICE_IMAGES.len(), 6);
        assert_eq!(SERVICE_IMAGES[0].service(), ServiceId::Input);
        assert_eq!(SERVICE_IMAGES[1].service(), ServiceId::Display);
        assert_eq!(SERVICE_IMAGES[2].service(), ServiceId::Terminal);
        assert_eq!(SERVICE_IMAGES[3].service(), ServiceId::Session);
        assert_eq!(SERVICE_IMAGES[4].service(), ServiceId::Commands);
        assert_eq!(SERVICE_IMAGES[5].service(), ServiceId::Storage);
        assert_eq!(SERVICE_IMAGES[2].dependencies(), 0b0000_0011);
        assert_eq!(SERVICE_IMAGES[5].name(), b"storage");
        assert_eq!(service_image(ServiceId::Display).path(), b"\\EFI\\LOGOS\\DISPLAY.ELF");
    }

    #[test]
    fn image_validation_rejects_empty_oversize_and_malformed_inputs() {
        let spec = service_image(ServiceId::Input);
        assert_eq!(spec.validate_image(&[]), Err(ServiceImageError::Empty));
        assert_eq!(
            validate_image_bytes(MAX_SERVICE_IMAGE_BYTES + 1),
            Err(ServiceImageError::TooLarge)
        );
        assert!(matches!(spec.validate_image(&[0; 64]), Err(ServiceImageError::InvalidElf(_))));
        assert_eq!(spec.validate_image(&image()).unwrap().entry(), 0x1000);
    }
}
