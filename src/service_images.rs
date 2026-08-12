//! Fixed manifest for the ring-3 service images.

use logos_abi::{
    CapabilityKind, MAX_CAPABILITIES, MAX_SERVICE_IMAGE_BYTES, ServiceDescriptor, ServiceId,
};

use crate::process::{ElfLoadPlan, ProcessError, ProcessKind};

pub const MAX_IMAGE_CAPABILITIES: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityGrant {
    pub kind: CapabilityKind,
    pub slot: u16,
}

impl CapabilityGrant {
    const fn new(kind: CapabilityKind, slot: u16) -> Self {
        Self { kind, slot }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceImageSpec {
    service: ServiceId,
    process_kind: ProcessKind,
    path: &'static [u8],
    capabilities: [Option<CapabilityGrant>; MAX_IMAGE_CAPABILITIES],
}

impl ServiceImageSpec {
    const fn new(
        service: ServiceId,
        process_kind: ProcessKind,
        path: &'static [u8],
        capabilities: [Option<CapabilityGrant>; MAX_IMAGE_CAPABILITIES],
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

    pub const fn capability_count(self) -> usize {
        let mut count = 0;
        while count < MAX_IMAGE_CAPABILITIES {
            if self.capabilities[count].is_some() {
                count += 1;
            } else {
                break;
            }
        }
        count
    }

    pub const fn capability(self, index: usize) -> Option<CapabilityGrant> {
        if index < MAX_IMAGE_CAPABILITIES { self.capabilities[index] } else { None }
    }

    pub fn descriptor(self, image_bytes: usize) -> Result<ServiceDescriptor, ServiceImageError> {
        validate_image_bytes(image_bytes)?;
        if self.capability_count() > MAX_CAPABILITIES {
            return Err(ServiceImageError::CapabilityLimit);
        }
        let mut descriptor = ServiceDescriptor::new(self.service, 1, 1);
        descriptor.image_bytes = image_bytes as u32;
        descriptor.capability_count = self.capability_count() as u16;
        Ok(descriptor)
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
    CapabilityLimit,
}

pub const SERVICE_IMAGES: [ServiceImageSpec; 5] = [
    ServiceImageSpec::new(
        ServiceId::Input,
        ProcessKind::Input,
        b"\\EFI\\LOGOS\\INPUT.ELF",
        [
            Some(CapabilityGrant::new(CapabilityKind::KeyboardBytes, 0)),
            Some(CapabilityGrant::new(CapabilityKind::IpcEndpoint, 1)),
            None,
            None,
        ],
    ),
    ServiceImageSpec::new(
        ServiceId::Display,
        ProcessKind::Display,
        b"\\EFI\\LOGOS\\DISPLAY.ELF",
        [
            Some(CapabilityGrant::new(CapabilityKind::Framebuffer, 0)),
            Some(CapabilityGrant::new(CapabilityKind::IpcEndpoint, 1)),
            None,
            None,
        ],
    ),
    ServiceImageSpec::new(
        ServiceId::Terminal,
        ProcessKind::Terminal,
        b"\\EFI\\LOGOS\\TERMINAL.ELF",
        [
            Some(CapabilityGrant::new(CapabilityKind::IpcEndpoint, 1)),
            Some(CapabilityGrant::new(CapabilityKind::IpcEndpoint, 2)),
            Some(CapabilityGrant::new(CapabilityKind::IpcEndpoint, 3)),
            Some(CapabilityGrant::new(CapabilityKind::IpcEndpoint, 4)),
        ],
    ),
    ServiceImageSpec::new(
        ServiceId::Session,
        ProcessKind::Session,
        b"\\EFI\\LOGOS\\SESSION.ELF",
        [
            Some(CapabilityGrant::new(CapabilityKind::IpcEndpoint, 1)),
            Some(CapabilityGrant::new(CapabilityKind::IpcEndpoint, 2)),
            Some(CapabilityGrant::new(CapabilityKind::IpcEndpoint, 3)),
            None,
        ],
    ),
    ServiceImageSpec::new(
        ServiceId::Commands,
        ProcessKind::Command,
        b"\\EFI\\LOGOS\\COMMANDS.ELF",
        [
            Some(CapabilityGrant::new(CapabilityKind::IpcEndpoint, 1)),
            Some(CapabilityGrant::new(CapabilityKind::ProcessControl, 2)),
            None,
            None,
        ],
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
    fn capabilities_are_explicit_and_descriptors_are_bounded() {
        let terminal = service_image(ServiceId::Terminal);
        assert_eq!(terminal.capability_count(), 4);
        assert_eq!(terminal.capability(0).unwrap().slot, 1);
        assert_eq!(terminal.capability(3).unwrap().slot, 4);
        let descriptor = terminal.descriptor(128).unwrap();
        assert_eq!(descriptor.image_bytes, 128);
        assert_eq!(descriptor.capability_count, 4);
        assert_eq!(descriptor.stack_pages, 8);
    }

    #[test]
    fn image_validation_rejects_empty_oversize_and_malformed_inputs() {
        let spec = service_image(ServiceId::Input);
        assert_eq!(spec.validate_image(&[]), Err(ServiceImageError::Empty));
        assert_eq!(spec.descriptor(MAX_SERVICE_IMAGE_BYTES + 1), Err(ServiceImageError::TooLarge));
        assert!(matches!(spec.validate_image(&[0; 64]), Err(ServiceImageError::InvalidElf(_))));
        assert_eq!(spec.validate_image(&image()).unwrap().entry(), 0x1000);
    }
}
