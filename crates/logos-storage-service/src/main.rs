#![no_main]
#![no_std]

use core::mem::MaybeUninit;
use core::mem::size_of;

use logos_service_rt::{BlockClient, BlockError, Context, Header, ProtocolVersion};
use logos_store::{Error, Recovery, SECTOR_SIZE, SectorBackend, Store};

use logos_storage_service::protocol::{ReadSelection, ReplaceTransaction};

const MINIMUM_SECTORS: usize = 10;

type DiskStore = Store<BlockBackend>;
const STORE_MEMORY: usize = 4 * logos_abi::PAGE_SIZE;

struct RuntimeState {
    replace: MaybeUninit<ReplaceTransaction>,
    replace_active: bool,
    read: MaybeUninit<ReadSelection>,
    read_active: bool,
    store: MaybeUninit<DiskStore>,
}

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header =
    Header::new(*b"storage\0\0\0\0\0\0\0\0\0", ProtocolVersion::V1, logos_service_entry);

struct BlockBackend {
    client: BlockClient,
    sectors: usize,
}

impl BlockBackend {
    fn new(context: &Context) -> Result<Self, Error> {
        let mut client = context.block_client().ok_or(Error::Invalid)?;
        let info = client.info().map_err(map_block_error)?;
        if !info.valid() || info.logical_block_size as usize != SECTOR_SIZE {
            return Err(Error::Invalid);
        }
        let sectors = usize::try_from(info.blocks).map_err(|_| Error::Full)?;
        (sectors >= MINIMUM_SECTORS).then_some(Self { client, sectors }).ok_or(Error::Invalid)
    }

    fn superblocks_zero(&mut self) -> Result<bool, Error> {
        let mut first = [0; SECTOR_SIZE];
        let mut second = [0; SECTOR_SIZE];
        self.read(0, &mut first)?;
        self.read(1, &mut second)?;
        Ok(first.iter().all(|byte| *byte == 0) && second.iter().all(|byte| *byte == 0))
    }
}

impl SectorBackend for BlockBackend {
    fn sectors(&self) -> usize {
        self.sectors
    }

    fn read(&mut self, sector: usize, output: &mut [u8; SECTOR_SIZE]) -> Result<(), Error> {
        self.client.read_sector(sector, output).map_err(map_block_error)
    }

    fn write(&mut self, sector: usize, input: &[u8; SECTOR_SIZE]) -> Result<(), Error> {
        self.client.write_sector(sector, input).map_err(map_block_error)
    }

    fn flush(&mut self) -> Result<(), Error> {
        self.client.flush().map_err(map_block_error)
    }
}

fn map_block_error(error: BlockError) -> Error {
    match error {
        BlockError::TimedOut => Error::TimedOut,
        BlockError::Corrupt => Error::Corrupt,
        BlockError::Full => Error::Full,
        BlockError::NotFound => Error::NotFound,
        BlockError::Invalid => Error::Invalid,
        BlockError::Io => Error::Io,
    }
}

fn start(context: &mut Context) -> Result<(&'static mut RuntimeState, bool), Error> {
    let mut backend = BlockBackend::new(context)?;
    #[cfg(feature = "block-probe")]
    if !backend.client.probe() {
        return Err(Error::Io);
    }
    let blank = backend.superblocks_zero()?;
    if size_of::<RuntimeState>() > STORE_MEMORY {
        return Err(Error::Full);
    }
    let slot = context.heap_slot::<RuntimeState>().ok_or(Error::Invalid)?;
    let state = unsafe { &mut *slot.as_mut_ptr() };
    state.replace_active = false;
    state.read_active = false;
    if blank {
        Store::format_with_backend_at(&mut state.store, backend)?
    } else {
        Store::recover_backend_at(&mut state.store, backend)?
    };
    let store = unsafe { state.store.assume_init_mut() };
    let recovery = store.recovery();
    context.set_storage_status(if blank {
        logos_service_rt::STORAGE_FORMATTED
    } else if recovery == Recovery::Incomplete {
        logos_service_rt::STORAGE_RECOVERED_INCOMPLETE
    } else if recovery == Recovery::Corrupt {
        logos_service_rt::STORAGE_CORRUPT
    } else {
        logos_service_rt::STORAGE_RECOVERED
    });
    Ok((state, blank))
}

