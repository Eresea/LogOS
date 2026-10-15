#![no_main]

use libfuzzer_sys::fuzz_target;
use logos_vnext::process::{ElfLoadPlan, MAX_IMAGE_BYTES, MAX_PROGRAM_HEADERS};

fuzz_target!(|data: &[u8]| {
    let Ok(plan) = ElfLoadPlan::parse(data) else { return };
    assert!(plan.entry() != 0);
    assert!((1..=MAX_PROGRAM_HEADERS).contains(&plan.segment_count()));
    assert!(plan.memory_bytes() <= MAX_IMAGE_BYTES);
    for index in 0..plan.segment_count() {
        let segment = plan.segment(index).unwrap();
        assert!(segment.file_size() <= segment.memory_size());
        assert!(segment.file_bytes(data).is_some());
        let flags = segment.flags();
        assert!(!(flags.writable && flags.executable));
        let end = segment.virtual_address().checked_add(segment.memory_size()).unwrap();
        assert!(end <= 0x0000_8000_0000_0000);
    }
});
