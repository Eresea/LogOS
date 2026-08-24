extern crate std;

use std::{
    env,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    process,
};

use logos_abi::ServiceId;
use logos_package::{
    PACKAGE_HEADER_V2_BYTES, PackageHeaderV2, PackageKind, PackageManifest, PackageName,
    SemanticVersion, crc32c,
};
use logos_storage::{BLOCK_BYTES, Block, BlockError, BlockIndex, BlockStore};
use logos_storage_service::{DurableNamespaceV5, PackageHandle, PackageInfo};

const DISK_BLOCKS: u64 = 16 * 1024;

struct FileBlockStore {
    file: File,
    blocks: u64,
}

impl FileBlockStore {
    fn create(path: &Path) -> std::io::Result<Self> {
        let file =
            OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path)?;
        file.set_len(DISK_BLOCKS * BLOCK_BYTES as u64)?;
        Ok(Self { file, blocks: DISK_BLOCKS })
    }

    fn seek_block(&mut self, index: BlockIndex) -> Result<(), BlockError> {
        if index.get() >= self.blocks {
            return Err(BlockError::OutOfBounds);
        }
        self.file
            .seek(SeekFrom::Start(index.get() * BLOCK_BYTES as u64))
            .map(|_| ())
            .map_err(|_| BlockError::Io)
    }
}

impl BlockStore for FileBlockStore {
    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_block(&mut self, index: BlockIndex, output: &mut Block) -> Result<(), BlockError> {
        self.seek_block(index)?;
        self.file.read_exact(output.as_bytes_mut()).map_err(|_| BlockError::Io)
    }

    fn write_block(&mut self, index: BlockIndex, input: &Block) -> Result<(), BlockError> {
        self.seek_block(index)?;
        self.file.write_all(input.as_bytes()).map_err(|_| BlockError::Io)
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        self.file.sync_all().map_err(|_| BlockError::Io)
    }
}

fn package(service: ServiceId, payload: &[u8]) -> Vec<u8> {
    let name = match service {
        ServiceId::Input => b"input".as_slice(),
        ServiceId::Display => b"display".as_slice(),
        ServiceId::Session => b"session".as_slice(),
        _ => b"service".as_slice(),
    };
    let manifest = PackageManifest::for_service(
        PackageName::parse(name).expect("package name"),
        SemanticVersion::new(1, 0, 0),
        service,
    );
    let header =
        PackageHeaderV2::new(manifest, payload.len(), crc32c(payload)).expect("package size");
    let mut bytes = vec![0; PACKAGE_HEADER_V2_BYTES + payload.len()];
    header.encode(&mut bytes).expect("package header");
    bytes[PACKAGE_HEADER_V2_BYTES..].copy_from_slice(payload);
    bytes
}

fn program(payload: &[u8]) -> Vec<u8> {
    let manifest = PackageManifest::new(
        PackageName::parse(b"demo").expect("package name"),
        SemanticVersion::new(1, 0, 0),
        PackageKind::Program,
    );
    let header =
        PackageHeaderV2::new(manifest, payload.len(), crc32c(payload)).expect("package size");
    let mut bytes = vec![0; PACKAGE_HEADER_V2_BYTES + payload.len()];
    header.encode(&mut bytes).expect("package header");
    bytes[PACKAGE_HEADER_V2_BYTES..].copy_from_slice(payload);
    bytes
}

fn install(
    filesystem: &mut DurableNamespaceV5<FileBlockStore>,
    service: ServiceId,
    bytes: &[u8],
) -> logos_storage_service::PackageInfo {
    let mut install =
        filesystem.begin_package_install(service, bytes.len()).expect("begin package");
    for (offset, chunk) in bytes.chunks(BLOCK_BYTES).enumerate() {
        filesystem
            .write_package_chunk(&mut install, offset * BLOCK_BYTES, chunk)
            .expect("write package");
    }
    filesystem.commit_package_install(install).expect("commit package");
    filesystem.lookup_package(service).expect("package catalog")
}

fn install_program(
    filesystem: &mut DurableNamespaceV5<FileBlockStore>,
    bytes: &[u8],
) -> logos_storage_service::PackageInfo {
    let name = PackageName::parse(b"demo").expect("package name");
    let mut install = filesystem
        .begin_package_install_target(logos_storage_service::PackageKey::Program(name), bytes.len())
        .expect("begin program package");
    for (offset, chunk) in bytes.chunks(BLOCK_BYTES).enumerate() {
        filesystem
            .write_package_chunk(&mut install, offset * BLOCK_BYTES, chunk)
            .expect("write program package");
    }
    filesystem.commit_package_install(install).expect("commit program package");
    filesystem.lookup_package_name(b"demo").expect("program catalog")
}

fn corrupt(filesystem: &mut DurableNamespaceV5<FileBlockStore>, info: PackageInfo) {
    let extent = info.extents[0];
    let mut block = Block::zero();
    let store = filesystem.block_store_mut();
    store.read_block(BlockIndex::new(extent.start), &mut block).expect("read corrupt target");
    // Corrupt the header so activation rejects before streaming the whole
    // payload. The proof targets rollback, not package transport duration.
    block.as_bytes_mut()[0] ^= 0xa5;
    store.write_block(BlockIndex::new(extent.start), &block).expect("write corrupt target");
    store.flush().expect("flush corrupt target");
}

fn main() {
    let mut arguments = env::args().skip(1);
    let disk = arguments.next().unwrap_or_else(|| {
        eprintln!("usage: package-seed <disk> <service-elf>");
        process::exit(2);
    });
    let elf = arguments.next().unwrap_or_else(|| {
        eprintln!("usage: package-seed <disk> <service-elf>");
        process::exit(2);
    });
    let payload = std::fs::read(&elf).expect("read service ELF");
    if payload.len() <= 8 * 1024 {
        eprintln!("service ELF must exceed the ordinary-file limit");
        process::exit(1);
    }
    let mut filesystem = DurableNamespaceV5::format_v5(
        FileBlockStore::create(Path::new(&disk)).expect("create disk"),
    )
    .expect("format v5 disk");
    let input = package(ServiceId::Input, &payload);
    let input_info = install(&mut filesystem, ServiceId::Input, &input);
    let session = package(ServiceId::Session, &payload);
    install(&mut filesystem, ServiceId::Session, &session);
    let demo = program(&payload);
    let _ = install_program(&mut filesystem, &demo);
    let mut round_trip = vec![0; input.len()];
    let bytes_read = filesystem
        .read_package(
            PackageHandle {
                target: logos_storage_service::PackageKey::Service(ServiceId::Input),
                generation: input_info.handle.generation,
            },
            0,
            &mut round_trip,
        )
        .expect("read seeded package");
    assert_eq!(bytes_read, input.len());
    assert_eq!(&round_trip, &input);
    let display = package(ServiceId::Display, &payload);
    let display_info = install(&mut filesystem, ServiceId::Display, &display);
    corrupt(&mut filesystem, display_info);
    println!(
        "seeded v3 service and program packages ({} bytes) and corrupt rollback fixture",
        input.len()
    );
}
