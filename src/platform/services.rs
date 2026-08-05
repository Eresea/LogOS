use logos_core::capabilities::{Capability, CapabilityKind, CapabilityManager};

const SERVICES: usize = 8;

pub const SUPERVISOR: &[u8] = b"supervisor";
pub const VIRTIO_BALLOON: &[u8] = b"virtio-balloon";
pub const VIRTIO_BLOCK: &[u8] = b"virtio-block";
pub const STORAGE: &[u8] = b"storage";
pub const TERMINAL: &[u8] = b"terminal";
pub const SESSIONS: &[u8] = b"sessions";
pub const NETWORK: &[u8] = b"network";
pub const GATEWAY: &[u8] = b"gateway";

#[derive(Clone, Copy)]
pub enum Profile {
    Normal,
    Recovery,
    Diagnostics,
}

#[derive(Clone, Copy)]
pub struct Profiles(u8);

impl Profiles {
    const NORMAL: u8 = 1;
    const RECOVERY: u8 = 2;
    const DIAGNOSTICS: u8 = 4;

    pub const ALL: Self = Self(Self::NORMAL | Self::RECOVERY | Self::DIAGNOSTICS);
    pub const NORMAL_RECOVERY: Self = Self(Self::NORMAL | Self::RECOVERY);
    pub const NORMAL_ONLY: Self = Self(Self::NORMAL);

