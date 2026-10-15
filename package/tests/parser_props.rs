//! Property tests for the package parsers: arbitrary bytes never panic, valid
//! encodings round-trip, and mutated valid encodings either re-validate or
//! fail cleanly. Skipped under Miri (proptest is too slow there).
#![cfg(not(miri))]

use logos_abi::{ABI_VERSION, ServiceId};
use logos_package::*;
use proptest::prelude::*;

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

fn name_strategy() -> impl Strategy<Value = Vec<u8>> {
    "[a-z0-9]([a-z0-9-]{0,30}[a-z0-9])?".prop_map(|s| s.into_bytes())
}

fn version_strategy() -> impl Strategy<Value = SemanticVersion> {
    (any::<u32>(), any::<u32>(), any::<u32>()).prop_map(|(a, b, c)| SemanticVersion::new(a, b, c))
}

fn range_strategy() -> impl Strategy<Value = Vec<u8>> {
    (0u8..9, 0u32..1000, 0u32..1000, 0u32..1000).prop_map(|(kind, a, b, c)| {
        let v = format!("{a}.{b}.{c}");
        match kind {
            0 => "*".to_string(),
            1 => format!("^{v}"),
            2 => format!("~{v}"),
            3 => format!(">={v}"),
            4 => format!(">{v}"),
            5 => format!("<={v}"),
            6 => format!("<{v}"),
            7 => v,
            _ => format!("{a}.{b}"),
        }
        .into_bytes()
    })
}

