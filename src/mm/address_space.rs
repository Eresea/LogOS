use core::{
    arch::{asm, x86_64::__cpuid},
    ptr,
};

use crate::mm::memory::{Contiguous, Page, PhysicalMemory};

const ENTRIES: usize = 512;
const MAPPED_PAGES: usize = 160;
const PAGE_SIZE: u64 = 4096;
const PRESENT: u64 = 1;
const WRITABLE: u64 = 1 << 1;
const USER: u64 = 1 << 2;
const NO_EXECUTE: u64 = 1 << 63;
const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
const SHARED_PAGE: usize = ENTRIES - 5;
const BLOCK_PAGE: usize = ENTRIES - 10;
const NETWORK_RX_PAGE: usize = ENTRIES - 23;
const NETWORK_TX_PAGE: usize = ENTRIES - 24;
const DISPLAY_PAGE: usize = ENTRIES - 26;
const INPUT_PAGE: usize = ENTRIES - 27;
const SESSION_CLIENT_PAGE: usize = ENTRIES - 28;
const SESSION_SERVER_PAGE: usize = ENTRIES - 29;
const EFFECT_PAGE: usize = ENTRIES - 30;
const STORE_CLIENT_PAGE: usize = ENTRIES - 31;
const STORE_SERVER_PAGE: usize = ENTRIES - 32;
const BLOCK_CLIENT_PAGE: usize = ENTRIES - 33;
const NETWORK_DEVICE_ENDPOINT_PAGE: usize = ENTRIES - 34;
const NETWORK_EVENT_ENDPOINT_PAGE: usize = ENTRIES - 35;
const NETWORK_CLIENT_PAGE: usize = ENTRIES - 36;
const NETWORK_SERVER_PAGE: usize = ENTRIES - 37;
const REMOTE_PAGE: usize = ENTRIES - 38;
const NETWORK_STREAM_PAGE: usize = ENTRIES - 25;
const STACK_TOP: usize = REMOTE_PAGE;
// NetworkState owns bounded TCP RX/TX storage in the service task frame.
// Keep enough fixed stack for that state plus packet parsing without adding
// an allocator or moving ownership out of the replaceable service.
const STACK_PAGES: usize = 96;
const STACK_BASE: usize = STACK_TOP - STACK_PAGES;
const CONTEXT_PAGE: usize = ENTRIES - 4;
const HEAP_PAGE: usize = ENTRIES - 9;
const HEAP_PAGES: usize = 4;

pub struct AddressSpace {
    pml4: Page,
    pdpt: Page,
    pd: Page,
    pt: Page,
    stack_lower: Page,
    stack: Contiguous,
    mapped: [Option<Mapping>; MAPPED_PAGES],
    borrowed: Option<u64>,
    base: u64,
}

struct Mapping {
    index: usize,
    page: Page,
}

pub struct ContextMapping {
    pub context: (u64, u64),
    pub input: Option<(u64, u64)>,
    pub display: Option<(u64, u64)>,
    pub session_client: Option<(u64, u64)>,
    pub session_server: Option<(u64, u64)>,
    pub effect: Option<(u64, u64)>,
    pub store_client: Option<(u64, u64)>,
    pub store_server: Option<(u64, u64)>,
    pub block_client: Option<(u64, u64)>,
    pub remote: Option<(u64, u64)>,
    pub network_client: Option<(u64, u64)>,
    pub network_server: Option<(u64, u64)>,
    pub network_device: Option<(u64, u64)>,
    pub network_event: Option<(u64, u64)>,
    pub network_stream: Option<(u64, u64)>,
}

