//! Property tests for the on-disk formats (journal volume, copy-on-write
//! volume, system catalog). Valid volumes round-trip through reopen/recovery;
//! volumes with arbitrary or mutated blocks must open, recover, read and
//! begin transactions without panicking. Skipped under Miri (too slow).
#![cfg(not(miri))]

use logos_storage::*;
use proptest::prelude::*;

#[derive(Clone)]
struct VecStore {
    blocks: Vec<Block>,
}

impl VecStore {
    fn new(blocks: usize) -> Self {
        Self { blocks: vec![Block::zero(); blocks] }
    }

    fn apply(&mut self, mutations: &[Mutation]) {
        let count = self.blocks.len();
        for mutation in mutations {
            let block = &mut self.blocks[mutation.block % count];
            match &mutation.kind {
                MutationKind::Flip { offset, value } => {
                    block.as_bytes_mut()[offset % BLOCK_BYTES] ^= value | 1;
                }
                MutationKind::Set { offset, value } => {
                    block.as_bytes_mut()[offset % BLOCK_BYTES] = *value;
                }
                MutationKind::Header(bytes) => {
                    block.as_bytes_mut()[..bytes.len()].copy_from_slice(bytes);
                }
                MutationKind::Garbage(bytes) => {
                    for (index, byte) in block.as_bytes_mut().iter_mut().enumerate() {
                        *byte = bytes[index % bytes.len()];
                    }
                }
                MutationKind::Zero => *block = Block::zero(),
            }
        }
    }
}

impl BlockStore for VecStore {
    fn block_count(&self) -> u64 {
        self.blocks.len() as u64
    }

    fn read_block(&mut self, index: BlockIndex, output: &mut Block) -> Result<(), BlockError> {
        *output = *self.blocks.get(index.get() as usize).ok_or(BlockError::OutOfBounds)?;
        Ok(())
    }

