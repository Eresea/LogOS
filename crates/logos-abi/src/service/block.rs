use super::*;

/// Core-mediated Block client page. Only Storage maps it.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct BlockClientPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub operation: u32,
    pub lba: u64,
    pub blocks: u32,
    pub page: u32,
    pub deadline: u64,
    pub transfer_page: u32,
    pub reply_status: u32,
    pub logical_block_size: u32,
    pub block_count: u64,
    pub max_transfer_blocks: u32,
}

#[allow(clippy::missing_safety_doc)]
impl BlockClientPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: PersistencePageState::Ready.wire(),
            request_id: 0,
            operation: 0,
            lba: 0,
            blocks: 0,
            page: 0,
            deadline: 0,
            transfer_page: 0,
            reply_status: 0,
            logical_block_size: 0,
            block_count: 0,
            max_transfer_blocks: 0,
        }
    }

    pub unsafe fn reset_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_page_identity::<Self>(address, service_generation, endpoint_generation) {
            return false;
        }
        let old = unsafe { (address as *const Self).read_volatile() };
        let mut page = Self::new(service_generation, endpoint_generation);
        page.transfer_page = old.transfer_page;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn configure_transfer_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        handle: logos_abi::PageHandle,
    ) -> bool {
        if handle.0 == 0 {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Ready)
        {
            return false;
        }
        page.transfer_page = handle.0;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn transfer_page_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<logos_abi::PageHandle> {
        let page = unsafe { (address as *const Self).read_volatile() };
        (client_identity(&page, service_generation, endpoint_generation) && page.transfer_page != 0)
            .then_some(logos_abi::PageHandle(page.transfer_page))
    }

    pub unsafe fn request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        request: logos_abi::BlockRequest,
    ) -> bool {
        if request.id == 0 || !request.valid_shape() {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Ready)
        {
            return false;
        }
        page.request_id = request.id;
        page.operation = request.operation as u32;
        page.lba = request.lba;
        page.blocks = request.blocks;
        page.page = request.page.0;
        page.deadline = request.deadline;
        page.reply_status = 0;
        page.logical_block_size = 0;
        page.block_count = 0;
        page.max_transfer_blocks = 0;
        page.state = PersistencePageState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn take_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<logos_abi::BlockRequest> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Request)
        {
            return None;
        }
        let request = block_request_from_fields(
            page.request_id,
            page.operation,
            page.lba,
            page.blocks,
            page.page,
            page.deadline,
        )?;
        page.state = PersistencePageState::Submitted.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(request)
    }

    pub unsafe fn request_at_current(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<logos_abi::BlockRequest> {
        let page = unsafe { (address as *const Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Submitted)
        {
            return None;
        }
        block_request_from_fields(
            page.request_id,
            page.operation,
            page.lba,
            page.blocks,
            page.page,
            page.deadline,
        )
    }

    pub unsafe fn pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        client_identity(&page, service_generation, endpoint_generation)
            && PersistencePageState::from_wire(page.state) == Some(PersistencePageState::Request)
    }

    pub unsafe fn reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        reply: logos_abi::BlockReply,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let Some(request) =
            (unsafe { Self::request_at_current(address, service_generation, endpoint_generation) })
        else {
            return false;
        };
        if !reply.valid_for(request) {
            return false;
        }
        page.reply_status = reply.status as u32;
        page.logical_block_size = reply.info.logical_block_size;
        page.block_count = reply.info.blocks;
        page.max_transfer_blocks = reply.info.max_transfer_blocks;
        page.state = super::storage::persistence_state(reply.status).wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn finish_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
    ) -> Option<logos_abi::BlockReply> {
        let page = unsafe { (address as *const Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || !matches!(
                PersistencePageState::from_wire(page.state),
                Some(
                    PersistencePageState::Reply
                        | PersistencePageState::Denied
                        | PersistencePageState::Failed
                        | PersistencePageState::Cancelled
                        | PersistencePageState::TimedOut
                )
            )
        {
            return None;
        }
        let reply = logos_abi::BlockReply {
            id: page.request_id,
            status: logos_abi::PersistenceStatus::from_wire(u8::try_from(page.reply_status).ok()?)?,
            info: logos_abi::BlockInfo {
                logical_block_size: page.logical_block_size,
                blocks: page.block_count,
                max_transfer_blocks: page.max_transfer_blocks,
            },
        };
        unsafe { Self::reset_at(address, service_generation, endpoint_generation) };
        Some(reply)
    }
}

fn block_request_from_fields(
    id: u32,
    operation: u32,
    lba: u64,
    blocks: u32,
    page: u32,
    deadline: u64,
) -> Option<logos_abi::BlockRequest> {
    let request = logos_abi::BlockRequest {
        id,
        operation: logos_abi::BlockOperation::from_wire(u8::try_from(operation).ok()?)?,
        lba,
        blocks,
        page: logos_abi::PageHandle(page),
        deadline,
    };
    request.valid_shape().then_some(request)
}