impl AddressSpace {
    pub fn new(physical: &mut PhysicalMemory) -> Option<Self> {
        let pml4 = physical.allocate_owned()?;
        let Some(pdpt) = physical.allocate_owned() else {
            let _ = physical.release_page(pml4);
            return None;
        };
        let Some(pd) = physical.allocate_owned() else {
            let _ = physical.release_page(pdpt);
            let _ = physical.release_page(pml4);
            return None;
        };
        let Some(pt) = physical.allocate_owned() else {
            let _ = physical.release_page(pd);
            let _ = physical.release_page(pdpt);
            let _ = physical.release_page(pml4);
            return None;
        };
        let Some(stack_lower) = physical.allocate_owned() else {
            let _ = physical.release_page(pt);
            let _ = physical.release_page(pd);
            let _ = physical.release_page(pdpt);
            let _ = physical.release_page(pml4);
            return None;
        };
        let Some(stack) = physical.allocate_contiguous(STACK_PAGES) else {
            let _ = physical.release_page(stack_lower);
            let _ = physical.release_page(pt);
            let _ = physical.release_page(pd);
            let _ = physical.release_page(pdpt);
            let _ = physical.release_page(pml4);
            return None;
        };
        let pml4_address = pml4.address();
        let pdpt_address = pdpt.address();
        let pd_address = pd.address();
        let pt_address = pt.address();
        let stack_address = stack.address();
        unsafe {
            ptr::copy_nonoverlapping(read_cr3() as *const u64, pml4_address as *mut u64, ENTRIES);
            ptr::write_bytes(pdpt_address as *mut u8, 0, PAGE_SIZE as usize);
            ptr::write_bytes(pd_address as *mut u8, 0, PAGE_SIZE as usize);
            ptr::write_bytes(pt_address as *mut u8, 0, PAGE_SIZE as usize);
            let pml4_table = pml4_address as *mut u64;
            let Some(slot) = (256..ENTRIES).find(|&index| pml4_table.add(index).read() == 0) else {
                let _ = stack.release(physical);
                let _ = physical.release_page(pt);
                let _ = physical.release_page(pd);
                let _ = physical.release_page(pdpt);
                let _ = physical.release_page(pml4);
                return None;
            };
            pml4_table.add(slot).write(pdpt_address | PRESENT | WRITABLE | USER);
            (pdpt_address as *mut u64).write(pd_address | PRESENT | WRITABLE | USER);
            (pd_address as *mut u64).write(pt_address | PRESENT | WRITABLE | USER);
            for index in STACK_BASE..STACK_TOP {
                let offset = u64::try_from(index - STACK_BASE).unwrap_or(0) * PAGE_SIZE;
                (pt_address as *mut u64)
                    .add(index)
                    .write((stack_address + offset) | PRESENT | WRITABLE | USER | NO_EXECUTE);
            }
            Some(Self {
                pml4,
                pdpt,
                pd,
                pt,
                stack_lower,
                stack,
                mapped: [const { None }; MAPPED_PAGES],
                borrowed: None,
                base: canonical_address(slot),
            })
        }
    }

    pub fn map_image(
        &mut self,
        physical: &mut PhysicalMemory,
        payload: crate::platform::payload::Payload,
    ) -> Option<u64> {
        if !enable_nx() {
            return None;
        }
        for section in payload.sections() {
            let start = usize::try_from(section.address).ok()? / PAGE_SIZE as usize;
            let end_rva = section.address.checked_add(section.size)?;
            let end = usize::try_from(end_rva.checked_add(PAGE_SIZE as u32 - 1)?).ok()?
                / PAGE_SIZE as usize;
            if end >= STACK_BASE {
                self.unmap_image(physical);
                return None;
            }
            for index in start..end {
                if !self.map_page(physical, payload, index, section.writable, section.executable) {
                    self.unmap_image(physical);
                    return None;
                }
            }
        }
        let entry = self.base.checked_add(u64::from(payload.entry_rva()))?;
        self.image_maps(entry).then_some(entry)
    }

