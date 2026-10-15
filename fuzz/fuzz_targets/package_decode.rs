#![no_main]

use libfuzzer_sys::fuzz_target;
use logos_abi::{ABI_VERSION, ServiceId};
use logos_package::*;

struct SliceReader<'a>(&'a [u8]);

impl PackageReader for SliceReader<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn read(&mut self, offset: usize, output: &mut [u8]) -> Result<usize, PackageError> {
        let end = offset.checked_add(output.len()).ok_or(PackageError::Reader)?;
        let source = self.0.get(offset..end).ok_or(PackageError::Reader)?;
        output.copy_from_slice(source);
        Ok(output.len())
    }
}

fuzz_target!(|data: &[u8]| {
    let _ = PackageName::parse(data);
    let _ = SemanticVersion::parse(data);
    if let Ok(range) = VersionRange::parse(data) {
        let _ = range.matches(SemanticVersion::new(1, 2, 3));
    }
    if let Ok(manifest) = PackageManifest::decode(data) {
        // Anything that decodes must re-encode to something that decodes equal.
        let mut again = [0u8; PACKAGE_MANIFEST_BYTES];
        manifest.encode(&mut again).expect("decoded manifest must encode");
        assert_eq!(PackageManifest::decode(&again), Ok(manifest));
    }
    let _ = ServicePackageHeader::decode(data);
    let _ = PackageHeaderV2::decode(data);
    let mut scratch = [0u8; 64];
    let _ = validate_package(&mut SliceReader(data), ServiceId::Input, ABI_VERSION, &mut scratch);
    let _ = validate_package_v2(&mut SliceReader(data), &mut scratch);
});
