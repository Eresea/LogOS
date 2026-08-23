//! Fixed manifest for the ring-3 service images.

use logos_abi::{MAX_SERVICE_IMAGE_BYTES, MAX_SERVICE_NAME_BYTES, ServiceId};

use crate::process::{ElfLoadPlan, ProcessError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceImageSpec {
    service: ServiceId,
    name: &'static [u8],
    path: &'static [u8],
    dependencies: &'static [ServiceId],
}

impl ServiceImageSpec {
    const fn new(
        service: ServiceId,
        name: &'static [u8],
        path: &'static [u8],
        dependencies: &'static [ServiceId],
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

    pub const fn dependencies(self) -> &'static [ServiceId] {
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

pub const SERVICE_IMAGES: [ServiceImageSpec; 10] = [
    ServiceImageSpec::new(ServiceId::Input, b"input", b"\\EFI\\LOGOS\\INPUT.ELF", &[]),
    ServiceImageSpec::new(ServiceId::Display, b"display", b"\\EFI\\LOGOS\\DISPLAY.ELF", &[]),
    ServiceImageSpec::new(
        ServiceId::Terminal,
        b"terminal",
        b"\\EFI\\LOGOS\\TERMINAL.ELF",
        &[ServiceId::Input, ServiceId::Display],
    ),
    ServiceImageSpec::new(
        ServiceId::Session,
        b"session",
        b"\\EFI\\LOGOS\\SESSION.ELF",
        &[ServiceId::Terminal],
    ),
    ServiceImageSpec::new(
        ServiceId::Flow,
        b"flow",
        b"\\EFI\\LOGOS\\FLOW.ELF",
        &[ServiceId::Session, ServiceId::Storage, ServiceId::Device],
    ),
    ServiceImageSpec::new(ServiceId::Storage, b"storage", b"\\EFI\\LOGOS\\STORAGE.ELF", &[]),
    ServiceImageSpec::new(ServiceId::Network, b"network", b"\\EFI\\LOGOS\\NETWORK.ELF", &[]),
    ServiceImageSpec::new(
        ServiceId::Fetch,
        b"fetch",
        b"\\EFI\\LOGOS\\FETCH.ELF",
        &[ServiceId::Flow, ServiceId::Storage, ServiceId::Network],
    ),
    ServiceImageSpec::new(ServiceId::Device, b"device", b"\\EFI\\LOGOS\\DEVICE.ELF", &[]),
    ServiceImageSpec::new(
        ServiceId::User,
        b"user",
        b"\\EFI\\LOGOS\\USER.ELF",
        &[ServiceId::Storage],
    ),
];

pub const SERVICE_START_ORDER: [ServiceId; 10] = [
    ServiceId::Input,
    ServiceId::Display,
    ServiceId::Terminal,
    ServiceId::Session,
    ServiceId::Storage,
    ServiceId::User,
    ServiceId::Device,
    ServiceId::Flow,
    ServiceId::Network,
    ServiceId::Fetch,
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
    fn manifest_is_fixed_and_dependencies_are_explicit() {
        assert_eq!(SERVICE_IMAGES.len(), 10);
        assert_eq!(SERVICE_IMAGES[0].service(), ServiceId::Input);
        assert_eq!(SERVICE_IMAGES[1].service(), ServiceId::Display);
        assert_eq!(SERVICE_IMAGES[2].service(), ServiceId::Terminal);
        assert_eq!(SERVICE_IMAGES[3].service(), ServiceId::Session);
        assert_eq!(SERVICE_IMAGES[4].service(), ServiceId::Flow);
        assert_eq!(SERVICE_IMAGES[5].service(), ServiceId::Storage);
        assert_eq!(SERVICE_IMAGES[2].dependencies(), &[ServiceId::Input, ServiceId::Display]);
        assert_eq!(
            SERVICE_IMAGES[4].dependencies(),
            &[ServiceId::Session, ServiceId::Storage, ServiceId::Device]
        );
        assert!(SERVICE_IMAGES[5].dependencies().is_empty());
        assert_eq!(SERVICE_IMAGES[5].name(), b"storage");
        assert_eq!(SERVICE_IMAGES[6].service(), ServiceId::Network);
        assert_eq!(SERVICE_IMAGES[7].service(), ServiceId::Fetch);
        assert_eq!(SERVICE_IMAGES[8].service(), ServiceId::Device);
        assert_eq!(SERVICE_IMAGES[9].service(), ServiceId::User);
        assert_eq!(SERVICE_IMAGES[9].dependencies(), &[ServiceId::Storage]);
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