    fn write_block(&mut self, index: BlockIndex, input: &Block) -> Result<(), BlockError> {
        *self.blocks.get_mut(index.get() as usize).ok_or(BlockError::OutOfBounds)? = *input;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct Mutation {
    block: usize,
    kind: MutationKind,
}

#[derive(Clone, Debug)]
enum MutationKind {
    Flip { offset: usize, value: u8 },
    Set { offset: usize, value: u8 },
    Header(Vec<u8>),
    Garbage(Vec<u8>),
    Zero,
}

/// Mutations biased toward the first blocks (superblocks, commit slots,
/// bitmaps) where the format metadata lives, with some anywhere.
fn mutations() -> impl Strategy<Value = Vec<Mutation>> {
    let block = prop_oneof![3 => 0usize..24, 1 => any::<usize>()];
    let kind = prop_oneof![
        4 => (0usize..128, any::<u8>()).prop_map(|(offset, value)| MutationKind::Flip { offset, value }),
        2 => (0usize..BLOCK_BYTES, any::<u8>()).prop_map(|(offset, value)| MutationKind::Set { offset, value }),
        2 => proptest::collection::vec(any::<u8>(), 1..96).prop_map(MutationKind::Header),
        1 => proptest::collection::vec(any::<u8>(), 1..32).prop_map(MutationKind::Garbage),
        1 => Just(MutationKind::Zero),
    ];
    proptest::collection::vec(
        (block, kind).prop_map(|(block, kind)| Mutation { block, kind }),
        1..8,
    )
}

fn garbage_store(blocks: usize, seeds: &[Vec<u8>]) -> VecStore {
    let mut store = VecStore::new(blocks);
    for (index, block) in store.blocks.iter_mut().enumerate() {
        let seed = &seeds[index % seeds.len()];
        for (offset, byte) in block.as_bytes_mut().iter_mut().enumerate() {
            *byte = seed[offset % seed.len()].wrapping_add((offset / seed.len()) as u8);
        }
    }
    store
}

// ---------------------------------------------------------------- journal

#[derive(Default)]
struct CollectSink(Vec<(u64, u16, Vec<u8>)>);

impl ReplaySink for CollectSink {
    fn record(
        &mut self,
        transaction_id: u64,
        kind: u16,
        payload: &[u8],
    ) -> Result<(), ReplayError> {
        self.0.push((transaction_id, kind, payload.to_vec()));
        Ok(())
    }
}

fn record_strategy() -> impl Strategy<Value = Vec<(u16, Vec<u8>)>> {
    proptest::collection::vec(
        (
            prop_oneof![0u16..JOURNAL_COMMIT_KIND, Just(0u16), Just(JOURNAL_COMMIT_KIND - 1)],
            proptest::collection::vec(any::<u8>(), 0..300),
        ),
        0..=MAX_RECORDS_PER_TRANSACTION.min(4),
    )
}

const JOURNAL_BLOCKS: usize = 160;

/// Exercise every public read/write entry point on a possibly-corrupt volume.
fn exercise_journal(store: &mut VecStore) {
    let Ok(mut volume) = Volume::open(store) else { return };
    let _ = volume.info();
    let _ = volume.package_arena();
    let _ = volume.should_checkpoint();
    let mut sink = CollectSink::default();
    let _ = volume.recover(store, &mut sink);
    let mut output = vec![0u8; CHECKPOINT_PAYLOAD_BYTES];
    let _ = volume.load_checkpoint(store, &mut output);
    let _ = volume.commit(store, &[JournalRecord { kind: 1, payload: b"x" }]);
    let _ = volume.checkpoint(store, b"checkpoint");
    let _ = volume.load_checkpoint(store, &mut output);
    let _ = Volume::open(store);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn journal_commits_recover_exactly(
        version in prop_oneof![Just(LEGACY_FORMAT_VERSION), Just(V2_FORMAT_VERSION), Just(FORMAT_VERSION)],
        transactions in proptest::collection::vec(record_strategy(), 0..5),
    ) {
        let mut store = VecStore::new(JOURNAL_BLOCKS);
        let mut volume = Volume::format_as(&mut store, version).unwrap();
        let mut expected = Vec::new();
        for records in &transactions {
            let batch: Vec<JournalRecord<'_>> = records
                .iter()
                .map(|(kind, payload)| JournalRecord { kind: *kind, payload })
                .collect();
            let Ok(id) = volume.commit(&mut store, &batch) else { continue };
            for (kind, payload) in records {
                expected.push((id, *kind, payload.clone()));
            }
        }
        let mut reopened = Volume::open(&mut store).unwrap();
        prop_assert_eq!(reopened.info(), volume.info());
        let mut sink = CollectSink::default();
        reopened.recover(&mut store, &mut sink).unwrap();
        prop_assert_eq!(sink.0, expected);
    }

    #[test]
    fn journal_mutated_volume_never_panics(
        version in prop_oneof![Just(LEGACY_FORMAT_VERSION), Just(V2_FORMAT_VERSION), Just(FORMAT_VERSION)],
        transactions in proptest::collection::vec(record_strategy(), 0..4),
        mutations in mutations(),
    ) {
        let mut store = VecStore::new(JOURNAL_BLOCKS);
        let mut volume = Volume::format_as(&mut store, version).unwrap();
        for records in &transactions {
            let batch: Vec<JournalRecord<'_>> = records
                .iter()
                .map(|(kind, payload)| JournalRecord { kind: *kind, payload })
                .collect();
            let _ = volume.commit(&mut store, &batch);
        }
        store.apply(&mutations);
        exercise_journal(&mut store);
    }

    #[test]
    fn journal_garbage_volume_never_panics(
        seeds in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 1..64), 1..4),
        blocks in 1usize..JOURNAL_BLOCKS,
    ) {
        let mut store = garbage_store(blocks, &seeds);
        exercise_journal(&mut store);
    }
}

// -------------------------------------------------------------------- cow

const COW_BLOCKS: usize = 128;

