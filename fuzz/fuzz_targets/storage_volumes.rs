#![no_main]

//! Format a fresh volume, then let the input patch its blocks before opening,
//! recovering and transacting on it.

use libfuzzer_sys::fuzz_target;
use logos_storage::*;

struct VecStore(Vec<Block>);

impl BlockStore for VecStore {
    fn block_count(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_block(&mut self, index: BlockIndex, output: &mut Block) -> Result<(), BlockError> {
        *output = *self.0.get(index.get() as usize).ok_or(BlockError::OutOfBounds)?;
        Ok(())
    }

    fn write_block(&mut self, index: BlockIndex, input: &Block) -> Result<(), BlockError> {
        *self.0.get_mut(index.get() as usize).ok_or(BlockError::OutOfBounds)? = *input;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        Ok(())
    }
}

struct Sink;

impl ReplaySink for Sink {
    fn record(&mut self, _: u64, _: u16, _: &[u8]) -> Result<(), ReplayError> {
        Ok(())
    }
}

/// Apply `(block, offset, len, bytes...)` patches taken from the input.
fn patch(store: &mut VecStore, mut data: &[u8]) {
    while data.len() >= 4 {
        let block = data[0] as usize % store.0.len();
        let offset = u16::from_le_bytes([data[1], data[2]]) as usize % BLOCK_BYTES;
        let len = (data[3] as usize).min(data.len() - 4).min(BLOCK_BYTES - offset);
        store.0[block].as_bytes_mut()[offset..offset + len].copy_from_slice(&data[4..4 + len]);
        data = &data[4 + len..];
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&selector, data)) = data.split_first() else { return };
    match selector % 3 {
        0 => {
            let mut store = VecStore(vec![Block::zero(); 160]);
            let versions = [LEGACY_FORMAT_VERSION, V2_FORMAT_VERSION, FORMAT_VERSION];
            let mut volume =
                Volume::format_as(&mut store, versions[(selector / 3) as usize % 3]).unwrap();
            let _ = volume.commit(&mut store, &[JournalRecord { kind: 1, payload: b"seed" }]);
            patch(&mut store, data);
            if let Ok(mut volume) = Volume::open(&mut store) {
                let _ = volume.recover(&mut store, &mut Sink);
                let mut out = vec![0u8; CHECKPOINT_PAYLOAD_BYTES];
                let _ = volume.load_checkpoint(&mut store, &mut out);
                let _ = volume.commit(&mut store, &[JournalRecord { kind: 2, payload: b"x" }]);
                let _ = volume.checkpoint(&mut store, b"cp");
            }
        }
        1 => {
            let mut store = VecStore(vec![Block::zero(); 128]);
            let mut volume = CowVolume::format(&mut store).unwrap();
            let mut transaction = volume.begin(&mut store).unwrap();
            let metadata = transaction.allocate_blocks(&mut store, 1).unwrap();
            transaction.write_block(&mut store, metadata.start, &Block::zero()).unwrap();
            volume.commit(&mut store, transaction, metadata).unwrap();
            patch(&mut store, data);
            if let Ok(mut volume) = CowVolume::open(&mut store) {
                let mut metadata = vec![Block::zero(); volume.root().metadata_blocks as usize];
                let _ = volume.read_metadata(&mut store, &mut metadata);
                if let Ok(mut transaction) = volume.begin(&mut store) {
                    if let Ok(extent) = transaction.allocate_blocks(&mut store, 1) {
                        let _ = transaction.write_block(&mut store, extent.start, &Block::zero());
                        let _ = volume.commit(&mut store, transaction, extent);
                    }
                }
            }
        }
        _ => {
            let mut store = VecStore(vec![Block::zero(); 128]);
            let mut volume = SystemCatalogVolume::format(&mut store, 32, 64).unwrap();
            volume.replace_catalog(&mut store, b"catalog").unwrap();
            volume.replace_metadata(&mut store, b"metadata").unwrap();
            patch(&mut store, data);
            if let Ok(mut volume) = SystemCatalogVolume::open(&mut store) {
                let mut out = vec![0u8; SYSTEM_CATALOG_MAX_BYTES];
                let _ = volume.read_catalog(&mut store, &mut out);
                let _ = volume.read_metadata(&mut store, &mut out);
                let _ = volume.begin(&mut store);
                let _ = volume.replace_catalog(&mut store, b"again");
            }
        }
    }
});
