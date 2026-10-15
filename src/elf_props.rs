//! Property tests for the untrusted-ELF path: `ElfLoadPlan::parse`, the
//! frame-backed loader and the page-table builder. Arbitrary bytes never
//! panic; every accepted plan satisfies the documented bounds; a plan that
//! loads can be populated and mapped, and every frame is returned on reclaim.
//!
//! These live inside the crate (not under `tests/`) because the frame pool's
//! host metadata only exists under `cfg(test)` of the library itself.
//! Skipped under Miri (proptest is too slow there).

use crate::{
    boot_resources::{MemoryDescriptor, MemoryMap},
    frame_pool::{FrameAddress, FramePool},
    loader::{LoadError, LoadedImage, PAGE_SIZE, PageSink, USER_STACK_BASE},
    memory::OwnerId,
    page_table::{PageTableBuilder, PageTableError, PageTableMemory},
    process::{ElfLoadPlan, ImageReader, MAX_IMAGE_BYTES, MAX_PROGRAM_HEADERS, ProcessError},
};
use proptest::prelude::*;
use std::{boxed::Box, collections::HashMap, format, vec, vec::Vec};

/// The host frame manager is sized for 256 frames; stay inside it.
const POOL_PAGES: u64 = 200;

fn pool() -> FramePool {
    let mut map = MemoryMap::new();
    map.push(MemoryDescriptor::new(0x10_0000, POOL_PAGES, true).unwrap()).unwrap();
    let mut pool = FramePool::empty();
    pool.initialize(&map).unwrap();
    pool
}

#[derive(Default)]
struct HostPages(HashMap<u64, Box<[u8; PAGE_SIZE]>>);

impl PageSink for HostPages {
    fn clear(&mut self, frame: FrameAddress) -> Result<(), LoadError> {
        self.0.insert(frame.raw(), Box::new([0; PAGE_SIZE]));
        Ok(())
    }

    fn write(&mut self, frame: FrameAddress, offset: usize, bytes: &[u8]) -> Result<(), LoadError> {
        if offset >= PAGE_SIZE || bytes.len() > PAGE_SIZE - offset {
            return Err(LoadError::Write);
        }
        let page = self.0.get_mut(&frame.raw()).ok_or(LoadError::Write)?;
        page[offset..offset + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }
}

#[derive(Default)]
struct HostTables(HashMap<u64, Box<[u64; 512]>>);

impl PageTableMemory for HostTables {
    fn clear(&mut self, frame: FrameAddress) -> Result<(), PageTableError> {
        self.0.insert(frame.raw(), Box::new([0; 512]));
        Ok(())
    }

    fn read(&self, frame: FrameAddress, index: usize) -> Result<u64, PageTableError> {
        self.0.get(&frame.raw()).and_then(|t| t.get(index)).copied().ok_or(PageTableError::Memory)
    }

    fn write(
        &mut self,
        frame: FrameAddress,
        index: usize,
        value: u64,
    ) -> Result<(), PageTableError> {
        *self
            .0
            .get_mut(&frame.raw())
            .and_then(|t| t.get_mut(index))
            .ok_or(PageTableError::Memory)? = value;
        Ok(())
    }
}

struct SliceReader<'a>(&'a [u8]);

impl ImageReader for SliceReader<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn read(&mut self, offset: usize, output: &mut [u8]) -> Result<usize, ProcessError> {
        let end = offset.checked_add(output.len()).ok_or(ProcessError::ReadFailure)?;
        let source = self.0.get(offset..end).ok_or(ProcessError::ReadFailure)?;
        output.copy_from_slice(source);
        Ok(output.len())
    }
}

#[derive(Clone, Debug)]
struct Segment {
    kind: u32,
    flags: u32,
    offset: u64,
    vaddr: u64,
    filesz: u64,
    memsz: u64,
}

#[derive(Clone, Debug)]
struct Elf {
    class: u8,
    data: u8,
    kind: u16,
    machine: u16,
    entry: u64,
    phentsize: u16,
    segments: Vec<Segment>,
    tail: Vec<u8>,
}

