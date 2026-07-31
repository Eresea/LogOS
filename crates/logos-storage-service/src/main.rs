#![no_main]
#![no_std]

use core::mem::size_of;

use logos_service_rt::{BlockClient, BlockError, Context, Header};
use logos_store::{Error, Recovery, SECTOR_SIZE, SectorBackend, Store};

const MINIMUM_SECTORS: usize = 10;
const STORE_MEMORY: usize = 4 * logos_abi::PAGE_SIZE;

type DiskStore = Store<BlockBackend>;

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header = Header::new(*b"storage\0\0\0\0\0\0\0\0\0", logos_service_entry);

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

fn start(context: &mut Context) -> Result<(&'static mut DiskStore, bool), Error> {
    let mut backend = BlockBackend::new(context)?;
    #[cfg(feature = "block-probe")]
    if !backend.client.probe() {
        return Err(Error::Io);
    }
    let blank = backend.superblocks_zero()?;
    if size_of::<DiskStore>() > STORE_MEMORY {
        return Err(Error::Full);
    }
    let slot = context.heap_slot::<DiskStore>().ok_or(Error::Invalid)?;
    let store = if blank {
        Store::format_with_backend_at(slot, backend)?
    } else {
        Store::recover_backend_at(slot, backend)?
    };
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
    Ok((store, blank))
}

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: logos_service_rt::EntryContext) -> ! {
    logos_service_rt::entry(context, run)
}

fn run(context: &mut Context) -> ! {
    if !context.ready() {
        spin();
    }
    let _store = match start(context) {
        Ok((store, _)) => Some(store),
        Err(Error::Corrupt) => {
            context.set_storage_status(logos_service_rt::STORAGE_CORRUPT);
            None
        }
        Err(_) => {
            context.set_storage_status(logos_service_rt::STORAGE_IO_FAILED);
            None
        }
    };
    while context.acknowledged() {
        if !context.wait_for_input() {
            spin();
        }
    }
    spin()
}

fn spin() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
