#![no_std]

use logos_abi::{ABI_VERSION, MAX_SERVICE_IMAGE_BYTES, ServiceId};

#[cfg(test)]
extern crate std;

pub const PACKAGE_MAGIC: [u8; 8] = *b"LOGOSPKG";
pub const PACKAGE_FORMAT_VERSION: u16 = 1;
pub const PACKAGE_KIND_SERVICE: u8 = 1;
pub const PACKAGE_HEADER_BYTES: usize = 32;
pub const MAX_PACKAGE_PAYLOAD_BYTES: usize = MAX_SERVICE_IMAGE_BYTES;
pub const MAX_PACKAGE_BYTES: usize = PACKAGE_HEADER_BYTES + MAX_PACKAGE_PAYLOAD_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageError {
    Reader,
    HeaderTooSmall,
    InvalidMagic,
    UnsupportedFormat,
    UnsupportedKind,
    InvalidService,
    WrongService,
    WrongAbi,
    InvalidPayloadLength,
    PayloadTooLarge,
    LengthMismatch,
    CrcMismatch,
}

#[allow(clippy::len_without_is_empty)]
pub trait PackageReader {
    fn len(&self) -> usize;
    fn read(&mut self, offset: usize, output: &mut [u8]) -> Result<usize, PackageError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServicePackageHeader {
    pub service: ServiceId,
    pub abi_version: u16,
    pub payload_length: u32,
    pub package_version: u32,
    pub payload_crc32c: u32,
}

impl ServicePackageHeader {
    pub const fn new(
        service: ServiceId,
        package_version: u32,
        payload_length: usize,
        payload_crc32c: u32,
    ) -> Option<Self> {
        if payload_length == 0 || payload_length > MAX_PACKAGE_PAYLOAD_BYTES {
            return None;
        }
        Some(Self {
            service,
            abi_version: ABI_VERSION,
            payload_length: payload_length as u32,
            package_version,
            payload_crc32c,
        })
    }

    pub const fn encoded_len() -> usize {
        PACKAGE_HEADER_BYTES
    }

    pub fn encode(self, output: &mut [u8]) -> Result<(), PackageError> {
        if output.len() < PACKAGE_HEADER_BYTES {
            return Err(PackageError::HeaderTooSmall);
        }
        output[..PACKAGE_HEADER_BYTES].fill(0);
        output[..8].copy_from_slice(&PACKAGE_MAGIC);
        put_u16(output, 8, PACKAGE_FORMAT_VERSION);
        output[10] = self.service as u8;
        output[11] = PACKAGE_KIND_SERVICE;
        put_u16(output, 12, self.abi_version);
        put_u32(output, 16, self.payload_length);
        put_u32(output, 20, self.package_version);
        put_u32(output, 24, self.payload_crc32c);
        Ok(())
    }

    pub fn decode(input: &[u8]) -> Result<Self, PackageError> {
        if input.len() < PACKAGE_HEADER_BYTES {
            return Err(PackageError::HeaderTooSmall);
        }
        if input[..8] != PACKAGE_MAGIC {
            return Err(PackageError::InvalidMagic);
        }
        if get_u16(input, 8) != PACKAGE_FORMAT_VERSION {
            return Err(PackageError::UnsupportedFormat);
        }
        if input[11] != PACKAGE_KIND_SERVICE {
            return Err(PackageError::UnsupportedKind);
        }
        if input[14..16].iter().any(|byte| *byte != 0)
            || input[28..PACKAGE_HEADER_BYTES].iter().any(|byte| *byte != 0)
        {
            return Err(PackageError::InvalidPayloadLength);
        }
        let service = input[10]
            .checked_sub(1)
            .and_then(|raw| ServiceId::from_index(raw as usize))
            .ok_or(PackageError::InvalidService)?;
        let payload_length = get_u32(input, 16) as usize;
        if payload_length == 0 {
            return Err(PackageError::InvalidPayloadLength);
        }
        if payload_length > MAX_PACKAGE_PAYLOAD_BYTES {
            return Err(PackageError::PayloadTooLarge);
        }
        Ok(Self {
            service,
            abi_version: get_u16(input, 12),
            payload_length: payload_length as u32,
            package_version: get_u32(input, 20),
            payload_crc32c: get_u32(input, 24),
        })
    }

    pub fn validate_for(self, service: ServiceId, abi_version: u16) -> Result<(), PackageError> {
        if self.service != service {
            return Err(PackageError::WrongService);
        }
        if self.abi_version != abi_version {
            return Err(PackageError::WrongAbi);
        }
        Ok(())
    }
}

pub fn validate_package<R: PackageReader>(
    reader: &mut R,
    service: ServiceId,
    abi_version: u16,
    scratch: &mut [u8],
) -> Result<ServicePackageHeader, PackageError> {
    if scratch.is_empty() {
        return Err(PackageError::Reader);
    }
    if reader.len() < PACKAGE_HEADER_BYTES {
        return Err(PackageError::HeaderTooSmall);
    }
    let mut header_bytes = [0u8; PACKAGE_HEADER_BYTES];
    read_exact(reader, 0, &mut header_bytes)?;
    let header = ServicePackageHeader::decode(&header_bytes)?;
    header.validate_for(service, abi_version)?;
    let expected_len = PACKAGE_HEADER_BYTES
        .checked_add(header.payload_length as usize)
        .ok_or(PackageError::PayloadTooLarge)?;
    if reader.len() != expected_len {
        return Err(PackageError::LengthMismatch);
    }

    let mut crc = 0xffff_ffff;
    let mut offset = PACKAGE_HEADER_BYTES;
    let end = expected_len;
    while offset < end {
        let amount = (end - offset).min(scratch.len());
        read_exact(reader, offset, &mut scratch[..amount])?;
        crc = crc32c_update(crc, &scratch[..amount]);
        offset += amount;
    }
    if !crc32c_finish(crc).eq(&header.payload_crc32c) {
        return Err(PackageError::CrcMismatch);
    }
    Ok(header)
}

pub fn crc32c(bytes: &[u8]) -> u32 {
    crc32c_finish(crc32c_update(0xffff_ffff, bytes))
}

fn read_exact<R: PackageReader>(
    reader: &mut R,
    offset: usize,
    output: &mut [u8],
) -> Result<(), PackageError> {
    let end = offset.checked_add(output.len()).ok_or(PackageError::Reader)?;
    if end > reader.len() {
        return Err(PackageError::Reader);
    }
    let amount = reader.read(offset, output)?;
    if amount != output.len() {
        return Err(PackageError::Reader);
    }
    Ok(())
}

fn crc32c_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0x82f6_3b78 } else { crc >> 1 };
        }
    }
    crc
}

