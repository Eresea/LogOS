use super::*;

/// Fixed-size Input endpoint page.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct InputPage {
    pub generation: u32,
    pub state: u32,
    pub event: u32,
    pub reserved: [u8; logos_abi::PAGE_SIZE - 12],
}

impl InputPage {
    pub const fn new(generation: u32) -> Self {
        Self {
            generation,
            state: EndpointState::Ready.wire(),
            event: 0,
            reserved: [0; logos_abi::PAGE_SIZE - 12],
        }
    }

    /// # Safety
    /// `address` must point to a live, aligned InputPage mapping.
    pub unsafe fn reset_at(address: u64, generation: u32) -> bool {
        if address == 0 || generation == 0 || !address.is_multiple_of(align_of::<Self>() as u64) {
            return false;
        }
        unsafe { (address as *mut Self).write_volatile(Self::new(generation)) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned InputPage mapping.
    pub unsafe fn wait_at(address: u64, generation: u32) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Ready)
        {
            return false;
        }
        page.state = EndpointState::Waiting.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned InputPage mapping.
    pub unsafe fn waiting_at(address: u64, generation: u32) -> bool {
        if address == 0 {
            return false;
        }
        let page = address as *const Self;
        let page_generation = unsafe { core::ptr::addr_of!((*page).generation).read_volatile() };
        let state = unsafe { core::ptr::addr_of!((*page).state).read_volatile() };
        page_generation == generation
            && EndpointState::from_wire(state) == Some(EndpointState::Waiting)
    }

    /// # Safety
    /// `address` must point to a live, aligned InputPage mapping owned by Core.
    pub unsafe fn deliver_at(address: u64, generation: u32, event: u8) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Waiting)
        {
            return false;
        }
        page.event = u32::from(event);
        page.state = EndpointState::Reply.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned InputPage mapping owned by the service.
    pub unsafe fn take_at(address: u64, generation: u32) -> Option<u8> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Reply)
        {
            return None;
        }
        let event = u8::try_from(page.event).ok()?;
        page.event = 0;
        page.state = EndpointState::Ready.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(event)
    }
}