impl Elf {
    fn build(&self) -> Vec<u8> {
        let ph_offset = 64usize;
        let entry_size = self.phentsize.max(56) as usize;
        let mut image = vec![0u8; ph_offset + entry_size * self.segments.len()];
        image[..4].copy_from_slice(b"\x7fELF");
        image[4] = self.class;
        image[5] = self.data;
        image[16..18].copy_from_slice(&self.kind.to_le_bytes());
        image[18..20].copy_from_slice(&self.machine.to_le_bytes());
        image[24..32].copy_from_slice(&self.entry.to_le_bytes());
        image[32..40].copy_from_slice(&(ph_offset as u64).to_le_bytes());
        image[54..56].copy_from_slice(&self.phentsize.to_le_bytes());
        image[56..58].copy_from_slice(&(self.segments.len() as u16).to_le_bytes());
        for (index, segment) in self.segments.iter().enumerate() {
            let base = ph_offset + index * entry_size;
            if base + 56 > image.len() {
                break;
            }
            image[base..base + 4].copy_from_slice(&segment.kind.to_le_bytes());
            image[base + 4..base + 8].copy_from_slice(&segment.flags.to_le_bytes());
            image[base + 8..base + 16].copy_from_slice(&segment.offset.to_le_bytes());
            image[base + 16..base + 24].copy_from_slice(&segment.vaddr.to_le_bytes());
            image[base + 32..base + 40].copy_from_slice(&segment.filesz.to_le_bytes());
            image[base + 40..base + 48].copy_from_slice(&segment.memsz.to_le_bytes());
        }
        image.extend_from_slice(&self.tail);
        image
    }
}

fn near_page() -> impl Strategy<Value = u64> {
    prop_oneof![
        3 => (1u64..64).prop_map(|p| p * 0x1000),
        2 => (1u64..64).prop_map(|p| p * 0x1000 + 0x123),
        1 => Just(USER_STACK_BASE as u64),
        1 => (0u64..16).prop_map(|p| USER_STACK_BASE as u64 - p * 0x1000),
        1 => (0u64..16).prop_map(|p| USER_STACK_BASE as u64 + p * 0x1000),
        1 => Just(0x0000_7fff_ffff_f000u64),
        1 => any::<u64>(),
    ]
}

fn segment() -> impl Strategy<Value = Segment> {
    (
        prop_oneof![8 => Just(1u32), 1 => any::<u32>()],
        prop_oneof![Just(4u32), Just(5), Just(6), Just(7), Just(0), any::<u32>()],
        prop_oneof![3 => 0u64..400, 1 => any::<u64>()],
        near_page(),
        prop_oneof![3 => 0u64..300, 1 => any::<u64>()],
        prop_oneof![
            3 => 1u64..0x4000,
            1 => Just(MAX_IMAGE_BYTES as u64),
            1 => Just(MAX_IMAGE_BYTES as u64 + 1),
            1 => any::<u64>(),
        ],
    )
        .prop_map(|(kind, flags, offset, vaddr, filesz, memsz)| Segment {
            kind,
            flags,
            offset,
            vaddr,
            filesz,
            memsz,
        })
}

fn elf() -> impl Strategy<Value = Elf> {
    (
        prop_oneof![9 => Just(2u8), 1 => any::<u8>()],
        prop_oneof![9 => Just(1u8), 1 => any::<u8>()],
        prop_oneof![9 => Just(2u16), 1 => any::<u16>()],
        prop_oneof![9 => Just(0x3eu16), 1 => any::<u16>()],
        prop_oneof![3 => near_page(), 1 => Just(0u64)],
        prop_oneof![8 => Just(56u16), 1 => 56u16..80, 1 => any::<u16>()],
        proptest::collection::vec(segment(), 0..=MAX_PROGRAM_HEADERS + 2),
        proptest::collection::vec(any::<u8>(), 0..512),
        prop_oneof![Just(false), Just(true)],
    )
        .prop_map(|(class, data, kind, machine, entry, phentsize, segments, tail, snap)| {
            // Often point the entry into the first segment so images get past the parser.
            let entry = match segments.first() {
                Some(first) if snap => first.vaddr,
                _ => entry,
            };
            Elf { class, data, kind, machine, entry, phentsize, segments, tail }
        })
}