fn map_error(error: Error) -> logos_abi::PersistenceStatus {
    match error {
        Error::Invalid => logos_abi::PersistenceStatus::Invalid,
        Error::Io => logos_abi::PersistenceStatus::Io,
        Error::TimedOut => logos_abi::PersistenceStatus::TimedOut,
        Error::Corrupt => logos_abi::PersistenceStatus::Corrupt,
        Error::Full => logos_abi::PersistenceStatus::Full,
        Error::NotFound => logos_abi::PersistenceStatus::NotFound,
        Error::Interrupted => logos_abi::PersistenceStatus::Cancelled,
    }
}

fn reply(
    request: logos_abi::StoreRequest,
    status: logos_abi::PersistenceStatus,
) -> logos_abi::StoreReply {
    logos_abi::StoreReply { id: request.id, status, version: 0, length: 0 }
}

fn process(
    context: &Context,
    state: &mut RuntimeState,
    request: logos_abi::StoreRequest,
) -> logos_abi::StoreReply {
    let Some(page) = context.shared_page() else {
        return reply(request, logos_abi::PersistenceStatus::Invalid);
    };
    if matches!(
        request.operation,
        logos_abi::StoreOperation::ReadChunk | logos_abi::StoreOperation::WriteChunk
    ) && request.page != page.handle
    {
        return reply(request, logos_abi::PersistenceStatus::Denied);
    }
    match request.operation {
        logos_abi::StoreOperation::OpenRead => process_open_read(state, request),
        logos_abi::StoreOperation::ReadChunk => process_read_chunk(state, request, page),
        logos_abi::StoreOperation::BeginReplace => process_begin_replace(state, request),
        logos_abi::StoreOperation::WriteChunk => process_write_chunk(state, request, page),
        logos_abi::StoreOperation::Commit => process_commit(state, request),
        logos_abi::StoreOperation::Abort | logos_abi::StoreOperation::Cancel => {
            process_abort(state, request)
        }
    }
}

#[inline(never)]
fn process_open_read(
    state: &mut RuntimeState,
    request: logos_abi::StoreRequest,
) -> logos_abi::StoreReply {
    let store = unsafe { state.store.assume_init_mut() };
    match store.metadata(
        request.namespace,
        &request.name[..request.name_length as usize],
        request.version,
    ) {
        Ok((version, length)) => {
            state.read.write(ReadSelection::new(request, length));
            state.read_active = true;
            logos_abi::StoreReply {
                id: request.id,
                status: logos_abi::PersistenceStatus::Complete,
                version,
                length: length as u32,
            }
        }
        Err(error) => reply(request, map_error(error)),
    }
}

#[inline(never)]
fn process_read_chunk(
    state: &mut RuntimeState,
    request: logos_abi::StoreRequest,
    page: logos_service_rt::SharedPage,
) -> logos_abi::StoreReply {
    if !state.read_active {
        return reply(request, logos_abi::PersistenceStatus::Invalid);
    }
    let selection = unsafe { state.read.assume_init_ref() };
    let store = unsafe { state.store.assume_init_mut() };
    let output =
        unsafe { core::slice::from_raw_parts_mut(page.address as *mut u8, logos_abi::PAGE_SIZE) };
    let (version, length) =
        match store.read(selection.namespace(), selection.name(), selection.version(), output) {
            Ok(value) => value,
            Err(error) => return reply(request, map_error(error)),
        };
    let offset = usize::try_from(request.offset).unwrap_or(usize::MAX);
    let copied = length.saturating_sub(offset).min(request.length as usize);
    if copied != 0 {
        output.copy_within(offset..offset + copied, 0);
    }
    logos_abi::StoreReply {
        id: request.id,
        status: logos_abi::PersistenceStatus::Complete,
        version,
        length: copied as u32,
    }
}

