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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EndpointKind {
    Input,
    Display,
    SessionClient,
    SessionServer,
    Effect,
    StoreClient,
    StoreServer,
    BlockClient,
    Remote,
    NetworkClient,
    NetworkServer,
    NetworkDevice,
    NetworkEvent,
    NetworkStream,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EndpointRole {
    Client,
    Server,
    Device,
    Event,
    Shared,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EndpointPermissions(u8);

impl EndpointPermissions {
    pub const READ_WRITE: Self = Self(0b11);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EndpointDescriptor {
    pub kind: EndpointKind,
    pub role: EndpointRole,
    pub permissions: EndpointPermissions,
}

impl EndpointDescriptor {
    pub const fn new(kind: EndpointKind, role: EndpointRole) -> Self {
        Self { kind, role, permissions: EndpointPermissions::READ_WRITE }
    }
}

pub fn has_endpoint(endpoints: &[EndpointDescriptor], kind: EndpointKind) -> bool {
    let mut index = 0;
    while index < endpoints.len() {
        if endpoints[index].kind == kind {
            return true;
        }
        index += 1;
    }
    false
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
    pub endpoints: &'static [EndpointDescriptor],
}

const STORAGE_ENDPOINTS: &[EndpointDescriptor] = &[
    EndpointDescriptor::new(EndpointKind::StoreServer, EndpointRole::Server),
    EndpointDescriptor::new(EndpointKind::BlockClient, EndpointRole::Client),
];
const TERMINAL_ENDPOINTS: &[EndpointDescriptor] = &[
    EndpointDescriptor::new(EndpointKind::Input, EndpointRole::Client),
    EndpointDescriptor::new(EndpointKind::Display, EndpointRole::Client),
    EndpointDescriptor::new(EndpointKind::SessionClient, EndpointRole::Client),
    EndpointDescriptor::new(EndpointKind::StoreClient, EndpointRole::Client),
    EndpointDescriptor::new(EndpointKind::NetworkClient, EndpointRole::Client),
    EndpointDescriptor::new(EndpointKind::NetworkStream, EndpointRole::Shared),
];
const SESSIONS_ENDPOINTS: &[EndpointDescriptor] = &[
    EndpointDescriptor::new(EndpointKind::SessionServer, EndpointRole::Server),
    EndpointDescriptor::new(EndpointKind::Effect, EndpointRole::Server),
];
const NETWORK_ENDPOINTS: &[EndpointDescriptor] = &[
    EndpointDescriptor::new(EndpointKind::NetworkServer, EndpointRole::Server),
    EndpointDescriptor::new(EndpointKind::NetworkDevice, EndpointRole::Device),
    EndpointDescriptor::new(EndpointKind::NetworkEvent, EndpointRole::Event),
    EndpointDescriptor::new(EndpointKind::NetworkStream, EndpointRole::Shared),
];
const GATEWAY_ENDPOINTS: &[EndpointDescriptor] = &[
    EndpointDescriptor::new(EndpointKind::NetworkClient, EndpointRole::Client),
    EndpointDescriptor::new(EndpointKind::NetworkStream, EndpointRole::Shared),
    EndpointDescriptor::new(EndpointKind::Remote, EndpointRole::Client),
    EndpointDescriptor::new(EndpointKind::StoreClient, EndpointRole::Client),
];

const SUPERVISOR_SPEC: ServiceSpec = ServiceSpec {
    service: Service::Supervisor,
    name: SUPERVISOR,
    dependencies: &[],
    capabilities: &[],
    protocol: Protocol { abi: 1, version: 0 },
    restart: RestartPolicy { retries: 0, backoff_ticks: 0 },
    recovery: RecoveryClass::Fatal,
    profiles: Profiles::ALL,
    endpoints: &[],
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
    endpoints: &[],
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
    endpoints: &[],
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
    endpoints: STORAGE_ENDPOINTS,
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
    endpoints: TERMINAL_ENDPOINTS,
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
    endpoints: SESSIONS_ENDPOINTS,
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
    endpoints: NETWORK_ENDPOINTS,
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
    endpoints: GATEWAY_ENDPOINTS,
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
        && has_endpoint(Service::Storage.spec().endpoints, EndpointKind::StoreServer)
        && has_endpoint(Service::Terminal.spec().endpoints, EndpointKind::SessionClient)
        && has_endpoint(Service::Sessions.spec().endpoints, EndpointKind::SessionServer)
        && has_endpoint(Service::Sessions.spec().endpoints, EndpointKind::Effect)
        && has_endpoint(Service::Network.spec().endpoints, EndpointKind::NetworkDevice)
        && has_endpoint(Service::Network.spec().endpoints, EndpointKind::NetworkEvent)
        && has_endpoint(Service::Network.spec().endpoints, EndpointKind::NetworkStream)
        && has_endpoint(Service::Terminal.spec().endpoints, EndpointKind::NetworkClient)
        && has_endpoint(Service::Terminal.spec().endpoints, EndpointKind::NetworkStream)
        && has_endpoint(Service::Network.spec().endpoints, EndpointKind::NetworkServer)
        && !has_endpoint(Service::Terminal.spec().endpoints, EndpointKind::NetworkDevice)
        && !has_endpoint(Service::Terminal.spec().endpoints, EndpointKind::NetworkEvent)
        && !has_endpoint(Service::Terminal.spec().endpoints, EndpointKind::NetworkServer)
        && !has_endpoint(Service::Gateway.spec().endpoints, EndpointKind::NetworkDevice)
        && !has_endpoint(Service::Gateway.spec().endpoints, EndpointKind::NetworkEvent)
        && !has_endpoint(Service::Gateway.spec().endpoints, EndpointKind::NetworkServer)
        && has_endpoint(Service::Storage.spec().endpoints, EndpointKind::BlockClient)
        && has_endpoint(Service::Gateway.spec().endpoints, EndpointKind::Remote)
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
