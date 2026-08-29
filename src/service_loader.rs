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

    /// View the retained image after UEFI has been exited.
    ///
    /// # Safety
    ///
    /// The caller must invoke this only while the UEFI allocation represented
    /// by this record remains identity-mapped and reserved. The returned view
    /// must not outlive the bundle or any architecture teardown of that
    /// allocation.
    #[cfg(target_os = "uefi")]
    pub unsafe fn bytes(&self) -> &[u8] {
        // SAFETY: The caller guarantees that the retained physical allocation
        // is identity-mapped and remains live for the returned view.
        unsafe { core::slice::from_raw_parts(self.physical_address as *const u8, self.image_bytes) }
    }

    fn new(
        service: ServiceId,
        physical_address: usize,
        image_bytes: usize,
    ) -> Result<Self, ServiceLoadError> {
        let allocation_bytes = align_up(image_bytes).ok_or(ServiceLoadError::InvalidAddress)?;
        Self::with_allocation(service, physical_address, image_bytes, allocation_bytes)
    }

    fn with_allocation(
        service: ServiceId,
        physical_address: usize,
        image_bytes: usize,
        allocation_bytes: usize,
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
        let minimum_allocation = align_up(image_bytes).ok_or(ServiceLoadError::InvalidAddress)?;
        if allocation_bytes < minimum_allocation
            || allocation_bytes & (PAGE_SIZE - 1) != 0
            || allocation_bytes > MAX_SERVICE_IMAGE_BYTES
        {
            return Err(ServiceLoadError::InvalidAddress);
        }
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
        self.records[service.index()]
    }

    /// View one retained service image after the UEFI filesystem is gone.
    ///
    /// # Safety
    ///
    /// The caller must keep the retained allocations identity-mapped and
    /// reserved for the lifetime of the returned slice.
    #[cfg(target_os = "uefi")]
    pub unsafe fn image(&self, service: ServiceId) -> Option<&[u8]> {
        // SAFETY: The caller assumes the retained-allocation invariant
        // documented by `ServiceImageLocation::bytes`.
        let location = self.records[service.index()].as_ref()?;
        Some(unsafe { location.bytes() })
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
        let index = spec.service().index();
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

#[cfg(target_os = "uefi")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UefiImageError {
    Firmware(uefi::Status),
    Path,
    NotRegularFile,
    Service(ServiceLoadError),
}

#[cfg(target_os = "uefi")]
/// Read every manifest image while UEFI services are still available.
///
/// Each file receives a bounded maximum allocation. The allocation remains
/// identity-mapped and is represented by `ServiceImageBundle` after the UEFI
/// handle is dropped.
pub fn load_from_esp() -> Result<ServiceImageBundle, UefiImageError> {
    use uefi::{
        boot,
        proto::media::file::{File, FileAttribute, FileInfo, FileMode},
    };
    #[repr(align(8))]
    struct FileInfoBuffer([u8; 512]);

    let mut filesystem = boot::get_image_file_system(boot::image_handle())
        .map_err(|error| UefiImageError::Firmware(error.status()))?;
    let mut root =
        filesystem.open_volume().map_err(|error| UefiImageError::Firmware(error.status()))?;
    let mut bundle = ServiceImageBundle::new();
    for spec in SERVICE_IMAGES {
        let path = spec.path();
        let mut path_buffer = [0u16; 64];
        if path.len() + 1 > path_buffer.len() || path.iter().any(|byte| *byte > 0x7f) {
            free_bundle(&mut bundle);
            return Err(UefiImageError::Path);
        }
        for (index, byte) in path.iter().enumerate() {
            path_buffer[index] = *byte as u16;
        }
        let path = uefi::CStr16::from_u16_with_nul(&path_buffer[..=path.len()]).map_err(|_| {
            free_bundle(&mut bundle);
            UefiImageError::Path
        })?;
        let file = root.open(path, FileMode::Read, FileAttribute::empty()).map_err(|error| {
            free_bundle(&mut bundle);
            UefiImageError::Firmware(error.status())
        })?;
        let mut file = file.into_regular_file().ok_or_else(|| {
            free_bundle(&mut bundle);
            UefiImageError::NotRegularFile
        })?;
        let mut info = FileInfoBuffer([0; 512]);
        let file_size = file
            .get_info::<FileInfo>(&mut info.0)
            .map_err(|error| {
                free_bundle(&mut bundle);
                UefiImageError::Firmware(error.status())
            })?
            .file_size();
        let file_size = usize::try_from(file_size).map_err(|_| {
            free_bundle(&mut bundle);
            UefiImageError::Service(ServiceLoadError::TooLarge)
        })?;
        if file_size == 0 || file_size > MAX_SERVICE_IMAGE_BYTES {
            free_bundle(&mut bundle);
            return Err(if file_size == 0 {
                UefiImageError::Service(ServiceLoadError::Empty)
            } else {
                UefiImageError::Service(ServiceLoadError::TooLarge)
            });
        }
        let pages = file_size.div_ceil(PAGE_SIZE);
        let allocation = boot::allocate_pages(
            boot::AllocateType::AnyPages,
            boot::MemoryType::LOADER_DATA,
            pages,
        )
        .map_err(|error| {
            free_bundle(&mut bundle);
            UefiImageError::Firmware(error.status())
        })?;
        let allocation_bytes = pages * PAGE_SIZE;
        let result = read_one_image(file, spec, allocation, allocation_bytes);
        match result {
            Ok(location) => {
                bundle.records[spec.service().index()] = Some(location);
                bundle.count += 1;
            }
            Err(error) => {
                let _ = unsafe { boot::free_pages(allocation, pages) };
                free_bundle(&mut bundle);
                return Err(error);
            }
        }
    }
    Ok(bundle)
}