fn exercise_cow(store: &mut VecStore) {
    let Ok(mut volume) = CowVolume::open(store) else { return };
    let _ = volume.root();
    let _ = volume.data_arena();
    let _ = volume.package_arena();
    let mut root = Block::zero();
    let _ = volume.read_metadata_root(store, &mut root);
    let mut metadata = vec![Block::zero(); volume.root().metadata_blocks as usize];
    let _ = volume.read_metadata(store, &mut metadata);
    let _ = volume.read_metadata_block(store, 0, &mut root);
    if let Ok(mut transaction) = volume.begin(store) {
        if let Ok(extent) = transaction.allocate_blocks(store, 1) {
            let _ = transaction.write_block(store, extent.start, &Block::zero());
            let _ = volume.commit(store, transaction, extent);
        }
    }
    let _ = CowVolume::open(store);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn cow_commits_survive_reopen(
        payloads in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 1..64), 0..5),
    ) {
        let mut store = VecStore::new(COW_BLOCKS);
        let mut volume = CowVolume::format(&mut store).unwrap();
        let mut last = None;
        for payload in &payloads {
            let mut transaction = volume.begin(&mut store).unwrap();
            let metadata = transaction.allocate_blocks(&mut store, 1).unwrap();
            let mut block = Block::zero();
            block.as_bytes_mut()[..payload.len()].copy_from_slice(payload);
            transaction.write_block(&mut store, metadata.start, &block).unwrap();
            if volume.root().metadata_blocks != 0 {
                transaction
                    .retire_extent(CowExtent::new(volume.root().metadata_root, volume.root().metadata_blocks).unwrap())
                    .unwrap();
            }
            volume.commit(&mut store, transaction, metadata).unwrap();
            last = Some(block);
        }
        let reopened = CowVolume::open(&mut store).unwrap();
        prop_assert_eq!(reopened.root(), volume.root());
        if let Some(block) = last {
            let mut read = Block::zero();
            reopened.read_metadata_root(&mut store, &mut read).unwrap();
            prop_assert_eq!(read, block);
        }
    }

    #[test]
    fn cow_mutated_volume_never_panics(
        commits in 0usize..4,
        mutations in mutations(),
    ) {
        let mut store = VecStore::new(COW_BLOCKS);
        let mut volume = CowVolume::format(&mut store).unwrap();
        for _ in 0..commits {
            let mut transaction = volume.begin(&mut store).unwrap();
            let metadata = transaction.allocate_blocks(&mut store, 1).unwrap();
            transaction.write_block(&mut store, metadata.start, &Block::zero()).unwrap();
            volume.commit(&mut store, transaction, metadata).unwrap();
        }
        store.apply(&mutations);
        exercise_cow(&mut store);
    }

    #[test]
    fn cow_garbage_volume_never_panics(
        seeds in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 1..64), 1..4),
        blocks in 1usize..COW_BLOCKS,
    ) {
        let mut store = garbage_store(blocks, &seeds);
        exercise_cow(&mut store);
    }
}

// ---------------------------------------------------------------- catalog

const CATALOG_BLOCKS: usize = 128;

fn exercise_catalog(store: &mut VecStore) {
    let Ok(mut volume) = SystemCatalogVolume::open(store) else { return };
    let _ = volume.root();
    let _ = volume.system_arena();
    let _ = volume.user_arena();
    let _ = volume.package_arena();
    let mut output = vec![0u8; SYSTEM_CATALOG_MAX_BYTES];
    let _ = volume.read_catalog(store, &mut output);
    let _ = volume.read_metadata(store, &mut output);
    let _ = volume.begin(store);
    let _ = volume.replace_catalog(store, b"catalog");
    let _ = volume.replace_metadata(store, b"metadata");
    let _ = SystemCatalogVolume::open(store);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn catalog_replace_round_trips(
        catalogs in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 1..6000), 1..4),
        metadata in proptest::collection::vec(any::<u8>(), 1..6000),
    ) {
        let mut store = VecStore::new(CATALOG_BLOCKS);
        let mut volume = SystemCatalogVolume::format(&mut store, 32, 64).unwrap();
        for catalog in &catalogs {
            volume.replace_catalog(&mut store, catalog).unwrap();
        }
        volume.replace_metadata(&mut store, &metadata).unwrap();
        let reopened = SystemCatalogVolume::open(&mut store).unwrap();
        prop_assert_eq!(reopened.root(), volume.root());
        let mut output = vec![0u8; SYSTEM_CATALOG_MAX_BYTES];
        let last = catalogs.last().unwrap();
        prop_assert_eq!(reopened.read_catalog(&mut store, &mut output), Ok(last.len()));
        prop_assert_eq!(&output[..last.len()], &last[..]);
        prop_assert_eq!(reopened.read_metadata(&mut store, &mut output), Ok(metadata.len()));
        prop_assert_eq!(&output[..metadata.len()], &metadata[..]);
    }

    #[test]
    fn catalog_mutated_volume_never_panics(
        catalog in proptest::collection::vec(any::<u8>(), 1..5000),
        mutations in mutations(),
    ) {
        let mut store = VecStore::new(CATALOG_BLOCKS);
        let mut volume = SystemCatalogVolume::format(&mut store, 32, 64).unwrap();
        volume.replace_catalog(&mut store, &catalog).unwrap();
        volume.replace_metadata(&mut store, b"namespace").unwrap();
        store.apply(&mutations);
        exercise_catalog(&mut store);
    }

    #[test]
    fn catalog_garbage_volume_never_panics(
        seeds in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 1..64), 1..4),
        blocks in 1usize..CATALOG_BLOCKS,
    ) {
        let mut store = garbage_store(blocks, &seeds);
        exercise_catalog(&mut store);
    }
}
