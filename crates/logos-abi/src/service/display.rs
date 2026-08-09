use super::*;

/// Fixed-size Display endpoint page.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct DisplayPage {
    pub generation: u32,
    pub state: u32,
    pub operation: u32,
    pub x: u32,
    pub y: u32,
    pub color: u32,
    pub text_length: u32,
    pub text: [u8; MAX_TEXT],
    pub reserved: [u8; logos_abi::PAGE_SIZE - 284],
}

impl DisplayPage {
    pub const fn new(generation: u32) -> Self {
        Self {
            generation,
            state: EndpointState::Ready.wire(),
            operation: 0,
            x: 0,
            y: 0,
            color: 0,
            text_length: 0,
            text: [0; MAX_TEXT],
            reserved: [0; logos_abi::PAGE_SIZE - 284],
        }
    }

    /// # Safety
    /// `address` must point to a live, aligned DisplayPage mapping.
    pub unsafe fn reset_at(address: u64, generation: u32) -> bool {
        if address == 0 || generation == 0 || !address.is_multiple_of(align_of::<Self>() as u64) {
            return false;
        }
        unsafe { (address as *mut Self).write_volatile(Self::new(generation)) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned DisplayPage mapping owned by the service.
    pub unsafe fn request_pixel_at(
        address: u64,
        generation: u32,
        x: u32,
        y: u32,
        color: logos_abi::DisplayColor,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Ready)
        {
            return false;
        }
        page.operation = PRESENT_PIXEL;
        page.x = x;
        page.y = y;
        page.color = color.wire();
        page.text_length = 0;
        page.state = EndpointState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned DisplayPage mapping owned by the service.
    pub unsafe fn request_text_at(
        address: u64,
        generation: u32,
        x: u32,
        y: u32,
        color: logos_abi::DisplayColor,
        text: &[u8],
    ) -> bool {
        if text.len() > MAX_TEXT {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Ready)
        {
            return false;
        }
        page.operation = PRESENT_TEXT;
        page.x = x;
        page.y = y;
        page.color = color.wire();
        page.text = [0; MAX_TEXT];
        page.text[..text.len()].copy_from_slice(text);
        page.text_length = text.len() as u32;
        page.state = EndpointState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned DisplayPage mapping owned by the service.
    pub unsafe fn request_clear_at(address: u64, generation: u32) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Ready)
        {
            return false;
        }
        page.operation = CLEAR_DISPLAY;
        page.text_length = 0;
        page.state = EndpointState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned DisplayPage mapping.
    pub unsafe fn pending_at(address: u64, generation: u32) -> bool {
        if address == 0 {
            return false;
        }
        let page = address as *const Self;
        let page_generation = unsafe { core::ptr::addr_of!((*page).generation).read_volatile() };
        let state = unsafe { core::ptr::addr_of!((*page).state).read_volatile() };
        page_generation == generation
            && EndpointState::from_wire(state) == Some(EndpointState::Request)
    }

    /// # Safety
    /// `address` must point to a live, aligned DisplayPage mapping owned by Core.
    pub unsafe fn request_at(address: u64, generation: u32) -> Option<DisplayRequest> {
        let page = unsafe { (address as *const Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Request)
            || !matches!(page.operation, PRESENT_PIXEL | PRESENT_TEXT | CLEAR_DISPLAY)
        {
            return None;
        }
        let color = logos_abi::DisplayColor::from_wire(page.color)?;
        let length = usize::try_from(page.text_length).ok()?;
        (length <= MAX_TEXT).then_some(DisplayRequest {
            operation: page.operation,
            x: page.x,
            y: page.y,
            color,
            text: page.text,
            length,
        })
    }

    /// # Safety
    /// `address` must point to a live, aligned DisplayPage mapping owned by Core.
    pub unsafe fn complete_at(address: u64, generation: u32) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Request)
        {
            return false;
        }
        page.state = EndpointState::Complete.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned DisplayPage mapping owned by the service.
    pub unsafe fn finish_at(address: u64, generation: u32) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Complete)
        {
            return false;
        }
        page.state = EndpointState::Ready.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }
}

#[derive(Clone, Copy)]
pub struct DisplayRequest {
    pub operation: u32,
    pub x: u32,
    pub y: u32,
    pub color: logos_abi::DisplayColor,
    pub text: [u8; MAX_TEXT],
    pub length: usize,
}