fn manifest_strategy() -> impl Strategy<Value = PackageManifest> {
    (
        name_strategy(),
        version_strategy(),
        prop_oneof![Just(None), (0usize..14).prop_map(Some)],
        proptest::collection::vec(
            (name_strategy(), range_strategy()),
            0..=MAX_PACKAGE_DEPENDENCIES,
        ),
    )
        .prop_map(|(name, version, service, deps)| {
            let name = PackageName::parse(&name).unwrap();
            let mut manifest = match service.and_then(ServiceId::from_index) {
                Some(id) => PackageManifest::for_service(name, version, id),
                None => PackageManifest::new(name, version, PackageKind::Program),
            };
            for (dep_name, range) in deps {
                let dep_name = PackageName::parse(&dep_name).unwrap();
                if let Ok(dep) = PackageDependency::new(dep_name, &range) {
                    let _ = manifest.add_dependency(dep);
                }
            }
            manifest
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..1024)) {
        let _ = PackageName::parse(&bytes);
        let _ = SemanticVersion::parse(&bytes);
        let _ = VersionRange::parse(&bytes);
        let _ = PackageManifest::decode(&bytes);
        let _ = ServicePackageHeader::decode(&bytes);
        let _ = PackageHeaderV2::decode(&bytes);
        let _ = crc32c(&bytes);
        let mut scratch = [0u8; 16];
        let _ = validate_package(&mut SliceReader(&bytes), ServiceId::Input, ABI_VERSION, &mut scratch);
        let _ = validate_package_v2(&mut SliceReader(&bytes), &mut scratch);
    }

    #[test]
    fn textual_inputs_never_panic(text in "[0-9a-zA-Z.*^~<>=x -]{0,48}") {
        let bytes = text.as_bytes();
        if let Ok(range) = VersionRange::parse(bytes) {
            let _ = range.matches(SemanticVersion::new(1, 2, 3));
            let _ = range.matches(SemanticVersion::new(u32::MAX, u32::MAX, u32::MAX));
        }
        if let (Ok(name), true) = (PackageName::parse(b"dep"), bytes.len() <= MAX_VERSION_RANGE_BYTES) {
            let _ = PackageDependency::new(name, bytes);
        }
    }

    #[test]
    fn semantic_version_text_round_trips(version in version_strategy()) {
        let text = format!("{}.{}.{}", version.major, version.minor, version.patch);
        prop_assert_eq!(SemanticVersion::parse(text.as_bytes()), Ok(version));
    }

    #[test]
    fn manifest_round_trips(manifest in manifest_strategy()) {
        let mut buf = [0u8; PACKAGE_MANIFEST_BYTES];
        manifest.encode(&mut buf).unwrap();
        prop_assert_eq!(PackageManifest::decode(&buf), Ok(manifest));
    }

    #[test]
    fn mutated_manifest_revalidates_or_errors(
        manifest in manifest_strategy(),
        flips in proptest::collection::vec((0usize..PACKAGE_MANIFEST_BYTES, any::<u8>()), 1..6),
    ) {
        let mut buf = [0u8; PACKAGE_MANIFEST_BYTES];
        manifest.encode(&mut buf).unwrap();
        for (index, value) in flips {
            buf[index] = value;
        }
        if let Ok(decoded) = PackageManifest::decode(&buf) {
            let mut again = [0u8; PACKAGE_MANIFEST_BYTES];
            decoded.encode(&mut again).unwrap();
            prop_assert_eq!(PackageManifest::decode(&again), Ok(decoded));
        }
    }

    #[test]
    fn service_header_round_trips_and_mutations_are_clean(
        service in 0usize..14,
        version in any::<u32>(),
        payload in proptest::collection::vec(any::<u8>(), 1..256),
        flips in proptest::collection::vec((0usize..PACKAGE_HEADER_BYTES + 256, any::<u8>()), 0..4),
    ) {
        let Some(service) = ServiceId::from_index(service) else { return Ok(()); };
        let header = ServicePackageHeader::new(service, version, payload.len(), crc32c(&payload)).unwrap();
        let mut image = vec![0u8; PACKAGE_HEADER_BYTES];
        header.encode(&mut image).unwrap();
        image.extend_from_slice(&payload);
        let mut scratch = [0u8; 7];
        prop_assert_eq!(
            validate_package(&mut SliceReader(&image), service, ABI_VERSION, &mut scratch),
            Ok(header)
        );
        for (index, value) in flips {
            let index = index % image.len();
            image[index] = value;
        }
        if let Ok(decoded) =
            validate_package(&mut SliceReader(&image), service, ABI_VERSION, &mut scratch)
        {
            // A package that still validates must be internally consistent.
            prop_assert_eq!(decoded.service, service);
            prop_assert_eq!(image.len(), PACKAGE_HEADER_BYTES + decoded.payload_length as usize);
            prop_assert_eq!(crc32c(&image[PACKAGE_HEADER_BYTES..]), decoded.payload_crc32c);
        }
    }

    #[test]
    fn v2_header_round_trips_and_mutations_are_clean(
        manifest in manifest_strategy(),
        payload in proptest::collection::vec(any::<u8>(), 1..256),
        flips in proptest::collection::vec((0usize..PACKAGE_HEADER_V2_BYTES + 256, any::<u8>()), 0..4),
    ) {
        let header = PackageHeaderV2::new(manifest, payload.len(), crc32c(&payload)).unwrap();
        let mut image = vec![0u8; PACKAGE_HEADER_V2_BYTES];
        header.encode(&mut image).unwrap();
        image.extend_from_slice(&payload);
        let mut scratch = [0u8; 13];
        prop_assert_eq!(validate_package_v2(&mut SliceReader(&image), &mut scratch), Ok(header));
        for (index, value) in flips {
            let index = index % image.len();
            image[index] = value;
        }
        if let Ok(decoded) = validate_package_v2(&mut SliceReader(&image), &mut scratch) {
            prop_assert_eq!(image.len(), PACKAGE_HEADER_V2_BYTES + decoded.payload_length as usize);
            prop_assert_eq!(crc32c(&image[PACKAGE_HEADER_V2_BYTES..]), decoded.payload_crc32c);
            let mut again = vec![0u8; PACKAGE_HEADER_V2_BYTES];
            decoded.encode(&mut again).unwrap();
            prop_assert_eq!(PackageHeaderV2::decode(&again), Ok(decoded));
        }
    }

    #[test]
    fn catalog_install_never_panics(manifests in proptest::collection::vec(manifest_strategy(), 0..24)) {
        let mut catalog = PackageCatalog::new();
        for manifest in &manifests {
            let _ = catalog.install(*manifest);
        }
        for manifest in &manifests {
            let mut out = [PackageName::default(); MAX_INSTALLED_PACKAGES];
            let _ = catalog.activation_order(manifest.name, &mut out);
        }
    }
}
