//! Bounded retained-image records for service ELF files.

use logos_abi::{MAX_SERVICE_IMAGE_BYTES, ServiceId};

use crate::service_images::{SERVICE_IMAGES, ServiceImageError, ServiceImageSpec};

const PAGE_SIZE: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceImageLocation {
    service: ServiceId,
    physical_address: usize,
    image_bytes: usize,
    allocation_bytes: usize,
}

impl ServiceImageLocation {
    pub const fn service(self) -> ServiceId {
        self.service
    }

    pub const fn physical_address(self) -> usize {
        self.physical_address
    }

    pub const fn image_bytes(self) -> usize {
        self.image_bytes
    }

    pub const fn allocation_bytes(self) -> usize {
        self.allocation_bytes
    }

    fn new(
        service: ServiceId,
        physical_address: usize,
        image_bytes: usize,
    ) -> Result<Self, ServiceLoadError> {
        if physical_address == 0 || physical_address & (PAGE_SIZE - 1) != 0 {
            return Err(ServiceLoadError::InvalidAddress);
        }
        if image_bytes == 0 {
            return Err(ServiceLoadError::Empty);
        }
        if image_bytes > MAX_SERVICE_IMAGE_BYTES {
            return Err(ServiceLoadError::TooLarge);
        }
        let allocation_bytes = align_up(image_bytes).ok_or(ServiceLoadError::InvalidAddress)?;
        physical_address.checked_add(allocation_bytes).ok_or(ServiceLoadError::InvalidAddress)?;
        Ok(Self { service, physical_address, image_bytes, allocation_bytes })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceLoadError {
    Empty,
    TooLarge,
    InvalidAddress,
    Duplicate,
    InvalidElf(ServiceImageError),
}

pub struct ServiceImageBundle {
    records: [Option<ServiceImageLocation>; SERVICE_IMAGES.len()],
    count: usize,
}

impl ServiceImageBundle {
    pub const fn new() -> Self {
        Self { records: [None; SERVICE_IMAGES.len()], count: 0 }
    }

    pub const fn count(&self) -> usize {
        self.count
    }

    pub const fn complete(&self) -> bool {
        self.count == SERVICE_IMAGES.len()
    }

    pub const fn location(&self, service: ServiceId) -> Option<ServiceImageLocation> {
        self.records[service_index(service)]
    }

    /// Admit one validated image whose bytes are already in retained memory.
    ///
    /// The UEFI reader owns copying into the physical allocation; this type
    /// owns only the fixed metadata needed after `ExitBootServices`.
    pub fn admit(
        &mut self,
        spec: ServiceImageSpec,
        physical_address: usize,
        image: &[u8],
    ) -> Result<ServiceImageLocation, ServiceLoadError> {
        let index = service_index(spec.service());
        if self.records[index].is_some() {
            return Err(ServiceLoadError::Duplicate);
        }
        spec.validate_image(image).map_err(ServiceLoadError::InvalidElf)?;
        let location = ServiceImageLocation::new(spec.service(), physical_address, image.len())?;
        self.records[index] = Some(location);
        self.count += 1;
        Ok(location)
    }

    pub fn clear(&mut self) {
        self.records.fill(None);
        self.count = 0;
    }
}

impl Default for ServiceImageBundle {
    fn default() -> Self {
        Self::new()
    }
}

const fn service_index(service: ServiceId) -> usize {
    match service {
        ServiceId::Input => 0,
        ServiceId::Display => 1,
        ServiceId::Terminal => 2,
        ServiceId::Session => 3,
        ServiceId::Commands => 4,
    }
}

fn align_up(bytes: usize) -> Option<usize> {
    bytes.checked_add(PAGE_SIZE - 1).map(|value| value & !(PAGE_SIZE - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service_images::service_image;

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
    fn bundle_retains_page_aligned_locations_and_requires_valid_elf() {
        let mut bundle = ServiceImageBundle::new();
        let spec = service_image(ServiceId::Input);
        let location = bundle.admit(spec, 0x20_000, &image()).unwrap();
        assert_eq!(location.service(), ServiceId::Input);
        assert_eq!(location.physical_address(), 0x20_000);
        assert_eq!(location.image_bytes(), 128);
        assert_eq!(location.allocation_bytes(), PAGE_SIZE);
        assert_eq!(bundle.location(ServiceId::Input), Some(location));
        assert_eq!(bundle.count(), 1);
        assert!(!bundle.complete());
    }

    #[test]
    fn bundle_rejects_duplicate_invalid_and_oversized_images() {
        let mut bundle = ServiceImageBundle::new();
        let spec = service_image(ServiceId::Display);
        assert_eq!(
            bundle.admit(spec, 0x30_000, &[0; 64]),
            Err(ServiceLoadError::InvalidElf(ServiceImageError::InvalidElf(
                crate::process::ProcessError::InvalidImage
            )))
        );
        assert_eq!(bundle.admit(spec, 0x30_123, &image()), Err(ServiceLoadError::InvalidAddress));
        assert_eq!(
            bundle.admit(spec, 0x30_000, &image()),
            Ok(ServiceImageLocation::new(ServiceId::Display, 0x30_000, 128).unwrap())
        );
        assert_eq!(bundle.admit(spec, 0x40_000, &image()), Err(ServiceLoadError::Duplicate));
    }

    #[test]
    fn clear_releases_all_fixed_records() {
        let mut bundle = ServiceImageBundle::new();
        bundle.admit(service_image(ServiceId::Commands), 0x50_000, &image()).unwrap();
        bundle.clear();
        assert_eq!(bundle.count(), 0);
        assert_eq!(bundle.location(ServiceId::Commands), None);
    }
}