fn check_plan(plan: ElfLoadPlan, image: &[u8]) -> Result<(), TestCaseError> {
    prop_assert!(plan.entry() != 0);
    prop_assert!(plan.segment_count() >= 1 && plan.segment_count() <= MAX_PROGRAM_HEADERS);
    prop_assert!(plan.memory_bytes() <= MAX_IMAGE_BYTES);
    prop_assert!(plan.segment(plan.segment_count()).is_none());
    let mut total = 0usize;
    let mut entry_executable = false;
    for index in 0..plan.segment_count() {
        let segment = plan.segment(index).unwrap();
        prop_assert!(segment.memory_size() > 0);
        prop_assert!(segment.file_size() <= segment.memory_size());
        prop_assert!(segment.virtual_address() >= 0x1000);
        let end = segment.virtual_address().checked_add(segment.memory_size()).unwrap();
        prop_assert!(end <= 0x0000_8000_0000_0000);
        prop_assert!(segment.file_bytes(image).is_some_and(|b| b.len() == segment.file_size()));
        let flags = segment.flags();
        prop_assert!(flags.user && !(flags.writable && flags.executable));
        entry_executable |=
            flags.executable && segment.virtual_address() <= plan.entry() && plan.entry() < end;
        total += segment.memory_size();
    }
    prop_assert_eq!(total, plan.memory_bytes());
    prop_assert!(entry_executable);
    Ok(())
}

