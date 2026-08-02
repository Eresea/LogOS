use crate::{
    ipc::approvals::{Id as ApprovalId, Store as Approvals},
    platform::{audit::Log, session::Principal},
};
use logos_core::capabilities::{CapabilityKind, CapabilityManager};

const MODELS: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ModelId(pub u8);

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Accelerator(pub u8);

#[derive(Clone, Copy)]
pub struct Request {
    pub principal: Principal,
    pub expires: u64,
    pub now: u64,
}

#[derive(Clone, Copy)]
struct Model {
    id: ModelId,
    accelerator: Option<Accelerator>,
}

pub struct Service {
    models: [Option<Model>; MODELS],
}

impl Service {
    pub const fn new() -> Self {
        Self { models: [None; MODELS] }
    }

    pub fn register(&mut self, id: ModelId) -> bool {
        let Some(slot) = self.models.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        *slot = Some(Model { id, accelerator: None });
        true
    }

    pub fn bind(&mut self, id: ModelId, accelerator: Accelerator) -> bool {
        let Some(model) = self.models.iter_mut().flatten().find(|model| model.id == id) else {
            return false;
        };
        model.accelerator = Some(accelerator);
        true
    }

    pub fn invoke(
        &self,
        id: ModelId,
        request: Request,
        approvals: &mut Approvals,
        capabilities: &mut CapabilityManager,
        audit: &mut Log,
    ) -> Option<ApprovalId> {
        self.models
            .iter()
            .flatten()
            .find(|model| model.id == id && model.accelerator.is_some())
            .and_then(|_| {
                approvals.grant(
                    capabilities,
                    request.principal,
                    CapabilityKind::Inference,
                    request.expires,
                    request.now,
                    audit,
                )
            })
    }
}

pub fn self_check() -> bool {
    let mut service = Service::new();
    let model = ModelId(1);
    let mut approvals = Approvals::new();
    let mut capabilities = CapabilityManager::new();
    let mut audit = Log::new();
    service.register(model)
        && service.bind(model, Accelerator(0))
        && service
            .invoke(
                model,
                Request { principal: Principal::process(1), expires: 12, now: 10 },
                &mut approvals,
                &mut capabilities,
                &mut audit,
            )
            .is_some_and(|grant| {
                approvals.allows(
                    &capabilities,
                    grant,
                    Principal::process(1),
                    CapabilityKind::Inference,
                    11,
                )
            })
}