    pub const fn includes(self, profile: Profile) -> bool {
        self.0
            & match profile {
                Profile::Normal => Self::NORMAL,
                Profile::Recovery => Self::RECOVERY,
                Profile::Diagnostics => Self::DIAGNOSTICS,
            }
            != 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RecoveryClass {
    Restartable,
    Resettable,
    Fatal,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Protocol {
    pub abi: u16,
    pub version: u16,
}

#[derive(Clone, Copy)]
pub struct RestartPolicy {
    pub retries: u8,
    pub backoff_ticks: u64,
}

/// Endpoint pages granted to a native service by the bootstrap mapper.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EndpointSet(u16);

impl EndpointSet {
    const INPUT_BIT: u16 = 1 << 0;
    const DISPLAY_BIT: u16 = 1 << 1;
    const SESSION_CLIENT_BIT: u16 = 1 << 2;
    const SESSION_SERVER_BIT: u16 = 1 << 3;
    const EFFECT_BIT: u16 = 1 << 4;
    const STORE_BIT: u16 = 1 << 5;
    const BLOCK_BIT: u16 = 1 << 6;
    const NETWORK_BIT: u16 = 1 << 7;
    const REMOTE_BIT: u16 = 1 << 8;

    pub const NONE: Self = Self(0);
    pub const INPUT: Self = Self(Self::INPUT_BIT);
    pub const DISPLAY: Self = Self(Self::DISPLAY_BIT);
    pub const SESSION_CLIENT: Self = Self(Self::SESSION_CLIENT_BIT);
    pub const SESSION_SERVER: Self = Self(Self::SESSION_SERVER_BIT);
    pub const EFFECT: Self = Self(Self::EFFECT_BIT);
    pub const STORE: Self = Self(Self::STORE_BIT);
    pub const BLOCK: Self = Self(Self::BLOCK_BIT);
    pub const NETWORK: Self = Self(Self::NETWORK_BIT);
    pub const REMOTE: Self = Self(Self::REMOTE_BIT);

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, endpoint: Self) -> bool {
        self.0 & endpoint.0 == endpoint.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Supervisor,
    VirtioBalloon,
    VirtioBlock,
    Storage,
    Terminal,
    Sessions,
    Network,
    Gateway,
}

#[derive(Clone, Copy)]
pub struct ServiceSpec {
    pub service: Service,
    pub name: &'static [u8],
    pub dependencies: &'static [&'static [u8]],
    pub capabilities: &'static [CapabilityKind],
    pub protocol: Protocol,
    pub restart: RestartPolicy,
    pub recovery: RecoveryClass,
    pub profiles: Profiles,
    pub endpoints: EndpointSet,
}

const SUPERVISOR_SPEC: ServiceSpec = ServiceSpec {
    service: Service::Supervisor,
    name: SUPERVISOR,
    dependencies: &[],
    capabilities: &[],
    protocol: Protocol { abi: 1, version: 0 },
    restart: RestartPolicy { retries: 0, backoff_ticks: 0 },
    recovery: RecoveryClass::Fatal,
    profiles: Profiles::ALL,
    endpoints: EndpointSet::NONE,
};
const VIRTIO_BALLOON_SPEC: ServiceSpec = ServiceSpec {
    service: Service::VirtioBalloon,
    name: VIRTIO_BALLOON,
    dependencies: &[SUPERVISOR],
    capabilities: &[CapabilityKind::Service],
    protocol: Protocol { abi: 1, version: 0 },
    restart: RestartPolicy { retries: 3, backoff_ticks: 2 },
    recovery: RecoveryClass::Resettable,
    profiles: Profiles::NORMAL_RECOVERY,
    endpoints: EndpointSet::NONE,
};
const VIRTIO_BLOCK_SPEC: ServiceSpec = ServiceSpec {
    service: Service::VirtioBlock,
    name: VIRTIO_BLOCK,
    dependencies: &[SUPERVISOR],
    capabilities: &[CapabilityKind::Service, CapabilityKind::Block, CapabilityKind::Memory],
    protocol: Protocol { abi: 1, version: 0 },
    restart: RestartPolicy { retries: 3, backoff_ticks: 2 },
    recovery: RecoveryClass::Resettable,
    profiles: Profiles::NORMAL_RECOVERY,
    endpoints: EndpointSet::NONE,
};
const STORAGE_SPEC: ServiceSpec = ServiceSpec {
    service: Service::Storage,
    name: STORAGE,
    dependencies: &[VIRTIO_BLOCK],
    capabilities: &[
        CapabilityKind::Service,
        CapabilityKind::Memory,
        CapabilityKind::StoreRead,
        CapabilityKind::StoreWrite,
    ],
    protocol: Protocol { abi: 1, version: 0 },
    restart: RestartPolicy { retries: 3, backoff_ticks: 2 },
    recovery: RecoveryClass::Restartable,
    profiles: Profiles::NORMAL_RECOVERY,
    endpoints: EndpointSet::STORE.union(EndpointSet::BLOCK),
};
const TERMINAL_SPEC: ServiceSpec = ServiceSpec {
    service: Service::Terminal,
    name: TERMINAL,
    dependencies: &[SUPERVISOR],
    capabilities: &[CapabilityKind::Service, CapabilityKind::Input, CapabilityKind::Display],
    protocol: Protocol { abi: 1, version: 0 },
    restart: RestartPolicy { retries: 1, backoff_ticks: 0 },
    recovery: RecoveryClass::Restartable,
    profiles: Profiles::NORMAL_ONLY,
    endpoints: EndpointSet::INPUT
        .union(EndpointSet::DISPLAY)
        .union(EndpointSet::SESSION_CLIENT)
        .union(EndpointSet::STORE)
        .union(EndpointSet::NETWORK),
};
const SESSIONS_SPEC: ServiceSpec = ServiceSpec {
    service: Service::Sessions,
    name: SESSIONS,
    dependencies: &[SUPERVISOR],
    capabilities: &[CapabilityKind::Service, CapabilityKind::Session],
    protocol: Protocol { abi: 1, version: 0 },
    restart: RestartPolicy { retries: 3, backoff_ticks: 2 },
    recovery: RecoveryClass::Restartable,
    profiles: Profiles::NORMAL_ONLY,
    endpoints: EndpointSet::SESSION_SERVER.union(EndpointSet::EFFECT),
};
const NETWORK_SPEC: ServiceSpec = ServiceSpec {
    service: Service::Network,
    name: NETWORK,
    dependencies: &[SUPERVISOR],
    capabilities: &[
        CapabilityKind::Service,
        CapabilityKind::NetworkBind,
        CapabilityKind::NetworkSend,
        CapabilityKind::NetworkReceive,
    ],
    protocol: Protocol { abi: 1, version: 0 },
    restart: RestartPolicy { retries: 3, backoff_ticks: 2 },
    recovery: RecoveryClass::Restartable,
    profiles: Profiles::NORMAL_ONLY,
    endpoints: EndpointSet::NETWORK,
};
const GATEWAY_SPEC: ServiceSpec = ServiceSpec {
    service: Service::Gateway,
    name: GATEWAY,
    dependencies: &[STORAGE, SESSIONS, NETWORK],
    capabilities: &[
        CapabilityKind::Service,
        CapabilityKind::NetworkBind,
        CapabilityKind::NetworkSend,
        CapabilityKind::NetworkReceive,
    ],
    protocol: Protocol { abi: 1, version: 0 },
    restart: RestartPolicy { retries: 3, backoff_ticks: 2 },
    recovery: RecoveryClass::Restartable,
    profiles: Profiles::NORMAL_ONLY,
    endpoints: EndpointSet::NETWORK.union(EndpointSet::REMOTE).union(EndpointSet::STORE),
};

/// The single typed service specification consumed by boot planning, payload
/// staging, and service lookup.
pub const SERVICE_SPECS: &[ServiceSpec] = &[
    SUPERVISOR_SPEC,
    VIRTIO_BALLOON_SPEC,
    VIRTIO_BLOCK_SPEC,
    STORAGE_SPEC,
    TERMINAL_SPEC,
    SESSIONS_SPEC,
    NETWORK_SPEC,
    GATEWAY_SPEC,
];

impl Service {
    pub const fn spec(self) -> &'static ServiceSpec {
        match self {
            Self::Supervisor => &SUPERVISOR_SPEC,
            Self::VirtioBalloon => &VIRTIO_BALLOON_SPEC,
            Self::VirtioBlock => &VIRTIO_BLOCK_SPEC,
            Self::Storage => &STORAGE_SPEC,
            Self::Terminal => &TERMINAL_SPEC,
            Self::Sessions => &SESSIONS_SPEC,
            Self::Network => &NETWORK_SPEC,
            Self::Gateway => &GATEWAY_SPEC,
        }
    }

    pub const fn protocol(self) -> Protocol {
        self.spec().protocol
    }
}

pub fn self_check() -> bool {
    SERVICE_SPECS.iter().all(|spec| spec.service.spec().name == spec.name)
        && SERVICE_SPECS.len() == SERVICES
        && Service::Terminal.spec().protocol == Protocol { abi: 1, version: 0 }
        && Service::Storage.spec().endpoints.contains(EndpointSet::STORE)
        && Service::Terminal.spec().endpoints.contains(EndpointSet::SESSION_CLIENT)
        && Service::Sessions.spec().endpoints.contains(EndpointSet::SESSION_SERVER)
        && Service::Sessions.spec().endpoints.contains(EndpointSet::EFFECT)
        && Service::VirtioBlock.spec().endpoints.contains(EndpointSet::BLOCK)
        && Service::Gateway.spec().endpoints.contains(EndpointSet::REMOTE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceHandle(u8);

impl ServiceHandle {
    pub(crate) const fn self_check() -> Self {
        Self(0)
    }

    pub const fn principal(self) -> crate::platform::session::Principal {
        crate::platform::session::Principal::service(self.0 as u32)
    }
}

pub struct Registry {
    services: [Option<Service>; SERVICES],
}

impl Registry {
    pub const fn new() -> Self {
        Self { services: [None; SERVICES] }
    }

    pub fn register(
        &mut self,
        capabilities: &CapabilityManager,
        capability: Capability,
        service: Service,
    ) -> Option<ServiceHandle> {
        if !capabilities.allows(capability, CapabilityKind::Service)
            || self.resolve(service).is_some()
        {
            return None;
        }
        for (index, slot) in self.services.iter_mut().enumerate() {
            if slot.is_none() {
                // ponytail: fixed registry; add dynamic lifecycle management with real services.
                *slot = Some(service);
                return Some(ServiceHandle(index as u8));
            }
        }
        None
    }

    pub fn resolve(&self, service: Service) -> Option<ServiceHandle> {
        self.services
            .iter()
            .position(|slot| *slot == Some(service))
            .map(|index| ServiceHandle(index as u8))
    }
}
