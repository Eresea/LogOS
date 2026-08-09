use logos_abi::endpoint_v5::MAX_ENDPOINT_SLOTS;

pub const MAX_MANIFEST_ENTRIES: usize = 8;
const ENTRY_BYTES: usize = 24;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManifestEntry {
    pub service_id: u32,
    pub abi: u16,
    pub version: u16,
    pub capabilities: u64,
    pub endpoint_count: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestError {
    Truncated,
    Full,
    Invalid,
}

pub struct ServiceManifest {
    entries: [Option<ManifestEntry>; MAX_MANIFEST_ENTRIES],
    count: usize,
}

impl ServiceManifest {
    pub const fn new() -> Self {
        Self { entries: [None; MAX_MANIFEST_ENTRIES], count: 0 }
    }

    pub fn add(&mut self, entry: ManifestEntry) -> Result<(), ManifestError> {
        if entry.service_id == 0
            || entry.abi == 0
            || entry.version == 0
            || usize::from(entry.endpoint_count) > MAX_ENDPOINT_SLOTS
        {
            return Err(ManifestError::Invalid);
        }
        if self.entries[..self.count]
            .iter()
            .flatten()
            .any(|current| current.service_id == entry.service_id)
        {
            return Err(ManifestError::Invalid);
        }
        let Some(slot) = self.entries.get_mut(self.count) else { return Err(ManifestError::Full) };
        *slot = Some(entry);
        self.count += 1;
        Ok(())
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestError> {
        if !bytes.len().is_multiple_of(ENTRY_BYTES) {
            return Err(ManifestError::Truncated);
        }
        let mut manifest = Self::new();
        for record in bytes.chunks_exact(ENTRY_BYTES) {
            manifest.add(ManifestEntry {
                service_id: u32::from_le_bytes(record[0..4].try_into().unwrap()),
                abi: u16::from_le_bytes(record[4..6].try_into().unwrap()),
                version: u16::from_le_bytes(record[6..8].try_into().unwrap()),
                capabilities: u64::from_le_bytes(record[8..16].try_into().unwrap()),
                endpoint_count: u16::from_le_bytes(record[16..18].try_into().unwrap()),
            })?;
        }
        Ok(manifest)
    }

    pub fn get(&self, service_id: u32) -> Option<ManifestEntry> {
        self.entries[..self.count]
            .iter()
            .flatten()
            .copied()
            .find(|entry| entry.service_id == service_id)
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }
}

impl Default for ServiceManifest {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bounded_binary_manifest_entries() {
        let mut bytes = [0u8; ENTRY_BYTES];
        bytes[0..4].copy_from_slice(&7u32.to_le_bytes());
        bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
        bytes[6..8].copy_from_slice(&2u16.to_le_bytes());
        bytes[8..16].copy_from_slice(&9u64.to_le_bytes());
        bytes[16..18].copy_from_slice(&3u16.to_le_bytes());
        let manifest = ServiceManifest::parse(&bytes).unwrap();
        assert_eq!(manifest.len(), 1);
        assert_eq!(manifest.get(7).unwrap().capabilities, 9);
    }

    #[test]
    fn rejects_duplicate_and_oversized_manifest_entries() {
        let mut manifest = ServiceManifest::new();
        let entry =
            ManifestEntry { service_id: 1, abi: 1, version: 1, capabilities: 0, endpoint_count: 1 };
        assert_eq!(manifest.add(entry), Ok(()));
        assert_eq!(manifest.add(entry), Err(ManifestError::Invalid));
        assert_eq!(
            manifest.add(ManifestEntry { endpoint_count: 17, ..entry }),
            Err(ManifestError::Invalid)
        );
    }
}