#[cfg(target_os = "uefi")]
fn read_one_image(
    mut file: uefi::proto::media::file::RegularFile,
    spec: ServiceImageSpec,
    allocation: core::ptr::NonNull<u8>,
    allocation_bytes: usize,
) -> Result<ServiceImageLocation, UefiImageError> {
    let buffer = unsafe { core::slice::from_raw_parts_mut(allocation.as_ptr(), allocation_bytes) };
    let bytes = file.read(buffer).map_err(|error| UefiImageError::Firmware(error.status()))?;
    if bytes == 0 {
        return Err(UefiImageError::Service(ServiceLoadError::Empty));
    }
    if bytes == MAX_SERVICE_IMAGE_BYTES {
        let mut extra = [0u8; 1];
        let extra_bytes =
            file.read(&mut extra).map_err(|error| UefiImageError::Firmware(error.status()))?;
        if extra_bytes != 0 {
            return Err(UefiImageError::Service(ServiceLoadError::TooLarge));
        }
    }
    spec.validate_image(&buffer[..bytes])
        .map_err(|error| UefiImageError::Service(ServiceLoadError::InvalidElf(error)))?;
    ServiceImageLocation::with_allocation(
        spec.service(),
        allocation.as_ptr() as usize,
        bytes,
        allocation_bytes,
    )
    .map_err(UefiImageError::Service)
}

#[cfg(target_os = "uefi")]
fn free_bundle(bundle: &mut ServiceImageBundle) {
    use core::ptr::NonNull;
    use uefi::boot;

    for spec in SERVICE_IMAGES {
        if let Some(location) = bundle.location(spec.service()) {
            if let Some(pointer) = NonNull::new(location.physical_address() as *mut u8) {
                let pages = location.allocation_bytes() / PAGE_SIZE;
                let _ = unsafe { boot::free_pages(pointer, pages) };
            }
        }
    }
    bundle.clear();
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
        assert_eq!(
            ServiceImageLocation::with_allocation(ServiceId::Input, 0x20_000, 128, PAGE_SIZE * 2)
                .unwrap()
                .allocation_bytes(),
            PAGE_SIZE * 2
        );
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
        bundle.admit(service_image(ServiceId::Flow), 0x50_000, &image()).unwrap();
        bundle.clear();
        assert_eq!(bundle.count(), 0);
        assert_eq!(bundle.location(ServiceId::Flow), None);
    }
}
