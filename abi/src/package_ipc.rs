use super::{ABI_VERSION, CapabilityHandle, IPC_PAGE_BYTES, MAX_SERVICE_IMAGE_BYTES, ServiceId};

pub const PACKAGE_TRANSFER_BYTES: usize = IPC_PAGE_BYTES;
pub const MAX_PACKAGE_NAME_BYTES: usize = 32;
const MIN_PACKAGE_HEADER_BYTES: usize = 32;
const PACKAGE_HEADER_BYTES: usize = 404;
const MAX_PACKAGE_BYTES: usize = PACKAGE_HEADER_BYTES + MAX_SERVICE_IMAGE_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PackageTargetKind {
    Service = 1,
    Program = 2,
}

impl PackageTargetKind {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Service),
            2 => Some(Self::Program),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct PackageTarget {
    pub kind: PackageTargetKind,
    pub service: u8,
    pub name_len: u8,
    pub reserved: u8,
    pub name: [u8; MAX_PACKAGE_NAME_BYTES],
}

impl PackageTarget {
    pub const EMPTY: Self = Self {
        kind: PackageTargetKind::Service,
        service: 0,
        name_len: 0,
        reserved: 0,
        name: [0; MAX_PACKAGE_NAME_BYTES],
    };

    pub const fn service(service: ServiceId) -> Self {
        Self { kind: PackageTargetKind::Service, service: service as u8, ..Self::EMPTY }
    }

