//! Fixed manifest for the ring-3 service images.

use logos_abi::{MAX_SERVICE_IMAGE_BYTES, ServiceId};

use crate::process::{Capabilities, ElfLoadPlan, ProcessError, ProcessKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceImageSpec {
    service: ServiceId,
    process_kind: ProcessKind,
    path: &'static [u8],
    capabilities: Capabilities,
}

impl ServiceImageSpec {
    const fn new(
        service: ServiceId,
        process_kind: ProcessKind,
        path: &'static [u8],
        capabilities: Capabilities,
    ) -> Self {
        Self { service, process_kind, path, capabilities }
    }

    pub const fn service(self) -> ServiceId {
        self.service
    }

    pub const fn process_kind(self) -> ProcessKind {
        self.process_kind
    }

    pub const fn path(self) -> &'static [u8] {
        self.path
    }

    pub const fn capabilities(self) -> Capabilities {
        self.capabilities
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

pub const SERVICE_IMAGES: [ServiceImageSpec; 5] = [
    ServiceImageSpec::new(
        ServiceId::Input,
        ProcessKind::Input,
        b"\\EFI\\LOGOS\\INPUT.ELF",
        Capabilities::INPUT,
    ),
    ServiceImageSpec::new(
        ServiceId::Display,
        ProcessKind::Display,
        b"\\EFI\\LOGOS\\DISPLAY.ELF",
        Capabilities::DISPLAY,
    ),
    ServiceImageSpec::new(
        ServiceId::Terminal,
        ProcessKind::Terminal,
        b"\\EFI\\LOGOS\\TERMINAL.ELF",
        Capabilities::TERMINAL,
    ),
    ServiceImageSpec::new(
        ServiceId::Session,
        ProcessKind::Session,
        b"\\EFI\\LOGOS\\SESSION.ELF",
        Capabilities::SESSION,
    ),
    ServiceImageSpec::new(
        ServiceId::Commands,
        ProcessKind::Command,
        b"\\EFI\\LOGOS\\COMMANDS.ELF",
        Capabilities::COMMAND,
    ),
];

pub const fn service_image(service: ServiceId) -> ServiceImageSpec {
    match service {
        ServiceId::Input => SERVICE_IMAGES[0],
        ServiceId::Display => SERVICE_IMAGES[1],
        ServiceId::Terminal => SERVICE_IMAGES[2],
        ServiceId::Session => SERVICE_IMAGES[3],
        ServiceId::Commands => SERVICE_IMAGES[4],
    }
}

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
        assert_eq!(SERVICE_IMAGES.len(), 5);
        assert_eq!(SERVICE_IMAGES[0].service(), ServiceId::Input);
        assert_eq!(SERVICE_IMAGES[1].service(), ServiceId::Display);
        assert_eq!(SERVICE_IMAGES[2].service(), ServiceId::Terminal);
        assert_eq!(SERVICE_IMAGES[3].service(), ServiceId::Session);
        assert_eq!(SERVICE_IMAGES[4].service(), ServiceId::Commands);
        assert_eq!(service_image(ServiceId::Terminal).process_kind(), ProcessKind::Terminal);
        assert_eq!(service_image(ServiceId::Display).path(), b"\\EFI\\LOGOS\\DISPLAY.ELF");
    }

    #[test]
    fn capabilities_are_explicit_and_bounded() {
        let terminal = service_image(ServiceId::Terminal);
        assert_eq!(terminal.capabilities(), Capabilities::TERMINAL);
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
