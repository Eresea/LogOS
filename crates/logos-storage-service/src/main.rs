#![no_main]
#![no_std]

use core::arch::asm;
use logos_core::native_service::{ACKNOWLEDGED, Context, Header, READ_INPUT, READY};
use logos_service_rt as _;

#[global_allocator]
static HEAP: logos_service_rt::heap::PageArena = logos_service_rt::heap::PageArena::new();

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header = Header::new(*b"storage\0\0\0\0\0\0\0\0\0", logos_service_entry);

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: *mut Context) -> ! {
    unsafe {
        let heap = (context as usize).saturating_sub(5 * logos_abi::PAGE_SIZE);
        if !HEAP.initialize(heap, 4 * logos_abi::PAGE_SIZE) {
            loop {
                core::hint::spin_loop();
            }
        }
        (*context).operation = READY;
        asm!("int 0x80");
        #[cfg(feature = "block-probe")]
        if !block_probe(context) {
            loop {
                core::hint::spin_loop();
            }
        }
        while (*context).status == ACKNOWLEDGED {
            (*context).operation = READ_INPUT;
            asm!("int 0x80");
        }
    }
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(feature = "block-probe")]
unsafe fn block_probe(context: *mut Context) -> bool {
    const DEADLINE: u64 = 1_000_000;
    let address = context as u64;
    let Some(page) = (unsafe { Context::block_page_at(address) }) else { return false };
    let info = logos_abi::BlockRequest {
        id: 1,
        operation: logos_abi::BlockOperation::Info,
        lba: 0,
        blocks: 0,
        page: logos_abi::PageHandle(0),
        deadline: DEADLINE,
    };
    let Some(reply) = (unsafe { request_block(address, info) }) else { return false };
    if reply.status != logos_abi::PersistenceStatus::Complete || !reply.info.valid() {
        return false;
    }
    let read = logos_abi::BlockRequest {
        id: 2,
        operation: logos_abi::BlockOperation::Read,
        lba: 0,
        blocks: 1,
        page: page.handle,
        deadline: DEADLINE,
    };
    if !unsafe { request_block(address, read) }
        .is_some_and(|reply| reply.status == logos_abi::PersistenceStatus::Complete)
    {
        return false;
    }
    let flush = logos_abi::BlockRequest {
        id: 3,
        operation: logos_abi::BlockOperation::Flush,
        lba: 0,
        blocks: 0,
        page: logos_abi::PageHandle(0),
        deadline: DEADLINE,
    };
    unsafe { request_block(address, flush) }
        .is_some_and(|reply| reply.status == logos_abi::PersistenceStatus::Complete)
}

#[cfg(feature = "block-probe")]
unsafe fn request_block(
    context: u64,
    request: logos_abi::BlockRequest,
) -> Option<logos_abi::BlockReply> {
    if !unsafe { Context::request_block_at(context, request) } {
        return None;
    }
    unsafe { asm!("int 0x80") };
    unsafe { Context::block_reply_at(context, request.id) }
}
