#![no_std]

use logos_abi::{ABI_VERSION, MAX_SERVICE_IMAGE_BYTES, ServiceId};

#[cfg(test)]
extern crate std;

pub const PACKAGE_MAGIC: [u8; 8] = *b"LOGOSPKG";
pub const PACKAGE_FORMAT_VERSION: u16 = 1;
pub const PACKAGE_FORMAT_VERSION_V2: u16 = 2;
pub const PACKAGE_KIND_SERVICE: u8 = 1;
pub const PACKAGE_KIND_PROGRAM: u8 = 2;
pub const PACKAGE_HEADER_BYTES: usize = 32;
pub const PACKAGE_MANIFEST_BYTES: usize = 384;
pub const PACKAGE_HEADER_V2_BYTES: usize = 20 + PACKAGE_MANIFEST_BYTES;
pub const MAX_PACKAGE_PAYLOAD_BYTES: usize = MAX_SERVICE_IMAGE_BYTES;
pub const MAX_PACKAGE_BYTES: usize = PACKAGE_HEADER_BYTES + MAX_PACKAGE_PAYLOAD_BYTES;
pub const MAX_PACKAGE_BYTES_V2: usize = PACKAGE_HEADER_V2_BYTES + MAX_PACKAGE_PAYLOAD_BYTES;
pub const MAX_PACKAGE_NAME_BYTES: usize = 32;
pub const MAX_PACKAGE_DEPENDENCIES: usize = 4;
pub const MAX_VERSION_RANGE_BYTES: usize = 32;
pub const MAX_INSTALLED_PACKAGES: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestError {
    EmptyName,
    NameTooLong,
    InvalidName,
    InvalidVersion,
    InvalidRange,
    InvalidEncoding,
    UnsupportedKind,
    InvalidTarget,
    TooManyDependencies,
    DuplicateDependency,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PackageName {
    bytes: [u8; MAX_PACKAGE_NAME_BYTES],
    length: u8,
}

impl PackageName {
    pub fn parse(input: &[u8]) -> Result<Self, ManifestError> {
        if input.is_empty() {
            return Err(ManifestError::EmptyName);
        }
        if input.len() > MAX_PACKAGE_NAME_BYTES {
            return Err(ManifestError::NameTooLong);
        }
        if input[0] == b'-' || input[input.len() - 1] == b'-' {
            return Err(ManifestError::InvalidName);
        }
        if input.iter().any(|byte| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-')) {
            return Err(ManifestError::InvalidName);
        }
        let mut bytes = [0; MAX_PACKAGE_NAME_BYTES];
        bytes[..input.len()].copy_from_slice(input);
        Ok(Self { bytes, length: input.len() as u8 })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.length as usize]
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SemanticVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl SemanticVersion {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch }
    }

    pub fn parse(input: &[u8]) -> Result<Self, ManifestError> {
        let first =
            input.iter().position(|byte| *byte == b'.').ok_or(ManifestError::InvalidVersion)?;
        let second = input[first + 1..]
            .iter()
            .position(|byte| *byte == b'.')
            .map(|offset| first + 1 + offset)
            .ok_or(ManifestError::InvalidVersion)?;
        if input[second + 1..].contains(&b'.') {
            return Err(ManifestError::InvalidVersion);
        }
        Ok(Self {
            major: parse_number(&input[..first])?,
            minor: parse_number(&input[first + 1..second])?,
            patch: parse_number(&input[second + 1..])?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VersionRange {
    Any,
    Major(u32),
    Minor(u32, u32),
    Exact(SemanticVersion),
    Caret(SemanticVersion),
    Tilde(SemanticVersion),
    Gte(SemanticVersion),
    Gt(SemanticVersion),
    Lte(SemanticVersion),
    Lt(SemanticVersion),
}

impl VersionRange {
    pub fn parse(input: &[u8]) -> Result<Self, ManifestError> {
        if input == b"*" || input == b"x" || input == b"X" {
            return Ok(Self::Any);
        }
        if let Some(version) = input.strip_prefix(b"^") {
            return Ok(Self::Caret(SemanticVersion::parse(version)?));
        }
        if let Some(version) = input.strip_prefix(b"~") {
            return Ok(Self::Tilde(SemanticVersion::parse(version)?));
        }
        if let Some(version) = input.strip_prefix(b">=") {
            return Ok(Self::Gte(SemanticVersion::parse(version)?));
        }
        if let Some(version) = input.strip_prefix(b">") {
            return Ok(Self::Gt(SemanticVersion::parse(version)?));
        }
        if let Some(version) = input.strip_prefix(b"<=") {
            return Ok(Self::Lte(SemanticVersion::parse(version)?));
        }
        if let Some(version) = input.strip_prefix(b"<") {
            return Ok(Self::Lt(SemanticVersion::parse(version)?));
        }

        let first = input.iter().position(|byte| *byte == b'.');
        let Some(first) = first else {
            return Ok(Self::Major(parse_number(input)?));
        };
        let first_part = &input[..first];
        let rest = &input[first + 1..];
        if is_wildcard(rest) {
            return Ok(Self::Major(parse_number(first_part)?));
        }
        let Some(second_offset) = rest.iter().position(|byte| *byte == b'.') else {
            return Ok(Self::Minor(parse_number(first_part)?, parse_number(rest)?));
        };
        if rest[second_offset + 1..].contains(&b'.') {
            return Err(ManifestError::InvalidRange);
        }
        let second_part = &rest[..second_offset];
        let third_part = &rest[second_offset + 1..];
        if is_wildcard(third_part) {
            return Ok(Self::Minor(parse_number(first_part)?, parse_number(second_part)?));
        }
        Ok(Self::Exact(SemanticVersion::parse(input)?))
    }

    pub fn matches(self, version: SemanticVersion) -> bool {
        match self {
            Self::Any => true,
            Self::Major(major) => version.major == major,
            Self::Minor(major, minor) => version.major == major && version.minor == minor,
            Self::Exact(required) => version == required,
            Self::Caret(required) => version >= required && version < caret_upper_bound(required),
            Self::Tilde(required) => version >= required && version < tilde_upper_bound(required),
            Self::Gte(required) => version >= required,
            Self::Gt(required) => version > required,
            Self::Lte(required) => version <= required,
            Self::Lt(required) => version < required,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageKind {
    Service,
    Program,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageTarget {
    None,
    Service(ServiceId),
}

impl PackageKind {
    pub const fn encoded(self) -> u8 {
        match self {
            Self::Service => PACKAGE_KIND_SERVICE,
            Self::Program => PACKAGE_KIND_PROGRAM,
        }
    }

    pub const fn decode(value: u8) -> Result<Self, ManifestError> {
        match value {
            PACKAGE_KIND_SERVICE => Ok(Self::Service),
            PACKAGE_KIND_PROGRAM => Ok(Self::Program),
            _ => Err(ManifestError::UnsupportedKind),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PackageDependency {
    pub name: PackageName,
    range: [u8; MAX_VERSION_RANGE_BYTES],
    range_length: u8,
}

impl PackageDependency {
    pub fn new(name: PackageName, range: &[u8]) -> Result<Self, ManifestError> {
        if range.is_empty() || range.len() > MAX_VERSION_RANGE_BYTES {
            return Err(ManifestError::InvalidRange);
        }
        VersionRange::parse(range)?;
        let mut encoded = [0; MAX_VERSION_RANGE_BYTES];
        encoded[..range.len()].copy_from_slice(range);
        Ok(Self { name, range: encoded, range_length: range.len() as u8 })
    }

    pub fn range(&self) -> &[u8] {
        &self.range[..self.range_length as usize]
    }

    pub fn matches(&self, version: SemanticVersion) -> Result<bool, ManifestError> {
        Ok(VersionRange::parse(self.range())?.matches(version))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageManifest {
    pub name: PackageName,
    pub version: SemanticVersion,
    pub kind: PackageKind,
    pub target: PackageTarget,
    dependencies: [PackageDependency; MAX_PACKAGE_DEPENDENCIES],
    dependency_count: u8,
}

impl PackageManifest {
    pub const fn new(name: PackageName, version: SemanticVersion, kind: PackageKind) -> Self {
        Self {
            name,
            version,
            kind,
            target: PackageTarget::None,
            dependencies: [PackageDependency {
                name: PackageName { bytes: [0; MAX_PACKAGE_NAME_BYTES], length: 0 },
                range: [0; MAX_VERSION_RANGE_BYTES],
                range_length: 0,
            }; MAX_PACKAGE_DEPENDENCIES],
            dependency_count: 0,
        }
    }

    pub const fn for_service(
        name: PackageName,
        version: SemanticVersion,
        service: ServiceId,
    ) -> Self {
        let mut manifest = Self::new(name, version, PackageKind::Service);
        manifest.target = PackageTarget::Service(service);
        manifest
    }

    pub fn add_dependency(&mut self, dependency: PackageDependency) -> Result<(), ManifestError> {
        if self.dependencies[..self.dependency_count as usize]
            .iter()
            .any(|existing| existing.name == dependency.name)
        {
            return Err(ManifestError::DuplicateDependency);
        }
        let slot = self.dependency_count as usize;
        if slot == MAX_PACKAGE_DEPENDENCIES {
            return Err(ManifestError::TooManyDependencies);
        }
        self.dependencies[slot] = dependency;
        self.dependency_count += 1;
        Ok(())
    }

    pub const fn dependency_count(&self) -> usize {
        self.dependency_count as usize
    }

    pub fn dependency(&self, index: usize) -> Option<&PackageDependency> {
        (index < self.dependency_count as usize).then(|| &self.dependencies[index])
    }

    pub const fn encoded_len() -> usize {
        PACKAGE_MANIFEST_BYTES
    }

    pub fn encode(&self, output: &mut [u8]) -> Result<(), ManifestError> {
        if output.len() < PACKAGE_MANIFEST_BYTES {
            return Err(ManifestError::InvalidEncoding);
        }
        output[..PACKAGE_MANIFEST_BYTES].fill(0);
        output[..self.name.as_bytes().len()].copy_from_slice(self.name.as_bytes());
        output[32] = self.name.as_bytes().len() as u8;
        put_u32(&mut output[..], 36, self.version.major);
        put_u32(&mut output[..], 40, self.version.minor);
        put_u32(&mut output[..], 44, self.version.patch);
        output[48] = self.kind.encoded();
        output[49] = match self.target {
            PackageTarget::None => 0,
            PackageTarget::Service(service) => service as u8,
        };
        output[50] = self.dependency_count;
        for index in 0..self.dependency_count() {
            let dependency = self.dependency(index).expect("dependency count is bounded");
            let offset = 64 + index * 80;
            output[offset..offset + dependency.name.as_bytes().len()]
                .copy_from_slice(dependency.name.as_bytes());
            output[offset + 32] = dependency.name.as_bytes().len() as u8;
            output[offset + 33] = dependency.range_length;
            output[offset + 34..offset + 34 + dependency.range().len()]
                .copy_from_slice(dependency.range());
        }
        Ok(())
    }

    pub fn decode(input: &[u8]) -> Result<Self, ManifestError> {
        if input.len() < PACKAGE_MANIFEST_BYTES {
            return Err(ManifestError::InvalidEncoding);
        }
        let name_length = input[32] as usize;
        if name_length == 0 || name_length > MAX_PACKAGE_NAME_BYTES {
            return Err(ManifestError::InvalidEncoding);
        }
        if input[name_length..32].iter().any(|byte| *byte != 0) {
            return Err(ManifestError::InvalidEncoding);
        }
        let name = PackageName::parse(&input[..name_length])?;
        let kind = PackageKind::decode(input[48])?;
        let target = match input[49] {
            0 => PackageTarget::None,
            raw => PackageTarget::Service(
                raw.checked_sub(1)
                    .and_then(|index| ServiceId::from_index(index as usize))
                    .ok_or(ManifestError::InvalidTarget)?,
            ),
        };
        let dependency_count = input[50] as usize;
        if dependency_count > MAX_PACKAGE_DEPENDENCIES
            || input[51..64].iter().any(|byte| *byte != 0)
        {
            return Err(ManifestError::InvalidEncoding);
        }
        let mut manifest = Self::new(
            name,
            SemanticVersion::new(get_u32(input, 36), get_u32(input, 40), get_u32(input, 44)),
            kind,
        );
        manifest.target = target;
        for index in 0..MAX_PACKAGE_DEPENDENCIES {
            let offset = 64 + index * 80;
            let dependency_name_length = input[offset + 32] as usize;
            let range_length = input[offset + 33] as usize;
            if dependency_name_length > MAX_PACKAGE_NAME_BYTES
                || range_length > MAX_VERSION_RANGE_BYTES
            {
                return Err(ManifestError::InvalidEncoding);
            }
            let reserved = &input[offset + 34 + range_length..offset + 80];
            if range_length == 0 {
                if index < dependency_count
                    || dependency_name_length != 0
                    || reserved.iter().any(|byte| *byte != 0)
                {
                    return Err(ManifestError::InvalidEncoding);
                }
                continue;
            }
            if index >= dependency_count
                || reserved.iter().any(|byte| *byte != 0)
                || input[offset + dependency_name_length..offset + 32].iter().any(|byte| *byte != 0)
            {
                return Err(ManifestError::InvalidEncoding);
            }
            let dependency_name =
                PackageName::parse(&input[offset..offset + dependency_name_length])?;
            let dependency = PackageDependency::new(
                dependency_name,
                &input[offset + 34..offset + 34 + range_length],
            )?;
            manifest.add_dependency(dependency)?;
        }
        Ok(manifest)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageHeaderV2 {
    pub manifest: PackageManifest,
    pub payload_length: u32,
    pub payload_crc32c: u32,
}

impl PackageHeaderV2 {
    pub const fn new(
        manifest: PackageManifest,
        payload_length: usize,
        payload_crc32c: u32,
    ) -> Option<Self> {
        if payload_length == 0 || payload_length > MAX_PACKAGE_PAYLOAD_BYTES {
            return None;
        }
        Some(Self { manifest, payload_length: payload_length as u32, payload_crc32c })
    }

    pub fn encode(&self, output: &mut [u8]) -> Result<(), PackageError> {
        if output.len() < PACKAGE_HEADER_V2_BYTES {
            return Err(PackageError::HeaderTooSmall);
        }
        output[..PACKAGE_HEADER_V2_BYTES].fill(0);
        output[..8].copy_from_slice(&PACKAGE_MAGIC);
        put_u16(output, 8, PACKAGE_FORMAT_VERSION_V2);
        put_u32(output, 12, self.payload_length);
        put_u32(output, 16, self.payload_crc32c);
        self.manifest
            .encode(&mut output[PACKAGE_HEADER_V2_BYTES - PACKAGE_MANIFEST_BYTES..])
            .map_err(PackageError::Manifest)
    }

    pub fn decode(input: &[u8]) -> Result<Self, PackageError> {
        if input.len() < PACKAGE_HEADER_V2_BYTES {
            return Err(PackageError::HeaderTooSmall);
        }
        if input[..8] != PACKAGE_MAGIC {
            return Err(PackageError::InvalidMagic);
        }
        if get_u16(input, 8) != PACKAGE_FORMAT_VERSION_V2 {
            return Err(PackageError::UnsupportedFormat);
        }
        if input[10..12].iter().any(|byte| *byte != 0) {
            return Err(PackageError::Manifest(ManifestError::InvalidEncoding));
        }
        let payload_length = get_u32(input, 12) as usize;
        if payload_length == 0 {
            return Err(PackageError::InvalidPayloadLength);
        }
        if payload_length > MAX_PACKAGE_PAYLOAD_BYTES {
            return Err(PackageError::PayloadTooLarge);
        }
        Ok(Self {
            manifest: PackageManifest::decode(
                &input[PACKAGE_HEADER_V2_BYTES - PACKAGE_MANIFEST_BYTES..PACKAGE_HEADER_V2_BYTES],
            )
            .map_err(PackageError::Manifest)?,
            payload_length: payload_length as u32,
            payload_crc32c: get_u32(input, 16),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogError {
    Capacity,
    Duplicate,
    NotFound,
    NotNewer,
    MissingDependency,
    DependencyConflict,
    DependencyCycle,
    InvalidTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstalledPackage {
    pub manifest: PackageManifest,
}

#[derive(Clone, Copy)]
pub struct PackageCatalog {
    records: [Option<PackageManifest>; MAX_INSTALLED_PACKAGES],
}

impl PackageCatalog {
    pub const fn new() -> Self {
        Self { records: [None; MAX_INSTALLED_PACKAGES] }
    }

    pub fn lookup(&self, name: PackageName) -> Option<InstalledPackage> {
        self.records
            .iter()
            .flatten()
            .find(|manifest| manifest.name == name)
            .copied()
            .map(|manifest| InstalledPackage { manifest })
    }

    pub fn install(&mut self, manifest: PackageManifest) -> Result<InstalledPackage, CatalogError> {
        self.validate_manifest(manifest)?;
        let existing = self.record_index(manifest.name);
        if let Some(index) = existing {
            let current = self.records[index].expect("record index must point to a record");
            if manifest.version <= current.version {
                return Err(CatalogError::NotNewer);
            }
            self.validate_dependents(manifest)?;
            self.records[index] = Some(manifest);
        } else {
            let index =
                self.records.iter().position(Option::is_none).ok_or(CatalogError::Capacity)?;
            self.records[index] = Some(manifest);
        }
        Ok(InstalledPackage { manifest })
    }

    pub const fn len(&self) -> usize {
        let mut count = 0;
        let mut index = 0;
        while index < MAX_INSTALLED_PACKAGES {
            if self.records[index].is_some() {
                count += 1;
            }
            index += 1;
        }
        count
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn activation_order(
        &self,
        root: PackageName,
        output: &mut [PackageName; MAX_INSTALLED_PACKAGES],
    ) -> Result<usize, CatalogError> {
        let mut states = [0u8; MAX_INSTALLED_PACKAGES];
        let mut count = 0;
        self.visit_service(root, &mut states, output, &mut count)?;
        Ok(count)
    }

    fn validate_manifest(&self, manifest: PackageManifest) -> Result<(), CatalogError> {
        match (manifest.kind, manifest.target) {
            (PackageKind::Service, PackageTarget::Service(_))
            | (PackageKind::Program, PackageTarget::None) => {}
            _ => return Err(CatalogError::InvalidTarget),
        }
        for index in 0..manifest.dependency_count() {
            let dependency = manifest.dependency(index).expect("dependency count is bounded");
            if dependency.name == manifest.name {
                return Err(CatalogError::DependencyCycle);
            }
            let Some(installed) = self.lookup(dependency.name) else {
                return Err(CatalogError::MissingDependency);
            };
            if !dependency
                .matches(installed.manifest.version)
                .map_err(|_| CatalogError::DependencyConflict)?
            {
                return Err(CatalogError::DependencyConflict);
            }
        }
        Ok(())
    }

    fn validate_dependents(&self, replacement: PackageManifest) -> Result<(), CatalogError> {
        for installed in self.records.iter().flatten() {
            if installed.name == replacement.name {
                continue;
            }
            for index in 0..installed.dependency_count() {
                let dependency = installed.dependency(index).expect("dependency count is bounded");
                if dependency.name == replacement.name
                    && !dependency
                        .matches(replacement.version)
                        .map_err(|_| CatalogError::DependencyConflict)?
                {
                    return Err(CatalogError::DependencyConflict);
                }
            }
        }
        Ok(())
    }

    fn record_index(&self, name: PackageName) -> Option<usize> {
        self.records.iter().position(|record| record.is_some_and(|manifest| manifest.name == name))
    }

    fn visit_service(
        &self,
        name: PackageName,
        states: &mut [u8; MAX_INSTALLED_PACKAGES],
        output: &mut [PackageName; MAX_INSTALLED_PACKAGES],
        count: &mut usize,
    ) -> Result<(), CatalogError> {
        let index = self.record_index(name).ok_or(CatalogError::NotFound)?;
        let manifest = self.records[index].expect("record index must point to a record");
        if !matches!(manifest.kind, PackageKind::Service) {
            return Ok(());
        }
        match states[index] {
            1 => return Err(CatalogError::DependencyCycle),
            2 => return Ok(()),
            _ => states[index] = 1,
        }
        for dependency_index in 0..manifest.dependency_count() {
            let dependency =
                manifest.dependency(dependency_index).expect("dependency count is bounded");
            let dependency_index =
                self.record_index(dependency.name).ok_or(CatalogError::NotFound)?;
            let dependency_manifest =
                self.records[dependency_index].expect("record index must point to a record");
            if matches!(dependency_manifest.kind, PackageKind::Service) {
                self.visit_service(dependency.name, states, output, count)?;
            }
        }
        if *count == MAX_INSTALLED_PACKAGES {
            return Err(CatalogError::Capacity);
        }
        output[*count] = name;
        *count += 1;
        states[index] = 2;
        Ok(())
    }
}

impl Default for PackageCatalog {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_number(input: &[u8]) -> Result<u32, ManifestError> {
    if input.is_empty() || (input.len() > 1 && input[0] == b'0') {
        return Err(ManifestError::InvalidVersion);
    }
    let mut value = 0u32;
    for byte in input {
        if !byte.is_ascii_digit() {
            return Err(ManifestError::InvalidVersion);
        }
        value = value
            .checked_mul(10)
            .and_then(|value| value.checked_add((byte - b'0') as u32))
            .ok_or(ManifestError::InvalidVersion)?;
    }
    Ok(value)
}

fn is_wildcard(input: &[u8]) -> bool {
    input == b"*" || input == b"x" || input == b"X"
}

fn caret_upper_bound(version: SemanticVersion) -> SemanticVersion {
    if version.major > 0 {
        SemanticVersion::new(version.major.saturating_add(1), 0, 0)
    } else if version.minor > 0 {
        SemanticVersion::new(0, version.minor.saturating_add(1), 0)
    } else {
        SemanticVersion::new(0, 0, version.patch.saturating_add(1))
    }
}

fn tilde_upper_bound(version: SemanticVersion) -> SemanticVersion {
    SemanticVersion::new(version.major, version.minor.saturating_add(1), 0)
}

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
    Manifest(ManifestError),
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

pub fn validate_package_v2<R: PackageReader>(
    reader: &mut R,
    scratch: &mut [u8],
) -> Result<PackageHeaderV2, PackageError> {
    if scratch.is_empty() {
        return Err(PackageError::Reader);
    }
    if reader.len() < PACKAGE_HEADER_V2_BYTES {
        return Err(PackageError::HeaderTooSmall);
    }
    let mut header_bytes = [0u8; PACKAGE_HEADER_V2_BYTES];
    read_exact(reader, 0, &mut header_bytes)?;
    let header = PackageHeaderV2::decode(&header_bytes)?;
    let expected_len = PACKAGE_HEADER_V2_BYTES
        .checked_add(header.payload_length as usize)
        .ok_or(PackageError::PayloadTooLarge)?;
    if reader.len() != expected_len {
        return Err(PackageError::LengthMismatch);
    }
    let mut crc = 0xffff_ffff;
    let mut offset = PACKAGE_HEADER_V2_BYTES;
    while offset < expected_len {
        let amount = (expected_len - offset).min(scratch.len());
        read_exact(reader, offset, &mut scratch[..amount])?;
        crc = crc32c_update(crc, &scratch[..amount]);
        offset += amount;
    }
    if crc32c_finish(crc) != header.payload_crc32c {
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

    #[test]
    fn manifest_names_and_versions_are_bounded() {
        let name = PackageName::parse(b"demo-service").unwrap();
        assert_eq!(name.as_bytes(), b"demo-service");
        assert_eq!(PackageName::parse(b""), Err(ManifestError::EmptyName));
        assert_eq!(PackageName::parse(b"Demo"), Err(ManifestError::InvalidName));
        assert_eq!(SemanticVersion::parse(b"1.2.3").unwrap(), SemanticVersion::new(1, 2, 3));
        assert_eq!(SemanticVersion::parse(b"01.2.3"), Err(ManifestError::InvalidVersion));
    }

    #[test]
    fn npm_style_ranges_match_without_allocation() {
        let version = |value| SemanticVersion::parse(value).unwrap();
        assert!(VersionRange::parse(b"^1.2.3").unwrap().matches(version(b"1.9.0")));
        assert!(!VersionRange::parse(b"^1.2.3").unwrap().matches(version(b"2.0.0")));
        assert!(VersionRange::parse(b"~1.2.3").unwrap().matches(version(b"1.2.9")));
        assert!(!VersionRange::parse(b"~1.2.3").unwrap().matches(version(b"1.3.0")));
        assert!(VersionRange::parse(b"1.x").unwrap().matches(version(b"1.99.0")));
        assert!(VersionRange::parse(b">=2.0.0").unwrap().matches(version(b"2.0.1")));
    }

    #[test]
    fn manifest_dependencies_are_unique_and_bounded() {
        let name = PackageName::parse(b"demo").unwrap();
        let dependency = PackageDependency::new(name, b"^1.0.0").unwrap();
        let mut manifest =
            PackageManifest::new(name, SemanticVersion::new(1, 0, 0), PackageKind::Program);
        manifest.add_dependency(dependency).unwrap();
        assert_eq!(manifest.dependency_count(), 1);
        assert_eq!(manifest.add_dependency(dependency), Err(ManifestError::DuplicateDependency));
        assert!(dependency.matches(SemanticVersion::new(1, 4, 0)).unwrap());
    }

    #[test]
    fn manifest_decode_rejects_oversized_dependency_ranges_without_panicking() {
        let mut bytes = [0; PACKAGE_MANIFEST_BYTES];
        bytes[..4].copy_from_slice(b"demo");
        bytes[32] = 4;
        bytes[48] = PACKAGE_KIND_SERVICE;
        bytes[49] = ServiceId::Flow as u8;
        bytes[50] = 1;
        bytes[64..67].copy_from_slice(b"dep");
        bytes[96] = 3;
        bytes[97] = u8::MAX;
        assert_eq!(PackageManifest::decode(&bytes), Err(ManifestError::InvalidEncoding));
    }

    #[test]
    fn catalog_requires_dependencies_and_updates_atomically() {
        let dep_name = PackageName::parse(b"runtime").unwrap();
        let app_name = PackageName::parse(b"app").unwrap();
        let runtime = PackageManifest::for_service(
            dep_name,
            SemanticVersion::new(1, 0, 0),
            ServiceId::Storage,
        );
        let mut catalog = PackageCatalog::new();
        catalog.install(runtime).unwrap();

        let mut app =
            PackageManifest::for_service(app_name, SemanticVersion::new(1, 0, 0), ServiceId::Flow);
        app.add_dependency(PackageDependency::new(dep_name, b"^1.0.0").unwrap()).unwrap();
        catalog.install(app).unwrap();

        let incompatible = PackageManifest::for_service(
            dep_name,
            SemanticVersion::new(2, 0, 0),
            ServiceId::Storage,
        );
        assert_eq!(catalog.install(incompatible), Err(CatalogError::DependencyConflict));
        assert_eq!(
            catalog.lookup(dep_name).unwrap().manifest.version,
            SemanticVersion::new(1, 0, 0)
        );
    }

    #[test]
    fn activation_order_is_topological_and_skips_program_dependencies() {
        let base_name = PackageName::parse(b"base").unwrap();
        let service_name = PackageName::parse(b"service").unwrap();
        let program_name = PackageName::parse(b"program").unwrap();
        let mut catalog = PackageCatalog::new();
        catalog
            .install(PackageManifest::for_service(
                base_name,
                SemanticVersion::new(1, 0, 0),
                ServiceId::Storage,
            ))
            .unwrap();
        let mut service = PackageManifest::for_service(
            service_name,
            SemanticVersion::new(1, 0, 0),
            ServiceId::Flow,
        );
        service.add_dependency(PackageDependency::new(base_name, b"^1.0.0").unwrap()).unwrap();
        catalog.install(service).unwrap();
        let mut program =
            PackageManifest::new(program_name, SemanticVersion::new(1, 0, 0), PackageKind::Program);
        program.add_dependency(PackageDependency::new(service_name, b"^1.0.0").unwrap()).unwrap();
        catalog.install(program).unwrap();

        let mut order = [PackageName::default(); MAX_INSTALLED_PACKAGES];
        let count = catalog.activation_order(service_name, &mut order).unwrap();
        assert_eq!(&order[..count], &[base_name, service_name]);
    }

    #[test]
    fn v2_header_round_trips_manifest_metadata() {
        let name = PackageName::parse(b"flow-addon").unwrap();
        let dependency_name = PackageName::parse(b"runtime").unwrap();
        let mut manifest =
            PackageManifest::for_service(name, SemanticVersion::new(2, 1, 0), ServiceId::Flow);
        manifest
            .add_dependency(PackageDependency::new(dependency_name, b">=1.2.0").unwrap())
            .unwrap();
        let header = PackageHeaderV2::new(manifest, 3, crc32c(b"elf")).unwrap();
        let mut bytes = vec![0; PACKAGE_HEADER_V2_BYTES];
        header.encode(&mut bytes).unwrap();
        assert_eq!(PackageHeaderV2::decode(&bytes).unwrap(), header);
    }

    #[test]
    fn v2_stream_validation_checks_payload_crc() {
        let name = PackageName::parse(b"streamed").unwrap();
        let manifest =
            PackageManifest::for_service(name, SemanticVersion::new(1, 0, 0), ServiceId::Flow);
        let payload = b"elf-payload";
        let header = PackageHeaderV2::new(manifest, payload.len(), crc32c(payload)).unwrap();
        let mut bytes = vec![0; PACKAGE_HEADER_V2_BYTES + payload.len()];
        header.encode(&mut bytes).unwrap();
        bytes[PACKAGE_HEADER_V2_BYTES..].copy_from_slice(payload);
        let mut reader = Reader(bytes.clone());
        let mut scratch = [0; 3];
        assert_eq!(validate_package_v2(&mut reader, &mut scratch).unwrap(), header);
        bytes[PACKAGE_HEADER_V2_BYTES] ^= 1;
        let mut reader = Reader(bytes);
        assert_eq!(validate_package_v2(&mut reader, &mut scratch), Err(PackageError::CrcMismatch));
    }
}
