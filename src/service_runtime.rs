//! Post-UEFI service image and address-space ownership.

use core::mem::MaybeUninit;

use logos_abi::ServiceId;

use crate::{
    frame_pool::FramePool,
    loader::{LoadError, LoadedImage},
    page_table::{IdentityPageTableMemory, PageTableBuilder, PageTableError},
    service_images::SERVICE_IMAGES,
    service_loader::ServiceImageBundle,
};

const SERVICE_COUNT: usize = SERVICE_IMAGES.len();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceRuntimeError {
    Resources,
    Image,
    Load(LoadError),
    Populate(LoadError),
    PageTableRoot(PageTableError),
    PageTableMap(PageTableError),
}

pub struct ServiceRuntime {
    frame_pool: FramePool,
    images: [LoadedImage; SERVICE_COUNT],
    tables: [MaybeUninit<PageTableBuilder>; SERVICE_COUNT],
    table_ready: [bool; SERVICE_COUNT],
}

impl ServiceRuntime {
    pub const fn new() -> Self {
        Self {
            frame_pool: FramePool::empty(),
            images: [
                LoadedImage::empty(),
                LoadedImage::empty(),
                LoadedImage::empty(),
                LoadedImage::empty(),
                LoadedImage::empty(),
            ],
            tables: [const { MaybeUninit::uninit() }; SERVICE_COUNT],
            table_ready: [false; SERVICE_COUNT],
        }
    }

    pub fn start(&mut self, bundle: &ServiceImageBundle) -> Result<(), ServiceRuntimeError> {
        let resources = crate::arch::boot_resources().ok_or(ServiceRuntimeError::Resources)?;
        self.frame_pool
            .initialize(resources.memory_map())
            .map_err(|_| ServiceRuntimeError::Resources)?;

        for (index, spec) in SERVICE_IMAGES.iter().enumerate() {
            let service = spec.service();
            let image = unsafe { bundle.image(service) }.ok_or(ServiceRuntimeError::Image)?;
            let plan = spec.validate_image(image).map_err(|_| ServiceRuntimeError::Image)?;
            let loaded =
                LoadedImage::load(plan, &mut self.frame_pool).map_err(ServiceRuntimeError::Load)?;
            let mut memory = IdentityPageTableMemory;
            if let Err(error) = loaded.populate(plan, image, &mut memory) {
                let mut loaded = loaded;
                loaded.reclaim(&mut self.frame_pool);
                return Err(ServiceRuntimeError::Populate(error));
            }
            let mut tables = match PageTableBuilder::new(&mut self.frame_pool, &mut memory) {
                Ok(tables) => tables,
                Err(error) => {
                    let mut loaded = loaded;
                    loaded.reclaim(&mut self.frame_pool);
                    return Err(ServiceRuntimeError::PageTableRoot(error));
                }
            };
            if let Err(error) = tables.map_image(&loaded, &mut self.frame_pool, &mut memory) {
                tables.reclaim(&mut self.frame_pool, &mut memory);
                let mut loaded = loaded;
                loaded.reclaim(&mut self.frame_pool);
                return Err(ServiceRuntimeError::PageTableMap(error));
            }
            self.images[index] = loaded;
            self.tables[index].write(tables);
            self.table_ready[index] = true;
        }
        Ok(())
    }

    pub fn image(&self, service: ServiceId) -> Option<&LoadedImage> {
        let index = service_index(service);
        if self.table_ready[index] { Some(&self.images[index]) } else { None }
    }

    pub fn root(&self, service: ServiceId) -> Option<usize> {
        let index = service_index(service);
        if !self.table_ready[index] {
            return None;
        }
        // SAFETY: `table_ready` is set only after the corresponding builder is
        // initialized and remains true for the runtime lifetime.
        Some(unsafe { self.tables[index].assume_init_ref().root().raw() as usize })
    }
}

impl Default for ServiceRuntime {
    fn default() -> Self {
        Self::new()
    }
}

const fn service_index(service: ServiceId) -> usize {
    match service {
        ServiceId::Input => 0,
        ServiceId::Display => 1,
        ServiceId::Terminal => 2,
        ServiceId::Session => 3,
        ServiceId::Commands => 4,
    }
}