const fn crc32c_finish(crc: u32) -> u32 {
    !crc
}

fn put_u16(output: &mut [u8], offset: usize, value: u16) {
    output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn get_u16(input: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([input[offset], input[offset + 1]])
}

fn get_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([input[offset], input[offset + 1], input[offset + 2], input[offset + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;
    use std::vec::Vec;

    struct Reader(Vec<u8>);

    impl PackageReader for Reader {
        fn len(&self) -> usize {
            self.0.len()
        }

        fn read(&mut self, offset: usize, output: &mut [u8]) -> Result<usize, PackageError> {
            let end = offset + output.len();
            if end > self.0.len() {
                return Err(PackageError::Reader);
            }
            output.copy_from_slice(&self.0[offset..end]);
            Ok(output.len())
        }
    }

    fn package(service: ServiceId, abi: u16, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; PACKAGE_HEADER_BYTES + payload.len()];
        let mut header =
            ServicePackageHeader::new(service, 7, payload.len(), crc32c(payload)).unwrap();
        header.abi_version = abi;
        header.encode(&mut bytes).unwrap();
        bytes[PACKAGE_HEADER_BYTES..].copy_from_slice(payload);
        bytes
    }

    #[test]
    fn valid_package_round_trips_and_stream_validates() {
        let bytes = package(ServiceId::Storage, ABI_VERSION, b"elf");
        let mut reader = Reader(bytes);
        let mut scratch = [0; 2];
        let header =
            validate_package(&mut reader, ServiceId::Storage, ABI_VERSION, &mut scratch).unwrap();
        assert_eq!(header.package_version, 7);
    }

    #[test]
    fn invalid_headers_are_bounded_and_typed() {
        assert_eq!(ServicePackageHeader::decode(&[]), Err(PackageError::HeaderTooSmall));
        let mut bytes = package(ServiceId::Storage, ABI_VERSION, b"elf");
        bytes[0] ^= 1;
        assert_eq!(ServicePackageHeader::decode(&bytes), Err(PackageError::InvalidMagic));
        bytes = package(ServiceId::Storage, ABI_VERSION, b"elf");
        bytes[11] = 2;
        assert_eq!(ServicePackageHeader::decode(&bytes), Err(PackageError::UnsupportedKind));
    }

    #[test]
    fn wrong_service_and_abi_are_rejected() {
        let bytes = package(ServiceId::Storage, ABI_VERSION, b"elf");
        let mut reader = Reader(bytes.clone());
        let mut scratch = [0; 8];
        assert_eq!(
            validate_package(&mut reader, ServiceId::Flow, ABI_VERSION, &mut scratch),
            Err(PackageError::WrongService)
        );
        let mut reader = Reader(bytes);
        assert_eq!(
            validate_package(&mut reader, ServiceId::Storage, ABI_VERSION + 1, &mut scratch),
            Err(PackageError::WrongAbi)
        );
    }

    #[test]
    fn truncation_oversize_and_crc_mismatch_are_rejected() {
        let bytes = package(ServiceId::Storage, ABI_VERSION, b"elf");
        let mut reader = Reader(bytes[..bytes.len() - 1].to_vec());
        let mut scratch = [0; 8];
        assert_eq!(
            validate_package(&mut reader, ServiceId::Storage, ABI_VERSION, &mut scratch),
            Err(PackageError::LengthMismatch)
        );

        let mut bytes = package(ServiceId::Storage, ABI_VERSION, b"elf");
        bytes[PACKAGE_HEADER_BYTES] ^= 1;
        let mut reader = Reader(bytes);
        assert_eq!(
            validate_package(&mut reader, ServiceId::Storage, ABI_VERSION, &mut scratch),
            Err(PackageError::CrcMismatch)
        );

        let mut header = [0; PACKAGE_HEADER_BYTES];
        header[..8].copy_from_slice(&PACKAGE_MAGIC);
        put_u16(&mut header, 8, PACKAGE_FORMAT_VERSION);
        header[10] = ServiceId::Storage as u8;
        header[11] = PACKAGE_KIND_SERVICE;
        put_u16(&mut header, 12, ABI_VERSION);
        put_u32(&mut header, 16, (MAX_PACKAGE_PAYLOAD_BYTES as u32) + 1);
        assert_eq!(ServicePackageHeader::decode(&header), Err(PackageError::PayloadTooLarge));
    }
}