#[inline(never)]
fn process_begin_replace(
    state: &mut RuntimeState,
    request: logos_abi::StoreRequest,
) -> logos_abi::StoreReply {
    if state.replace_active {
        return reply(request, logos_abi::PersistenceStatus::Invalid);
    }
    if let Some(replace) = ReplaceTransaction::begin(request) {
        state.replace.write(replace);
        state.replace_active = true;
        reply(request, logos_abi::PersistenceStatus::Complete)
    } else {
        reply(request, logos_abi::PersistenceStatus::Invalid)
    }
}

#[inline(never)]
fn process_write_chunk(
    state: &mut RuntimeState,
    request: logos_abi::StoreRequest,
    page: logos_service_rt::SharedPage,
) -> logos_abi::StoreReply {
    if !state.replace_active {
        return reply(request, logos_abi::PersistenceStatus::Invalid);
    }
    let replace = unsafe { state.replace.assume_init_mut() };
    let page =
        unsafe { core::slice::from_raw_parts(page.address as *const u8, logos_abi::PAGE_SIZE) };
    if replace.write(request, page) {
        reply(request, logos_abi::PersistenceStatus::Complete)
    } else {
        reply(request, logos_abi::PersistenceStatus::Invalid)
    }
}

#[inline(never)]
fn process_commit(
    state: &mut RuntimeState,
    request: logos_abi::StoreRequest,
) -> logos_abi::StoreReply {
    if !state.replace_active {
        return reply(request, logos_abi::PersistenceStatus::Invalid);
    }
    let replace = state.replace.as_ptr();
    if !unsafe { ReplaceTransaction::complete_at(replace) } {
        return reply(request, logos_abi::PersistenceStatus::Invalid);
    }
    let result = commit_store(state, replace);
    state.replace_active = false;
    match result {
        Ok(version) => logos_abi::StoreReply {
            id: request.id,
            status: logos_abi::PersistenceStatus::Complete,
            version,
            length: 0,
        },
        Err(error) => reply(request, map_error(error)),
    }
}

#[inline(never)]
fn commit_store(
    state: &mut RuntimeState,
    replace: *const ReplaceTransaction,
) -> Result<u64, Error> {
    let (namespace, name_ptr, name_length, bytes_ptr, bytes_length) =
        unsafe { ReplaceTransaction::raw_parts_at(replace) };
    let name = unsafe { core::slice::from_raw_parts(name_ptr, name_length) };
    let bytes = unsafe { core::slice::from_raw_parts(bytes_ptr, bytes_length) };
    unsafe { state.store.assume_init_mut() }.replace(namespace, name, bytes)
}

#[inline(never)]
fn process_abort(
    state: &mut RuntimeState,
    request: logos_abi::StoreRequest,
) -> logos_abi::StoreReply {
    state.replace_active = false;
    if request.operation == logos_abi::StoreOperation::Cancel {
        state.read_active = false;
    }
    reply(request, logos_abi::PersistenceStatus::Complete)
}

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: logos_service_rt::EntryContext) -> ! {
    logos_service_rt::entry(context, run)
}

fn run(context: &mut Context) -> ! {
    if !context.ready() {
        spin();
    }
    let state = match start(context) {
        Ok((state, _)) => Some(state),
        Err(Error::Corrupt) => {
            context.set_storage_status(logos_service_rt::STORAGE_CORRUPT);
            None
        }
        Err(_) => {
            context.set_storage_status(logos_service_rt::STORAGE_IO_FAILED);
            None
        }
    };
    let Some(state) = state else { spin() };
    while context.acknowledged() {
        if !context.wait_for_input() {
            spin();
        }
        if let Some(request) = context.store_request() {
            #[cfg(feature = "test-hooks")]
            inject_failure(request.id);
            let response = process(context, state, request);
            if !context.store_reply(response) {
                spin();
            }
        }
    }
    spin()
}

#[cfg(feature = "test-hooks")]
fn inject_failure(id: u32) {
    if id == u32::MAX - 1 {
        panic!("test panic");
    }
    if id == u32::MAX - 2 {
        let address = core::hint::black_box(1usize);
        unsafe { (address as *mut u8).write_volatile(1) };
    }
}

fn spin() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
