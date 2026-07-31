pub const NAME: &[u8] = b"virtio-block";
pub const SERVICE: crate::platform::services::Service =
    crate::platform::services::Service::VirtioBlock;

const BLOCK: crate::device::DriverManifest = crate::device::DriverManifest {
    interface: crate::device::Interface::new(crate::device::Class::Block),
    vendor_id: 0x1af4,
    device_id: 0x1001,
    capabilities: &[
        logos_core::capabilities::CapabilityKind::Block,
        logos_core::capabilities::CapabilityKind::Memory,
    ],
};

pub fn discover(devices: &crate::pci::PciDevices) -> Option<crate::pci::PciDevice> {
    crate::device::bind(&[BLOCK], BLOCK.vendor_id, BLOCK.device_id)
        .and_then(|manifest| devices.find(manifest.vendor_id, manifest.device_id))
}

pub struct Dispatch {
    pending: Option<logos_abi::BlockRequest>,
}

pub struct DispatchContext<'a> {
    pub endpoint: crate::sched::native_task::BlockEndpoint,
    pub pages: &'a mut logos_core::shared_pages::SharedPages,
    pub store_owner: u64,
    pub store_page: logos_abi::PageHandle,
    pub device: &'a mut crate::drivers::block::Device,
    pub memory: &'a mut crate::mm::memory::PhysicalMemory,
}

impl Dispatch {
    pub const fn new() -> Self {
        Self { pending: None }
    }

    pub fn poll(
        &mut self,
        context: &mut DispatchContext<'_>,
        tick: u64,
    ) -> Option<logos_abi::BlockReply> {
        if let Some(request) = self.pending {
            let status = context.device.complete(context.memory).or_else(|| {
                (tick >= request.deadline).then(|| context.device.timeout(context.memory))
            })?;
            self.pending = None;
            return Some(reply(request, status));
        }

        let request = context.endpoint.request()?;
        let info = context.device.info();
        if !request.valid(info) {
            return Some(reply(request, logos_abi::PersistenceStatus::Invalid));
        }
        if request.operation == logos_abi::BlockOperation::Info {
            return Some(logos_abi::BlockReply {
                id: request.id,
                status: logos_abi::PersistenceStatus::Complete,
                info,
            });
        }
        let page = match request.operation {
            logos_abi::BlockOperation::Read | logos_abi::BlockOperation::Write => {
                if request.page != context.store_page {
                    return Some(reply(request, logos_abi::PersistenceStatus::Denied));
                }
                let Some(address) = context.pages.address(context.store_owner, request.page) else {
                    return Some(reply(request, logos_abi::PersistenceStatus::Denied));
                };
                Some(address)
            }
            logos_abi::BlockOperation::Flush
            | logos_abi::BlockOperation::Cancel
            | logos_abi::BlockOperation::Reset => None,
            logos_abi::BlockOperation::Info => unreachable!(),
        };
        if tick >= request.deadline {
            return Some(reply(request, context.device.timeout(context.memory)));
        }
        let status = context.device.submit(request, page, context.memory);
        if status == logos_abi::PersistenceStatus::Complete
            && matches!(
                request.operation,
                logos_abi::BlockOperation::Read
                    | logos_abi::BlockOperation::Write
                    | logos_abi::BlockOperation::Flush
            )
        {
            self.pending = Some(request);
            None
        } else {
            Some(reply(request, status))
        }
    }

    pub const fn accepts_new_request(&self) -> bool {
        self.pending.is_none()
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub fn cancel_on_exit(&mut self, context: &mut DispatchContext<'_>) {
        if self.pending.take().is_some() {
            let _ = context.device.timeout(context.memory);
        } else {
            let _ = context.device.reset();
        }
    }
}

fn reply(
    request: logos_abi::BlockRequest,
    status: logos_abi::PersistenceStatus,
) -> logos_abi::BlockReply {
    logos_abi::BlockReply { id: request.id, status, info: logos_abi::BlockInfo::default() }
}

pub fn self_check() -> bool {
    let request = logos_abi::BlockRequest {
        id: 1,
        operation: logos_abi::BlockOperation::Flush,
        lba: 0,
        blocks: 0,
        page: logos_abi::PageHandle(0),
        deadline: 1,
    };
    reply(request, logos_abi::PersistenceStatus::Complete).valid_for(request)
        && Dispatch::new().accepts_new_request()
        && !Dispatch { pending: Some(request) }.accepts_new_request()
}