/// Parse, and when accepted, check the plan and push it through load,
/// populate (both flavours), page-table mapping and reclaim.
fn exercise(image: &[u8]) -> Result<(), TestCaseError> {
    let plan = match ElfLoadPlan::parse(image) {
        Ok(plan) => plan,
        Err(error) => {
            prop_assert!(matches!(error, ProcessError::InvalidImage | ProcessError::ReadFailure));
            return Ok(());
        }
    };
    check_plan(plan, image)?;
    // The reader-based parse must agree with the slice parse.
    prop_assert_eq!(ElfLoadPlan::parse_reader(&mut SliceReader(image)), Ok(plan));

    let mut pool = pool();
    // `available()` ignores frames parked in the per-CPU cache, so count live leases.
    let live = |pool: &FramePool| pool.manager().owner_live(OwnerId::KERNEL);
    prop_assert_eq!(live(&pool), 0);
    let mut loaded = match LoadedImage::load(plan, &mut pool) {
        Ok(loaded) => loaded,
        Err(error) => {
            prop_assert!(matches!(
                error,
                LoadError::Capacity | LoadError::Exhausted | LoadError::Overlap
            ));
            prop_assert_eq!(live(&pool), 0, "failed load leaked frames");
            return Ok(());
        }
    };

    // No two image pages may claim the same virtual address (this includes
    // the user stack), and permissions must stay W^X.
    let mut seen = std::collections::HashSet::new();
    for index in 0..loaded.page_count() {
        let page = loaded.page(index).unwrap();
        prop_assert!(
            seen.insert(page.virtual_address()),
            "duplicate page {:#x}",
            page.virtual_address()
        );
        prop_assert_eq!(page.virtual_address() % PAGE_SIZE, 0);
        prop_assert!(page.virtual_address() < 0x0000_8000_0000_0000);
        prop_assert!(!(page.flags().writable && page.flags().executable));
    }

    let mut sink = HostPages::default();
    prop_assert_eq!(loaded.populate(plan, image, &mut sink), Ok(()));
    let mut streamed = HostPages::default();
    let mut scratch = vec![0u8; PAGE_SIZE];
    prop_assert_eq!(
        loaded.populate_reader(plan, &mut SliceReader(image), &mut scratch, &mut streamed),
        Ok(())
    );
    for index in 0..loaded.page_count() {
        let frame = loaded.page(index).unwrap().frame().raw();
        prop_assert!(sink.0[&frame][..] == streamed.0[&frame][..], "populate flavours disagree");
    }

    let mut memory = HostTables::default();
    let mut tables = PageTableBuilder::new(&mut pool, &mut memory).unwrap();
    match tables.map_image(&loaded, &mut pool, &mut memory) {
        Ok(()) => prop_assert_eq!(tables.mapped_pages(), loaded.page_count()),
        Err(error) => prop_assert!(
            matches!(error, PageTableError::Exhausted | PageTableError::Capacity),
            "loadable image failed to map: {:?}",
            error
        ),
    }
    tables.reclaim(&mut pool, &mut memory);
    loaded.reclaim(&mut pool);
    prop_assert_eq!(live(&pool), 0, "frames leaked after reclaim");
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
        let _ = ElfLoadPlan::parse(&bytes);
        let _ = crate::process::validate_elf(&bytes);
    }

    #[test]
    fn structured_images_hold_loader_invariants(elf in elf()) {
        exercise(&elf.build())?;
    }

    #[test]
    fn mutated_images_hold_loader_invariants(
        elf in elf(),
        flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 0..6),
        truncate in proptest::option::of(any::<usize>()),
    ) {
        let mut image = elf.build();
        for (index, value) in flips {
            let at = index % image.len();
            image[at] = value;
        }
        if let Some(len) = truncate {
            image.truncate(len % (image.len() + 1));
        }
        exercise(&image)?;
    }

    /// A well-formed single-segment image always parses, loads and maps.
    #[test]
    fn well_formed_images_round_trip(
        pages in 1u64..8,
        vaddr_page in 1u64..1024,
        file_len in 0u64..0x2000,
        code in proptest::collection::vec(any::<u8>(), 0..0x2000),
    ) {
        let memsz = pages * 0x1000;
        let filesz = file_len.min(memsz).min(code.len() as u64);
        let vaddr = vaddr_page * 0x1000;
        let elf = Elf {
            class: 2,
            data: 1,
            kind: 2,
            machine: 0x3e,
            entry: vaddr,
            phentsize: 56,
            segments: vec![Segment { kind: 1, flags: 5, offset: 64 + 56, vaddr, filesz, memsz }],
            tail: code[..filesz as usize].to_vec(),
        };
        let image = elf.build();
        let plan = ElfLoadPlan::parse(&image).expect("well-formed image must parse");
        prop_assert_eq!(plan.entry() as u64, vaddr);
        prop_assert_eq!(plan.segment(0).unwrap().file_bytes(&image).unwrap(), &code[..filesz as usize]);
        exercise(&image)?;
        let mut pool = pool();
        prop_assert!(LoadedImage::load(plan, &mut pool).is_ok());
    }
}

/// Regression: a PT_LOAD segment covering the user stack used to be admitted
/// (the stack pages were pushed without an overlap check), leaving two pages
/// with one virtual address and a later mapping conflict.
#[test]
fn segment_overlapping_the_user_stack_is_rejected_by_the_loader() {
    let elf = Elf {
        class: 2,
        data: 1,
        kind: 2,
        machine: 0x3e,
        entry: USER_STACK_BASE as u64,
        phentsize: 56,
        segments: vec![Segment {
            kind: 1,
            flags: 5,
            offset: 0,
            vaddr: USER_STACK_BASE as u64,
            filesz: 0,
            memsz: 0x1000,
        }],
        tail: Vec::new(),
    };
    let image = elf.build();
    let plan = ElfLoadPlan::parse(&image).expect("plan is well formed");
    let mut pool = pool();
    assert_eq!(LoadedImage::load(plan, &mut pool).unwrap_err(), LoadError::Overlap);
    assert_eq!(pool.manager().owner_live(OwnerId::KERNEL), 0);
}
