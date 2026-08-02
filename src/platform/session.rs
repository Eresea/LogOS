use logos_core::capabilities::{Capability, CapabilityKind, CapabilityManager};

const CAPABILITIES: usize = 12;
const VARIABLES: usize = 4;
const VARIABLE_NAME: usize = 16;
const VARIABLE_VALUE: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Id(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Principal {
    LocalUser(u32),
    Service(u32),
    Process(u32),
}

impl Principal {
    pub const LOCAL: Self = Self::LocalUser(0);

    pub const fn service(id: u32) -> Self {
        Self::Service(id)
    }

    pub const fn process(id: u32) -> Self {
        Self::Process(id)
    }

    pub const fn page_owner(self) -> u64 {
        match self {
            Self::LocalUser(id) => id as u64,
            Self::Service(id) => (1_u64 << 32) | id as u64,
            Self::Process(id) => (2_u64 << 32) | id as u64,
        }
    }
}

pub struct Context {
    id: Id,
    principal: Principal,
    capabilities: [Option<Capability>; CAPABILITIES],
    length: usize,
    variables: [Variable; VARIABLES],
}

#[derive(Clone, Copy)]
struct Variable {
    name: [u8; VARIABLE_NAME],
    name_length: usize,
    value: [u8; VARIABLE_VALUE],
    value_length: usize,
}

impl Variable {
    const EMPTY: Self = Self {
        name: [0; VARIABLE_NAME],
        name_length: 0,
        value: [0; VARIABLE_VALUE],
        value_length: 0,
    };
}

impl Context {
    pub fn new(id: Id, principal: Principal, capabilities: &[Capability]) -> Option<Self> {
        if capabilities.len() > CAPABILITIES {
            return None;
        }
        let mut context = Self {
            id,
            principal,
            capabilities: [None; CAPABILITIES],
            length: capabilities.len(),
            variables: [Variable::EMPTY; VARIABLES],
        };
        for (slot, capability) in context.capabilities.iter_mut().zip(capabilities) {
            *slot = Some(*capability);
        }
        Some(context)
    }

    pub const fn id(&self) -> Id {
        self.id
    }

    pub const fn principal(&self) -> Principal {
        self.principal
    }

    pub fn allows(&self, manager: &CapabilityManager, kind: CapabilityKind) -> bool {
        self.allows_scoped(manager, kind, 0)
    }

    pub fn allows_scoped(
        &self,
        manager: &CapabilityManager,
        kind: CapabilityKind,
        resource: u32,
    ) -> bool {
        self.allows_scoped64(manager, kind, u64::from(resource))
    }

    pub fn allows_scoped64(
        &self,
        manager: &CapabilityManager,
        kind: CapabilityKind,
        resource: u64,
    ) -> bool {
        self.capabilities[..self.length]
            .iter()
            .flatten()
            .any(|capability| manager.allows_scoped64(*capability, kind, resource))
    }

    pub fn set_variable(&mut self, name: &[u8], value: &[u8]) -> bool {
        if name.is_empty()
            || name.len() > VARIABLE_NAME
            || value.len() > VARIABLE_VALUE
            || core::str::from_utf8(name).is_err()
            || core::str::from_utf8(value).is_err()
        {
            return false;
        }
        let slot = self.variables.iter_mut().find(|variable| {
            variable.name_length == 0 || variable.name[..variable.name_length] == *name
        });
        let Some(variable) = slot else {
            return false;
        };
        variable.name[..name.len()].copy_from_slice(name);
        variable.name_length = name.len();
        variable.value[..value.len()].copy_from_slice(value);
        variable.value_length = value.len();
        true
    }

    pub fn variable(&self, name: &[u8]) -> Option<&[u8]> {
        self.variables
            .iter()
            .find(|variable| variable.name[..variable.name_length] == *name)
            .map(|variable| &variable.value[..variable.value_length])
    }

    pub fn self_check() -> bool {
        let mut manager = CapabilityManager::new();
        let Some(debug) = manager.grant(CapabilityKind::Debug) else {
            return false;
        };
        let Some(recovery) = manager.grant(CapabilityKind::Recovery) else {
            return false;
        };
        let Some(mut context) = Self::new(Id(1), Principal::LOCAL, &[recovery]) else {
            return false;
        };
        context.id() == Id(1)
            && context.principal() == Principal::LOCAL
            && Principal::service(1) != Principal::process(1)
            && Principal::service(1).page_owner() != Principal::process(1).page_owner()
            && context.allows(&manager, CapabilityKind::Recovery)
            && !context.allows(&manager, CapabilityKind::Debug)
            && manager.revoke(recovery)
            && !context.allows(&manager, CapabilityKind::Recovery)
            && Self::new(Id(2), Principal::LOCAL, &[debug; CAPABILITIES + 1]).is_none()
            && context.set_variable(b"layout", b"qwerty")
            && context.variable(b"layout") == Some(b"qwerty" as &[u8])
    }
}

#[derive(Clone, Copy)]
pub enum Relay {
    Handled(bool),
    Recovery,
}

impl Relay {
    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub fn ok(self) -> bool {
        match self {
            Self::Handled(ok) => ok,
            Self::Recovery => true,
        }
    }
}

pub fn relay_native(
    terminal: crate::sched::native_task::SyscallEndpoint,
    sessions: Option<crate::sched::native_task::SessionEndpoint>,
    scheduler: &mut crate::sched::native_task::Scheduler<'_>,
    sessions_handle: Option<crate::sched::native_task::Handle>,
    context: crate::ipc::effects::Context<'_, '_>,
) -> Relay {
    let Some(request) = terminal.request() else {
        return Relay::Handled(true);
    };
    if !context.session.allows(context.capabilities, CapabilityKind::Session) {
        return Relay::Handled(terminal.reply(b"permission denied"));
    }
    let (Some(sessions), Some(sessions_handle)) = (sessions, sessions_handle) else {
        return Relay::Handled(terminal.reply(b"session unavailable"));
    };
    if !sessions.deliver(request)
        || !scheduler.wake(sessions_handle)
        || !scheduler.run(sessions_handle)
    {
        return Relay::Handled(false);
    }
    let Some(effect) = sessions.effect() else {
        return Relay::Handled(false);
    };
    let result = crate::ipc::effects::execute(effect, context);
    if !sessions.reply_effect(result)
        || !scheduler.wake(sessions_handle)
        || !scheduler.run(sessions_handle)
    {
        return Relay::Handled(false);
    }
    let Some(reply) = sessions.reply() else {
        return Relay::Handled(false);
    };
    if !terminal.reply(&reply.text[..reply.length])
        || !scheduler.wake(sessions_handle)
        || !scheduler.run(sessions_handle)
    {
        return Relay::Handled(false);
    }
    if result == logos_abi::EffectResult::Recovery { Relay::Recovery } else { Relay::Handled(true) }
}