    pub fn program(name: &[u8]) -> Option<Self> {
        if name.is_empty()
            || name.len() > MAX_PACKAGE_NAME_BYTES
            || name[0] == b'-'
            || name[name.len() - 1] == b'-'
            || name.iter().any(|byte| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
        {
            return None;
        }
        let mut target = Self {
            kind: PackageTargetKind::Program,
            service: 0,
            name_len: name.len() as u8,
            reserved: 0,
            name: [0; MAX_PACKAGE_NAME_BYTES],
        };
        target.name[..name.len()].copy_from_slice(name);
        Some(target)
    }

    pub fn validate(self) -> Result<(), PackageStatus> {
        if self.reserved != 0 || self.name_len as usize > MAX_PACKAGE_NAME_BYTES {
            return Err(PackageStatus::Invalid);
        }
        match self.kind {
            PackageTargetKind::Service => {
                if ServiceId::from_index(self.service.saturating_sub(1) as usize).is_none()
                    || self.name_len != 0
                    || self.name.iter().any(|byte| *byte != 0)
                {
                    return Err(PackageStatus::Invalid);
                }
            }
            PackageTargetKind::Program => {
                if self.service != 0
                    || self.name_len == 0
                    || self.name[self.name_len as usize..].iter().any(|byte| *byte != 0)
                {
                    return Err(PackageStatus::Invalid);
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PackageOperation {
    Lookup = 1,
    Read = 2,
}

impl PackageOperation {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Lookup),
            2 => Some(Self::Read),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PackageStatus {
    Ok = 0,
    Invalid = 1,
    Io = 2,
    NotFound = 3,
    Stale = 4,
    Unsupported = 5,
    Full = 6,
}

impl PackageStatus {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Ok),
            1 => Some(Self::Invalid),
            2 => Some(Self::Io),
            3 => Some(Self::NotFound),
            4 => Some(Self::Stale),
            5 => Some(Self::Unsupported),
            6 => Some(Self::Full),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct PackageRequest {
    pub operation: PackageOperation,
    pub flags: u8,
    pub target: PackageTarget,
    pub request_id: u32,
    pub generation: u16,
    pub capability: CapabilityHandle,
    pub service_epoch: u64,
    pub package_generation: u32,
    pub offset: u32,
    pub length: u16,
    pub reserved2: u16,
}

impl PackageRequest {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        operation: PackageOperation,
        service: ServiceId,
        request_id: u32,
        generation: u16,
        capability: CapabilityHandle,
        service_epoch: u64,
        package_generation: u32,
        offset: u32,
        length: u16,
    ) -> Option<Self> {
        if request_id == 0 || generation == 0 || service_epoch == 0 {
            return None;
        }
        match operation {
            PackageOperation::Lookup if package_generation == 0 && offset == 0 && length == 0 => {}
            PackageOperation::Read
                if package_generation != 0
                    && (length as usize) <= PACKAGE_TRANSFER_BYTES
                    && length != 0
                    && offset.checked_add(length as u32).is_some() => {}
            _ => return None,
        }
        Some(Self {
            operation,
            flags: 0,
            target: PackageTarget::service(service),
            request_id,
            generation,
            capability,
            service_epoch,
            package_generation,
            offset,
            length,
            reserved2: 0,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_program(
        operation: PackageOperation,
        name: &[u8],
        request_id: u32,
        generation: u16,
        capability: CapabilityHandle,
        service_epoch: u64,
        package_generation: u32,
        offset: u32,
        length: u16,
    ) -> Option<Self> {
        let mut request = Self::new(
            operation,
            ServiceId::Storage,
            request_id,
            generation,
            capability,
            service_epoch,
            package_generation,
            offset,
            length,
        )?;
        request.target = PackageTarget::program(name)?;
        Some(request)
    }

    pub fn validate(
        self,
        capability: CapabilityHandle,
        generation: u16,
        service_epoch: u64,
    ) -> Result<ServiceId, PackageStatus> {
        let target = self.validate_target(capability, generation, service_epoch)?;
        match target.kind {
            PackageTargetKind::Service => {
                ServiceId::from_index(target.service.saturating_sub(1) as usize)
                    .ok_or(PackageStatus::Invalid)
            }
            PackageTargetKind::Program => Err(PackageStatus::Unsupported),
        }
    }

    pub fn validate_target(
        self,
        capability: CapabilityHandle,
        generation: u16,
        service_epoch: u64,
    ) -> Result<PackageTarget, PackageStatus> {
        if self.capability != capability {
            return Err(PackageStatus::Invalid);
        }
        if self.generation != generation || self.service_epoch != service_epoch {
            return Err(PackageStatus::Stale);
        }
        if self.flags != 0 || self.reserved2 != 0 || self.request_id == 0 {
            return Err(PackageStatus::Invalid);
        }
        self.target.validate()?;
        Ok(self.target)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct PackageResponse {
    pub operation: PackageOperation,
    pub status: PackageStatus,
    pub target: PackageTarget,
    pub request_id: u32,
    pub generation: u16,
    pub reserved2: u16,
    pub service_epoch: u64,
    pub package_generation: u32,
    pub offset: u32,
    pub bytes: u16,
    pub reserved3: u16,
    pub package_bytes: u32,
    pub package_version: u32,
    pub payload_crc32c: u32,
}

impl PackageResponse {
    pub const fn new(request: PackageRequest, status: PackageStatus) -> Self {
        Self {
            operation: request.operation,
            status,
            target: request.target,
            request_id: request.request_id,
            generation: request.generation,
            reserved2: 0,
            service_epoch: request.service_epoch,
            package_generation: request.package_generation,
            offset: request.offset,
            bytes: 0,
            reserved3: 0,
            package_bytes: 0,
            package_version: 0,
            payload_crc32c: 0,
        }
    }

    pub const fn with_package(
        mut self,
        package_generation: u32,
        package_bytes: u32,
        package_version: u32,
        payload_crc32c: u32,
    ) -> Self {
        self.package_generation = package_generation;
        self.package_bytes = package_bytes;
        self.package_version = package_version;
        self.payload_crc32c = payload_crc32c;
        self
    }

    pub const fn with_bytes(mut self, bytes: u16) -> Self {
        self.bytes = bytes;
        self
    }

    pub fn validate_for(
        self,
        request: PackageRequest,
        generation: u16,
        service_epoch: u64,
    ) -> Result<(), PackageStatus> {
        if self.operation != request.operation
            || self.request_id != request.request_id
            || self.generation != generation
            || self.service_epoch != service_epoch
            || self.target != request.target
            || self.reserved2 != 0
            || self.reserved3 != 0
            || PackageStatus::from_raw(self.status as u8).is_none()
            || self.bytes as usize > request.length as usize
        {
            return Err(PackageStatus::Invalid);
        }
        if self.status == PackageStatus::Ok {
            match request.operation {
                PackageOperation::Lookup
                    if self.package_generation == 0
                        || self.package_bytes < MIN_PACKAGE_HEADER_BYTES as u32
                        || self.package_bytes as usize > MAX_PACKAGE_BYTES
                        || self.offset != 0
                        || self.bytes != 0 =>
                {
                    return Err(PackageStatus::Invalid);
                }
                PackageOperation::Read
                    if self.package_generation != request.package_generation
                        || self.offset != request.offset
                        || self.bytes == 0 =>
                {
                    return Err(PackageStatus::Invalid);
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn wire_enums_valid(bytes: &[u8]) -> bool {
        bytes.get(core::mem::offset_of!(Self, operation)).is_some_and(|raw| {
            *raw >= PackageOperation::Lookup as u8 && *raw <= PackageOperation::Read as u8
        }) && bytes
            .get(core::mem::offset_of!(Self, status))
            .is_some_and(|raw| *raw <= PackageStatus::Full as u8)
            && bytes.get(core::mem::offset_of!(Self, target)).is_some_and(|raw| {
                *raw >= PackageTargetKind::Service as u8 && *raw <= PackageTargetKind::Program as u8
            })
    }
}

pub const PACKAGE_ABI_VERSION: u16 = ABI_VERSION;

#[cfg(test)]
mod tests {
    use super::*;

    fn capability() -> CapabilityHandle {
        CapabilityHandle::new(6, 1).unwrap()
    }

    fn lookup() -> PackageRequest {
        PackageRequest::new(
            PackageOperation::Lookup,
            ServiceId::Storage,
            7,
            3,
            capability(),
            9,
            0,
            0,
            0,
        )
        .unwrap()
    }

    #[test]
    fn request_validation_is_bound_to_identity_and_shape() {
        let request = lookup();
        assert_eq!(request.validate(capability(), 3, 9), Ok(ServiceId::Storage));
        assert_eq!(
            request.validate(CapabilityHandle::new(7, 1).unwrap(), 3, 9),
            Err(PackageStatus::Invalid)
        );
        assert_eq!(request.validate(capability(), 4, 9), Err(PackageStatus::Stale));
        let mut malformed = request;
        malformed.flags = 1;
        assert_eq!(malformed.validate(capability(), 3, 9), Err(PackageStatus::Invalid));
    }

    #[test]
    fn read_response_rejects_stale_request_and_oversized_transfer() {
        let request = PackageRequest::new(
            PackageOperation::Read,
            ServiceId::Storage,
            1,
            3,
            capability(),
            9,
            2,
            4096,
            PACKAGE_TRANSFER_BYTES as u16,
        )
        .unwrap();
        let response = PackageResponse::new(request, PackageStatus::Ok)
            .with_package(2, 8192, 4, 0x55)
            .with_bytes(PACKAGE_TRANSFER_BYTES as u16);
        assert_eq!(response.validate_for(request, 3, 9), Ok(()));
        assert_eq!(response.validate_for(request, 4, 9), Err(PackageStatus::Invalid));
        assert!(
            PackageRequest::new(
                PackageOperation::Read,
                ServiceId::Storage,
                1,
                3,
                capability(),
                9,
                2,
                0,
                (PACKAGE_TRANSFER_BYTES + 1) as u16,
            )
            .is_none()
        );
        assert!(
            PackageRequest::new(
                PackageOperation::Read,
                ServiceId::Storage,
                1,
                3,
                capability(),
                9,
                2,
                u32::MAX,
                1,
            )
            .is_none()
        );
        let invalid_lookup =
            PackageResponse::new(lookup(), PackageStatus::Ok).with_package(0, 64, 1, 0);
        assert_eq!(invalid_lookup.validate_for(lookup(), 3, 9), Err(PackageStatus::Invalid));
    }

    #[test]
    fn program_target_roundtrips_without_service_aliasing() {
        let request = PackageRequest::new_program(
            PackageOperation::Lookup,
            b"demo",
            9,
            3,
            capability(),
            11,
            0,
            0,
            0,
        )
        .unwrap();
        assert_eq!(
            request.validate_target(capability(), 3, 11).unwrap().kind,
            PackageTargetKind::Program
        );
        assert_eq!(request.validate(capability(), 3, 11), Err(PackageStatus::Unsupported));
        let response = PackageResponse::new(request, PackageStatus::Ok).with_package(1, 404, 0, 0);
        assert_eq!(response.validate_for(request, 3, 11), Ok(()));
    }

    #[test]
    fn stale_response_can_be_followed_by_current_response() {
        let request = PackageRequest::new(
            PackageOperation::Lookup,
            ServiceId::Storage,
            1,
            3,
            capability(),
            9,
            0,
            0,
            0,
        )
        .unwrap();
        let stale = PackageResponse::new(request, PackageStatus::Ok)
            .with_package(1, PACKAGE_HEADER_BYTES as u32, 1, 0)
            .with_bytes(0);
        let current = PackageResponse::new(request, PackageStatus::Ok)
            .with_package(2, PACKAGE_HEADER_BYTES as u32, 1, 0)
            .with_bytes(0);

        assert_eq!(stale.validate_for(request, 4, 9), Err(PackageStatus::Invalid));
        assert_eq!(current.validate_for(request, 3, 9), Ok(()));
    }

    #[test]
    fn response_wire_enums_are_checked_before_decoding() {
        let mut bytes = [0u8; core::mem::size_of::<PackageResponse>()];
        bytes[core::mem::offset_of!(PackageResponse, operation)] = PackageOperation::Lookup as u8;
        bytes[core::mem::offset_of!(PackageResponse, status)] = PackageStatus::Ok as u8;
        bytes[core::mem::offset_of!(PackageResponse, target)] = PackageTargetKind::Service as u8;
        assert!(PackageResponse::wire_enums_valid(&bytes));
        bytes[core::mem::offset_of!(PackageResponse, status)] = u8::MAX;
        assert!(!PackageResponse::wire_enums_valid(&bytes));
    }
}