    pub fn map_probe(&mut self, physical: &mut PhysicalMemory) -> Option<u64> {
        if self.mapping(0).is_some() {
            return None;
        }
        let page = physical.allocate_owned()?;
        unsafe {
            ptr::write_bytes(page.address() as *mut u8, 0, PAGE_SIZE as usize);
            (page.address() as *mut u8).write_volatile(0xcd);
            (page.address() as *mut u8).add(1).write_volatile(0x80);
        }
        let address = page.address();
        if let Err(page) = self.insert_mapping(0, page) {
            let _ = physical.release_page(page);
            return None;
        }
        unsafe {
            (self.pt.address() as *mut u64).write_volatile(address | PRESENT | USER);
            (self.pt.address() as *mut u64)
                .add(1)
                .write_volatile(address | PRESENT | WRITABLE | USER | NO_EXECUTE);
        }
        Some(self.base)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn map_context(
        &mut self,
        physical: &mut PhysicalMemory,
        input: bool,
        display: bool,
        session_client: bool,
        session_server: bool,
        effect: bool,
        store_client: bool,
        store_server: bool,
        block_client: bool,
        remote: bool,
        network_client: bool,
        network_server: bool,
        network_device: bool,
        network_event: bool,
        network_stream: bool,
    ) -> Option<ContextMapping> {
        if self.mapping(CONTEXT_PAGE).is_some()
            || input && self.mapping(INPUT_PAGE).is_some()
            || display && self.mapping(DISPLAY_PAGE).is_some()
            || session_client && self.mapping(SESSION_CLIENT_PAGE).is_some()
            || session_server && self.mapping(SESSION_SERVER_PAGE).is_some()
            || effect && self.mapping(EFFECT_PAGE).is_some()
            || store_client && self.mapping(STORE_CLIENT_PAGE).is_some()
            || store_server && self.mapping(STORE_SERVER_PAGE).is_some()
            || block_client && self.mapping(BLOCK_CLIENT_PAGE).is_some()
            || remote && self.mapping(REMOTE_PAGE).is_some()
            || network_client && self.mapping(NETWORK_CLIENT_PAGE).is_some()
            || network_server && self.mapping(NETWORK_SERVER_PAGE).is_some()
            || network_device && self.mapping(NETWORK_DEVICE_ENDPOINT_PAGE).is_some()
            || network_event && self.mapping(NETWORK_EVENT_ENDPOINT_PAGE).is_some()
            || network_stream && self.mapping(NETWORK_STREAM_PAGE).is_some()
        {
            return None;
        }
        let context = physical.allocate_owned()?;
        let input_page = if input {
            match physical.allocate_owned() {
                Some(page) => Some(page),
                None => {
                    let _ = physical.release_page(context);
                    return None;
                }
            }
        } else {
            None
        };
        let display_page = if display {
            match physical.allocate_owned() {
                Some(page) => Some(page),
                None => {
                    if let Some(page) = input_page {
                        let _ = physical.release_page(page);
                    }
                    let _ = physical.release_page(context);
                    return None;
                }
            }
        } else {
            None
        };
        let session_client_page = if session_client {
            match physical.allocate_owned() {
                Some(page) => Some(page),
                None => {
                    if let Some(page) = input_page {
                        let _ = physical.release_page(page);
                    }
                    if let Some(page) = display_page {
                        let _ = physical.release_page(page);
                    }
                    let _ = physical.release_page(context);
                    return None;
                }
            }
        } else {
            None
        };
        let session_server_page = if session_server {
            match physical.allocate_owned() {
                Some(page) => Some(page),
                None => {
                    for page in
                        [input_page, display_page, session_client_page].into_iter().flatten()
                    {
                        let _ = physical.release_page(page);
                    }
                    let _ = physical.release_page(context);
                    return None;
                }
            }
        } else {
            None
        };
        let effect_page = if effect {
            match physical.allocate_owned() {
                Some(page) => Some(page),
                None => {
                    for page in [input_page, display_page, session_client_page, session_server_page]
                        .into_iter()
                        .flatten()
                    {
                        let _ = physical.release_page(page);
                    }
                    let _ = physical.release_page(context);
                    return None;
                }
            }
        } else {
            None
        };
        let store_client_page = if store_client {
            match physical.allocate_owned() {
                Some(page) => Some(page),
                None => {
                    for page in [
                        input_page,
                        display_page,
                        session_client_page,
                        session_server_page,
                        effect_page,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let _ = physical.release_page(page);
                    }
                    let _ = physical.release_page(context);
                    return None;
                }
            }
        } else {
            None
        };
        let store_server_page = if store_server {
            match physical.allocate_owned() {
                Some(page) => Some(page),
                None => {
                    for page in [
                        input_page,
                        display_page,
                        session_client_page,
                        session_server_page,
                        effect_page,
                        store_client_page,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let _ = physical.release_page(page);
                    }
                    let _ = physical.release_page(context);
                    return None;
                }
            }
        } else {
            None
        };
        let block_client_page = if block_client {
            match physical.allocate_owned() {
                Some(page) => Some(page),
                None => {
                    for page in [
                        input_page,
                        display_page,
                        session_client_page,
                        session_server_page,
                        effect_page,
                        store_client_page,
                        store_server_page,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let _ = physical.release_page(page);
                    }
                    let _ = physical.release_page(context);
                    return None;
                }
            }
        } else {
            None
        };
        let remote_page = if remote {
            match physical.allocate_owned() {
                Some(page) => Some(page),
                None => {
                    for page in [
                        input_page,
                        display_page,
                        session_client_page,
                        session_server_page,
                        effect_page,
                        store_client_page,
                        store_server_page,
                        block_client_page,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let _ = physical.release_page(page);
                    }
                    let _ = physical.release_page(context);
                    return None;
                }
            }
        } else {
            None
        };
        let network_device_page = if network_device {
            match physical.allocate_owned() {
                Some(page) => Some(page),
                None => {
                    for page in [
                        input_page,
                        display_page,
                        session_client_page,
                        session_server_page,
                        effect_page,
                        store_client_page,
                        store_server_page,
                        block_client_page,
                        remote_page,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let _ = physical.release_page(page);
                    }
                    let _ = physical.release_page(context);
                    return None;
                }
            }
        } else {
            None
        };
        let network_client_page = if network_client {
            match physical.allocate_owned() {
                Some(page) => Some(page),
                None => {
                    for page in [
                        input_page,
                        display_page,
                        session_client_page,
                        session_server_page,
                        effect_page,
                        store_client_page,
                        store_server_page,
                        block_client_page,
                        remote_page,
                        network_device_page,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let _ = physical.release_page(page);
                    }
                    let _ = physical.release_page(context);
                    return None;
                }
            }
        } else {
            None
        };
        let network_server_page = if network_server {
            match physical.allocate_owned() {
                Some(page) => Some(page),
                None => {
                    for page in [
                        input_page,
                        display_page,
                        session_client_page,
                        session_server_page,
                        effect_page,
                        store_client_page,
                        store_server_page,
                        block_client_page,
                        remote_page,
                        network_device_page,
                        network_client_page,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let _ = physical.release_page(page);
                    }
                    let _ = physical.release_page(context);
                    return None;
                }
            }
        } else {
            None
        };
        let network_event_page = if network_event {
            match physical.allocate_owned() {
                Some(page) => Some(page),
                None => {
                    for page in [
                        input_page,
                        display_page,
                        session_client_page,
                        session_server_page,
                        effect_page,
                        store_client_page,
                        store_server_page,
                        block_client_page,
                        network_device_page,
                        network_client_page,
                        network_server_page,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let _ = physical.release_page(page);
                    }
                    let _ = physical.release_page(context);
                    return None;
                }
            }
        } else {
            None
        };
        let network_stream_page = if network_stream {
            match physical.allocate_owned() {
                Some(page) => Some(page),
                None => {
                    for page in [
                        input_page,
                        display_page,
                        session_client_page,
                        session_server_page,
                        effect_page,
                        store_client_page,
                        store_server_page,
                        block_client_page,
                        remote_page,
                        network_device_page,
                        network_client_page,
                        network_server_page,
                        network_event_page,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let _ = physical.release_page(page);
                    }
                    let _ = physical.release_page(context);
                    return None;
                }
            }
        } else {
            None
        };
        let context_address = context.address();
        let input_address = input_page.as_ref().map(Page::address);
        let display_address = display_page.as_ref().map(Page::address);
        let session_client_address = session_client_page.as_ref().map(Page::address);
        let session_server_address = session_server_page.as_ref().map(Page::address);
        let effect_address = effect_page.as_ref().map(Page::address);
        let store_client_address = store_client_page.as_ref().map(Page::address);
        let store_server_address = store_server_page.as_ref().map(Page::address);
        let block_client_address = block_client_page.as_ref().map(Page::address);
        let remote_address = remote_page.as_ref().map(Page::address);
        let network_client_address = network_client_page.as_ref().map(Page::address);
        let network_server_address = network_server_page.as_ref().map(Page::address);
        let network_device_address = network_device_page.as_ref().map(Page::address);
        let network_event_address = network_event_page.as_ref().map(Page::address);
        let network_stream_address = network_stream_page.as_ref().map(Page::address);
        let input_virtual = input_page.as_ref().map(|_| self.base + PAGE_SIZE * INPUT_PAGE as u64);
        let display_virtual =
            display_page.as_ref().map(|_| self.base + PAGE_SIZE * DISPLAY_PAGE as u64);
        let session_client_virtual = session_client_page
            .as_ref()
            .map(|_| self.base + PAGE_SIZE * SESSION_CLIENT_PAGE as u64);
        let session_server_virtual = session_server_page
            .as_ref()
            .map(|_| self.base + PAGE_SIZE * SESSION_SERVER_PAGE as u64);
        let effect_virtual =
            effect_page.as_ref().map(|_| self.base + PAGE_SIZE * EFFECT_PAGE as u64);
        let store_client_virtual =
            store_client_page.as_ref().map(|_| self.base + PAGE_SIZE * STORE_CLIENT_PAGE as u64);
        let store_server_virtual =
            store_server_page.as_ref().map(|_| self.base + PAGE_SIZE * STORE_SERVER_PAGE as u64);
        let block_client_virtual =
            block_client_page.as_ref().map(|_| self.base + PAGE_SIZE * BLOCK_CLIENT_PAGE as u64);
        let remote_virtual =
            remote_page.as_ref().map(|_| self.base + PAGE_SIZE * REMOTE_PAGE as u64);
        let network_client_virtual = network_client_page
            .as_ref()
            .map(|_| self.base + PAGE_SIZE * NETWORK_CLIENT_PAGE as u64);
        let network_server_virtual = network_server_page
            .as_ref()
            .map(|_| self.base + PAGE_SIZE * NETWORK_SERVER_PAGE as u64);
        let network_device_virtual = network_device_page
            .as_ref()
            .map(|_| self.base + PAGE_SIZE * NETWORK_DEVICE_ENDPOINT_PAGE as u64);
        let network_event_virtual = network_event_page
            .as_ref()
            .map(|_| self.base + PAGE_SIZE * NETWORK_EVENT_ENDPOINT_PAGE as u64);
        let network_stream_virtual = network_stream_page
            .as_ref()
            .map(|_| self.base + PAGE_SIZE * NETWORK_STREAM_PAGE as u64);
        unsafe {
            ptr::write_bytes(context_address as *mut u8, 0, PAGE_SIZE as usize);
            (context_address as *mut logos_core::native_service::ControlPage)
                .write_volatile(logos_core::native_service::ControlPage::new());
            (self.pt.address() as *mut u64)
                .add(CONTEXT_PAGE)
                .write_volatile(context_address | PRESENT | WRITABLE | USER | NO_EXECUTE);
        }
        if let Err(page) = self.insert_mapping(CONTEXT_PAGE, context) {
            let _ = physical.release_page(page);
            if let Some(page) = input_page {
                let _ = physical.release_page(page);
            }
            if let Some(page) = display_page {
                let _ = physical.release_page(page);
            }
            for page in [
                session_client_page,
                session_server_page,
                effect_page,
                store_client_page,
                store_server_page,
                block_client_page,
                remote_page,
                network_client_page,
                network_server_page,
                network_device_page,
                network_event_page,
                network_stream_page,
            ]
            .into_iter()
            .flatten()
            {
                let _ = physical.release_page(page);
            }
            return None;
        }
        for (index, page) in [
            (input.then_some(INPUT_PAGE), input_page),
            (display.then_some(DISPLAY_PAGE), display_page),
            (session_client.then_some(SESSION_CLIENT_PAGE), session_client_page),
            (session_server.then_some(SESSION_SERVER_PAGE), session_server_page),
            (effect.then_some(EFFECT_PAGE), effect_page),
            (store_client.then_some(STORE_CLIENT_PAGE), store_client_page),
            (store_server.then_some(STORE_SERVER_PAGE), store_server_page),
            (block_client.then_some(BLOCK_CLIENT_PAGE), block_client_page),
            (remote.then_some(REMOTE_PAGE), remote_page),
            (network_client.then_some(NETWORK_CLIENT_PAGE), network_client_page),
            (network_server.then_some(NETWORK_SERVER_PAGE), network_server_page),
            (network_device.then_some(NETWORK_DEVICE_ENDPOINT_PAGE), network_device_page),
            (network_event.then_some(NETWORK_EVENT_ENDPOINT_PAGE), network_event_page),
            (network_stream.then_some(NETWORK_STREAM_PAGE), network_stream_page),
        ] {
            let (Some(index), Some(page)) = (index, page) else { continue };
            let address = page.address();
            unsafe {
                ptr::write_bytes(address as *mut u8, 0, PAGE_SIZE as usize);
                if index == INPUT_PAGE {
                    (address as *mut logos_core::native_service::InputPage)
                        .write_volatile(logos_core::native_service::InputPage::new(1));
                } else if index == DISPLAY_PAGE {
                    (address as *mut logos_core::native_service::DisplayPage)
                        .write_volatile(logos_core::native_service::DisplayPage::new(1));
                } else if index == SESSION_CLIENT_PAGE {
                    (address as *mut logos_core::native_service::SessionClientPage)
                        .write_volatile(logos_core::native_service::SessionClientPage::new(1, 1));
                } else if index == SESSION_SERVER_PAGE {
                    (address as *mut logos_core::native_service::SessionServerPage)
                        .write_volatile(logos_core::native_service::SessionServerPage::new(1, 1));
                } else if index == EFFECT_PAGE {
                    (address as *mut logos_core::native_service::EffectPage)
                        .write_volatile(logos_core::native_service::EffectPage::new(1, 1));
                } else if index == STORE_CLIENT_PAGE {
                    (address as *mut logos_core::native_service::StoreClientPage)
                        .write_volatile(logos_core::native_service::StoreClientPage::new(1, 1));
                } else if index == STORE_SERVER_PAGE {
                    (address as *mut logos_core::native_service::StoreServerPage)
                        .write_volatile(logos_core::native_service::StoreServerPage::new(1, 1));
                } else if index == NETWORK_DEVICE_ENDPOINT_PAGE {
                    (address as *mut logos_core::native_service::NetworkDevicePage).write_volatile(
                        logos_core::native_service::NetworkDevicePage::new(1, 1, 1),
                    );
                } else if index == NETWORK_EVENT_ENDPOINT_PAGE {
                    (address as *mut logos_core::native_service::NetworkEventPage)
                        .write_volatile(logos_core::native_service::NetworkEventPage::new(1, 1, 1));
                } else if index == NETWORK_STREAM_PAGE {
                    (address as *mut logos_core::native_service::StreamPage)
                        .write_volatile(logos_core::native_service::StreamPage::new(1, 1));
                } else if index == NETWORK_CLIENT_PAGE {
                    (address as *mut logos_core::native_service::NetworkClientPage)
                        .write_volatile(logos_core::native_service::NetworkClientPage::new(1, 1));
                } else if index == NETWORK_SERVER_PAGE {
                    (address as *mut logos_core::native_service::NetworkServerPage)
                        .write_volatile(logos_core::native_service::NetworkServerPage::new(1, 1));
                } else if index == REMOTE_PAGE {
                    (address as *mut logos_core::native_service::RemotePage)
                        .write_volatile(logos_core::native_service::RemotePage::new(1, 1));
                } else {
                    (address as *mut logos_core::native_service::BlockClientPage)
                        .write_volatile(logos_core::native_service::BlockClientPage::new(1, 1));
                }
                (self.pt.address() as *mut u64)
                    .add(index)
                    .write_volatile(address | PRESENT | WRITABLE | USER | NO_EXECUTE);
            }
            if let Err(page) = self.insert_mapping(index, page) {
                let _ = physical.release_page(page);
                self.unmap_index(CONTEXT_PAGE, physical);
                self.unmap_index(INPUT_PAGE, physical);
                self.unmap_index(DISPLAY_PAGE, physical);
                self.unmap_index(SESSION_CLIENT_PAGE, physical);
                self.unmap_index(SESSION_SERVER_PAGE, physical);
                self.unmap_index(EFFECT_PAGE, physical);
                self.unmap_index(STORE_CLIENT_PAGE, physical);
                self.unmap_index(STORE_SERVER_PAGE, physical);
                self.unmap_index(BLOCK_CLIENT_PAGE, physical);
                self.unmap_index(REMOTE_PAGE, physical);
                self.unmap_index(NETWORK_CLIENT_PAGE, physical);
                self.unmap_index(NETWORK_SERVER_PAGE, physical);
                self.unmap_index(NETWORK_DEVICE_ENDPOINT_PAGE, physical);
                self.unmap_index(NETWORK_EVENT_ENDPOINT_PAGE, physical);
                self.unmap_index(NETWORK_STREAM_PAGE, physical);
                return None;
            }
        }
        let context_virtual = self.base + PAGE_SIZE * CONTEXT_PAGE as u64;
        if !unsafe {
            logos_core::native_service::ControlPage::configure_endpoint_pages_at(
                context_address,
                1,
                input_virtual,
                display_virtual,
                session_client_virtual,
                session_server_virtual,
                effect_virtual,
                store_client_virtual,
                store_server_virtual,
                block_client_virtual,
                remote_virtual,
                network_client_virtual,
                network_server_virtual,
                network_device_virtual,
                network_event_virtual,
                network_stream_virtual,
            )
        } {
            self.unmap_index(CONTEXT_PAGE, physical);
            self.unmap_index(INPUT_PAGE, physical);
            self.unmap_index(DISPLAY_PAGE, physical);
            self.unmap_index(SESSION_CLIENT_PAGE, physical);
            self.unmap_index(SESSION_SERVER_PAGE, physical);
            self.unmap_index(EFFECT_PAGE, physical);
            self.unmap_index(STORE_CLIENT_PAGE, physical);
            self.unmap_index(STORE_SERVER_PAGE, physical);
            self.unmap_index(BLOCK_CLIENT_PAGE, physical);
            self.unmap_index(REMOTE_PAGE, physical);
            self.unmap_index(NETWORK_CLIENT_PAGE, physical);
            self.unmap_index(NETWORK_SERVER_PAGE, physical);
            self.unmap_index(NETWORK_DEVICE_ENDPOINT_PAGE, physical);
            self.unmap_index(NETWORK_EVENT_ENDPOINT_PAGE, physical);
            self.unmap_index(NETWORK_STREAM_PAGE, physical);
            return None;
        }
        Some(ContextMapping {
            context: (context_address, context_virtual),
            input: input_address.zip(input_virtual),
            display: display_address.zip(display_virtual),
            session_client: session_client_address.zip(session_client_virtual),
            session_server: session_server_address.zip(session_server_virtual),
            effect: effect_address.zip(effect_virtual),
            store_client: store_client_address.zip(store_client_virtual),
            store_server: store_server_address.zip(store_server_virtual),
            block_client: block_client_address.zip(block_client_virtual),
            remote: remote_address.zip(remote_virtual),
            network_client: network_client_address.zip(network_client_virtual),
            network_server: network_server_address.zip(network_server_virtual),
            network_device: network_device_address.zip(network_device_virtual),
            network_event: network_event_address.zip(network_event_virtual),
            network_stream: network_stream_address.zip(network_stream_virtual),
        })
    }

    pub fn map_shared_owned(&mut self, physical: &mut PhysicalMemory) -> Option<u64> {
        if self.mapping(SHARED_PAGE).is_some() || self.borrowed.is_some() {
            return None;
        }
        let page = physical.allocate_owned()?;
        let address = page.address();
        if let Err(page) = self.insert_mapping(SHARED_PAGE, page) {
            let _ = physical.release_page(page);
            return None;
        }
        unsafe {
            (self.pt.address() as *mut u64)
                .add(SHARED_PAGE)
                .write_volatile(address | PRESENT | WRITABLE | USER | NO_EXECUTE);
        }
        Some(address)
    }

    pub fn map_block_owned(&mut self, physical: &mut PhysicalMemory) -> Option<(u64, u64)> {
        if self.mapping(BLOCK_PAGE).is_some() {
            return None;
        }
        let page = physical.allocate_owned()?;
        let address = page.address();
        if let Err(page) = self.insert_mapping(BLOCK_PAGE, page) {
            let _ = physical.release_page(page);
            return None;
        }
        unsafe {
            (self.pt.address() as *mut u64)
                .add(BLOCK_PAGE)
                .write_volatile(address | PRESENT | WRITABLE | USER | NO_EXECUTE);
        }
        Some((address, self.base + PAGE_SIZE * BLOCK_PAGE as u64))
    }

    pub fn map_heap(&mut self, physical: &mut PhysicalMemory) -> Option<u64> {
        for index in HEAP_PAGE..HEAP_PAGE + HEAP_PAGES {
            if self.mapping(index).is_some() {
                return None;
            }
            let page = physical.allocate_owned()?;
            let address = page.address();
            if let Err(page) = self.insert_mapping(index, page) {
                let _ = physical.release_page(page);
                return None;
            }
            unsafe {
                (self.pt.address() as *mut u64)
                    .add(index)
                    .write_volatile(address | PRESENT | WRITABLE | USER | NO_EXECUTE);
            }
        }
        Some(self.base + PAGE_SIZE * HEAP_PAGE as u64)
    }

    pub fn map_shared_borrowed(&mut self, address: u64) -> bool {
        if address == 0
            || address % PAGE_SIZE != 0
            || self.mapping(SHARED_PAGE).is_some()
            || self.borrowed.is_some()
        {
            return false;
        }
        self.borrowed = Some(address);
        unsafe {
            (self.pt.address() as *mut u64)
                .add(SHARED_PAGE)
                .write_volatile(address | PRESENT | WRITABLE | USER | NO_EXECUTE);
        }
        true
    }

    pub fn remap_shared_borrowed(&mut self, address: u64) -> bool {
        if address == 0
            || address % PAGE_SIZE != 0
            || self.borrowed.is_none()
            || self.mapping(SHARED_PAGE).is_some()
        {
            return false;
        }
        self.borrowed = Some(address);
        unsafe {
            (self.pt.address() as *mut u64)
                .add(SHARED_PAGE)
                .write_volatile(address | PRESENT | WRITABLE | USER | NO_EXECUTE);
        }
        true
    }

    pub fn map_network_owned(
        &mut self,
        physical: &mut PhysicalMemory,
    ) -> Option<((u64, u64), (u64, u64))> {
        if self.mapping(NETWORK_RX_PAGE).is_some() || self.mapping(NETWORK_TX_PAGE).is_some() {
            return None;
        }
        let rx = physical.allocate_owned()?;
        let Some(tx) = physical.allocate_owned() else {
            let _ = physical.release_page(rx);
            return None;
        };
        let rx_address = rx.address();
        let tx_address = tx.address();
        if let Err(rx) = self.insert_mapping(NETWORK_RX_PAGE, rx) {
            let _ = physical.release_page(rx);
            let _ = physical.release_page(tx);
            return None;
        }
        if let Err(tx) = self.insert_mapping(NETWORK_TX_PAGE, tx) {
            self.unmap_index(NETWORK_RX_PAGE, physical);
            let _ = physical.release_page(tx);
            return None;
        }
        unsafe {
            (self.pt.address() as *mut u64)
                .add(NETWORK_RX_PAGE)
                .write_volatile(rx_address | PRESENT | WRITABLE | USER | NO_EXECUTE);
            (self.pt.address() as *mut u64)
                .add(NETWORK_TX_PAGE)
                .write_volatile(tx_address | PRESENT | WRITABLE | USER | NO_EXECUTE);
        }
        Some((
            (rx_address, self.base + PAGE_SIZE * NETWORK_RX_PAGE as u64),
            (tx_address, self.base + PAGE_SIZE * NETWORK_TX_PAGE as u64),
        ))
    }

    pub const fn cr3(&self) -> u64 {
        self.pml4.address()
    }

    pub fn stack_top(&self) -> u64 {
        self.base + PAGE_SIZE * STACK_TOP as u64
    }

    pub fn map_kernel_stack(&mut self, address: u64) -> bool {
        if address & (PAGE_SIZE - 1) != 0 {
            return false;
        }
        unsafe {
            (self.pt.address() as *mut u64)
                .add(ENTRIES - 3)
                .write_volatile(address | PRESENT | WRITABLE | NO_EXECUTE);
        }
        true
    }

    pub fn kernel_stack_top(&self) -> u64 {
        self.base + PAGE_SIZE * (ENTRIES - 2) as u64
    }

    pub fn verifies_isolation(&self) -> bool {
        unsafe {
            let pml4 = self.pml4.address() as *const u64;
            let slot = (self.base >> 39) as usize & 0x1ff;
            let entry = pml4.add(slot).read_volatile();
            entry & (PRESENT | USER) == PRESENT | USER
                && (STACK_BASE..STACK_TOP).all(|index| {
                    (self.pt.address() as *const u64).add(index).read_volatile()
                        & (PRESENT | WRITABLE | USER)
                        == PRESENT | WRITABLE | USER
                })
                && (0..ENTRIES)
                    .filter(|&index| index != slot)
                    .all(|index| pml4.add(index).read_volatile() & USER == 0)
        }
    }

    pub fn release(self, physical: &mut PhysicalMemory) -> bool {
        let mapped = self
            .mapped
            .into_iter()
            .flatten()
            .fold(true, |released, mapping| physical.release_page(mapping.page) && released);
        let stack = self.stack.release(physical);
        let stack_lower = physical.release_page(self.stack_lower);
        let pt = physical.release_page(self.pt);
        let pd = physical.release_page(self.pd);
        let pdpt = physical.release_page(self.pdpt);
        let pml4 = physical.release_page(self.pml4);
        mapped && stack && stack_lower && pt && pd && pdpt && pml4
    }

    fn map_page(
        &mut self,
        physical: &mut PhysicalMemory,
        payload: crate::platform::payload::Payload,
        index: usize,
        writable: bool,
        executable: bool,
    ) -> bool {
        let table = self.pt.address() as *mut u64;
        let entry = unsafe { table.add(index).read_volatile() };
        if self.mapping(index).is_none() {
            let rva = match u32::try_from(index * PAGE_SIZE as usize) {
                Ok(rva) => rva,
                Err(_) => return false,
            };
            let Some(page) = physical.allocate_owned() else {
                return false;
            };
            if !payload.copy_page(rva, page.address(), self.base) {
                let _ = physical.release_page(page);
                return false;
            }
            if let Err(page) = self.insert_mapping(index, page) {
                let _ = physical.release_page(page);
                return false;
            }
        }
        let Some(page) = self.mapping(index) else {
            return false;
        };
        let writable = writable || entry & WRITABLE != 0;
        let executable = executable || entry & NO_EXECUTE == 0 && entry & PRESENT != 0;
        let flags = PRESENT
            | USER
            | if writable { WRITABLE } else { 0 }
            | if executable { 0 } else { NO_EXECUTE };
        unsafe { table.add(index).write_volatile(page.address() | flags) };
        true
    }

    fn unmap_image(&mut self, physical: &mut PhysicalMemory) {
        for mapping in &mut self.mapped {
            if let Some(mapping) = mapping.take() {
                let _ = physical.release_page(mapping.page);
                unsafe { (self.pt.address() as *mut u64).add(mapping.index).write_volatile(0) };
            }
        }
    }

    fn unmap_index(&mut self, index: usize, physical: &mut PhysicalMemory) {
        if let Some(slot) = self
            .mapped
            .iter_mut()
            .find(|slot| slot.as_ref().is_some_and(|mapping| mapping.index == index))
        {
            if let Some(mapping) = slot.take() {
                let _ = physical.release_page(mapping.page);
                unsafe { (self.pt.address() as *mut u64).add(index).write_volatile(0) };
            }
        }
    }

    fn image_maps(&self, entry: u64) -> bool {
        let index = ((entry - self.base) / PAGE_SIZE) as usize;
        index < ENTRIES - 1 && self.mapping(index).is_some()
    }

    fn mapping(&self, index: usize) -> Option<&Page> {
        self.mapped
            .iter()
            .flatten()
            .find(|mapping| mapping.index == index)
            .map(|mapping| &mapping.page)
    }

    fn insert_mapping(&mut self, index: usize, page: Page) -> Result<(), Page> {
        let Some(slot) = self.mapped.iter_mut().find(|slot| slot.is_none()) else {
            return Err(page);
        };
        *slot = Some(Mapping { index, page });
        Ok(())
    }
}

fn enable_nx() -> bool {
    let (max_leaf, _) = cpuid(0x8000_0000);
    let (_, features) = cpuid(0x8000_0001);
    if max_leaf < 0x8000_0001 || features & (1 << 20) == 0 {
        return false;
    }
    let low: u32;
    let high: u32;
    unsafe {
        asm!("rdmsr", in("ecx") 0xc000_0080u32, lateout("eax") low, lateout("edx") high);
        asm!("wrmsr", in("ecx") 0xc000_0080u32, in("eax") low | (1 << 11), in("edx") high);
    }
    true
}

#[allow(unused_unsafe)]
fn cpuid(leaf: u32) -> (u32, u32) {
    let result = unsafe { __cpuid(leaf) };
    (result.eax, result.edx)
}

unsafe fn read_cr3() -> u64 {
    let value: u64;
    unsafe { asm!("mov {}, cr3", out(reg) value) };
    value & ADDRESS_MASK
}

const fn canonical_address(pml4_index: usize) -> u64 {
    ((pml4_index as u64) << 39) | 0xffff_0000_0000_0000
}
