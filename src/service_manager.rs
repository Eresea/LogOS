//! Fixed Core-owned service lifecycle state and control-plane validation.

use logos_abi::{
    MAX_MANAGER_SERVICES, MAX_SERVICE_NAME_BYTES, ManagerOperation, ManagerRequest, ManagerRights,
    ManagerState, ManagerStatus, ServiceManagerRecord,
};

use crate::service_images::SERVICE_IMAGES;

pub const MAX_SERVICE_SLOTS: usize = MAX_MANAGER_SERVICES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceImageSource {
    Predeclared,
    FilesystemPackage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceHandle {
    slot: u8,
    generation: u32,
}

impl ServiceHandle {
    pub const fn new(slot: usize, generation: u32) -> Option<Self> {
        if slot >= MAX_SERVICE_SLOTS || generation == 0 {
            return None;
        }
        Some(Self { slot: slot as u8, generation })
    }

    pub const fn slot(self) -> usize {
        self.slot as usize
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagerAction {
    None,
    Start(logos_abi::ServiceId),
    Stop(logos_abi::ServiceId),
    Restart([logos_abi::ServiceId; MAX_SERVICE_SLOTS], usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagerDecision {
    pub response: logos_abi::ManagerResponse,
    pub action: ManagerAction,
}

#[derive(Clone, Copy)]
struct Slot {
    service: Option<logos_abi::ServiceId>,
    name: [u8; MAX_SERVICE_NAME_BYTES],
    name_len: u8,
    dependencies: u8,
    generation: u32,
    state: ManagerState,
    restarts: u8,
}

impl Slot {
    const EMPTY: Self = Self {
        service: None,
        name: [0; MAX_SERVICE_NAME_BYTES],
        name_len: 0,
        dependencies: 0,
        generation: 1,
        state: ManagerState::Vacant,
        restarts: 0,
    };

    const fn record(self, slot: usize) -> ServiceManagerRecord {
        ServiceManagerRecord {
            slot: slot as u8,
            state: self.state,
            restarts: self.restarts,
            name_len: self.name_len,
            generation: self.generation,
            dependencies: self.dependencies,
            reserved: [0; 3],
            name: self.name,
        }
    }
}

pub struct ServiceManager {
    slots: [Slot; MAX_SERVICE_SLOTS],
    image_sources: [ServiceImageSource; MAX_SERVICE_SLOTS],
}

impl ServiceManager {
    pub const fn new() -> Self {
        let mut manager = Self {
            slots: [Slot::EMPTY; MAX_SERVICE_SLOTS],
            image_sources: [ServiceImageSource::Predeclared; MAX_SERVICE_SLOTS],
        };
        manager.install_profiles();
        manager
    }

    const fn install_profiles(&mut self) {
        let mut index = 0;
        while index < SERVICE_IMAGES.len() {
            let spec = SERVICE_IMAGES[index];
            let service = spec.service();
            let name = spec.name();
            let mut bytes = [0; MAX_SERVICE_NAME_BYTES];
            let mut name_index = 0;
            while name_index < name.len() {
                bytes[name_index] = name[name_index];
                name_index += 1;
            }
            self.slots[index] = Slot {
                service: Some(service),
                name: bytes,
                name_len: name.len() as u8,
                dependencies: spec.dependencies(),
                generation: 1,
                state: ManagerState::Stopped,
                restarts: 0,
            };
            index += 1;
        }
    }

    pub fn initialize_running(&mut self) {
        for slot in &mut self.slots[..SERVICE_IMAGES.len()] {
            if slot.state != ManagerState::Disabled {
                slot.state = ManagerState::Running;
            }
        }
    }

    pub fn set_network_enabled(&mut self, enabled: bool) {
        let slot = &mut self.slots[logos_abi::ServiceId::Network.index()];
        if slot.service.is_some() {
            slot.state = if enabled { ManagerState::Stopped } else { ManagerState::Disabled };
        }
        let fetch = &mut self.slots[logos_abi::ServiceId::Fetch.index()];
        if fetch.service.is_some() {
            fetch.state = if enabled { ManagerState::Stopped } else { ManagerState::Disabled };
        }
    }

    pub const fn record(&self, slot: usize) -> Option<ServiceManagerRecord> {
        if slot >= self.slots.len() || self.slots[slot].service.is_none() {
            return None;
        }
        Some(self.slots[slot].record(slot))
    }

    pub const fn service(&self, slot: usize) -> Option<logos_abi::ServiceId> {
        if slot >= self.slots.len() { None } else { self.slots[slot].service }
    }

    pub const fn image_source(&self, service: logos_abi::ServiceId) -> Option<ServiceImageSource> {
        if service.index() < SERVICE_IMAGES.len() && self.slots[service.index()].service.is_some() {
            Some(self.image_sources[service.index()])
        } else {
            None
        }
    }

    pub fn set_image_source(&mut self, service: logos_abi::ServiceId, source: ServiceImageSource) {
        if service.index() < self.image_sources.len()
            && self.slots[service.index()].service.is_some()
        {
            self.image_sources[service.index()] = source;
        }
    }

    pub const fn handle(&self, slot: usize) -> Option<ServiceHandle> {
        if slot >= self.slots.len() || self.slots[slot].service.is_none() {
            return None;
        }
        ServiceHandle::new(slot, self.slots[slot].generation)
    }

    pub const fn state(&self, slot: usize) -> Option<ManagerState> {
        if slot >= self.slots.len() || self.slots[slot].service.is_none() {
            None
        } else {
            Some(self.slots[slot].state)
        }
    }

    pub fn mark_running(&mut self, service: logos_abi::ServiceId) {
        let index = service.index();
        if index < self.slots.len() {
            self.slots[index].state = ManagerState::Running;
        }
    }

    pub fn mark_starting(&mut self, service: logos_abi::ServiceId) {
        let index = service.index();
        if index < self.slots.len() {
            self.slots[index].state = ManagerState::Starting;
        }
    }

    pub fn mark_stopping(&mut self, service: logos_abi::ServiceId) {
        let index = service.index();
        if index < self.slots.len() {
            self.slots[index].state = ManagerState::Stopping;
        }
    }

    pub fn mark_stopped(&mut self, service: logos_abi::ServiceId) {
        let index = service.index();
        if index < self.slots.len() {
            self.slots[index].state = ManagerState::Stopped;
            self.slots[index].generation = self.slots[index].generation.wrapping_add(1).max(1);
        }
    }

    pub fn mark_failed(&mut self, service: logos_abi::ServiceId) {
        let index = service.index();
        if index < self.slots.len() {
            self.slots[index].state = ManagerState::Failed;
        }
    }

    pub fn restart_complete(&mut self, services: &[logos_abi::ServiceId]) {
        for service in services {
            let index = service.index();
            if index < self.slots.len() {
                self.slots[index].state = ManagerState::Running;
                self.slots[index].restarts = self.slots[index].restarts.saturating_add(1);
            }
        }
    }

    pub fn mark_restart_stopping(&mut self, services: &[logos_abi::ServiceId]) {
        for service in services {
            let index = service.index();
            if index < self.slots.len() {
                self.slots[index].state = ManagerState::Stopping;
            }
        }
    }

    pub fn prepare_graph_restart(&mut self) {
        for slot in &mut self.slots[..SERVICE_IMAGES.len()] {
            if slot.state != ManagerState::Disabled {
                slot.state = ManagerState::Stopped;
                slot.generation = slot.generation.wrapping_add(1).max(1);
            }
        }
    }

    pub fn request(&mut self, request: ManagerRequest, rights: ManagerRights) -> ManagerDecision {
        let mut response = logos_abi::ManagerResponse::new(
            request.operation,
            ManagerStatus::Malformed,
            request.request_id,
        );
        if request.abi_version != logos_abi::MANAGER_ABI_VERSION {
            return ManagerDecision { response, action: ManagerAction::None };
        }
        if request.reserved != 0 || request.reserved_tail != [0; 2] {
            return ManagerDecision { response, action: ManagerAction::None };
        }
        if request.request_id == 0 {
            return ManagerDecision { response, action: ManagerAction::None };
        }
        let shape_valid = match request.operation {
            ManagerOperation::List => {
                request.slot == u8::MAX
                    && request.generation == 0
                    && usize::from(request.cursor) <= MAX_MANAGER_SERVICES
            }
            ManagerOperation::Status
            | ManagerOperation::Start
            | ManagerOperation::Stop
            | ManagerOperation::Restart => request.slot != u8::MAX && request.cursor == 0,
        };
        if !shape_valid {
            return ManagerDecision { response, action: ManagerAction::None };
        }
        let required = if request.operation.requires_lifecycle() {
            ManagerRights::LIFECYCLE
        } else {
            ManagerRights::INSPECT
        };
        if !rights.contains(required) {
            response.status = ManagerStatus::Unauthorized;
            return ManagerDecision { response, action: ManagerAction::None };
        }
        match request.operation {
            ManagerOperation::List => {
                let Some((slot, record)) = self.next_record(request.cursor as usize) else {
                    response.status = ManagerStatus::Ok;
                    response.cursor = u8::MAX;
                    return ManagerDecision { response, action: ManagerAction::None };
                };
                response.status = ManagerStatus::Ok;
                response.cursor = slot.saturating_add(1) as u8;
                response.record = record;
            }
            ManagerOperation::Status => {
                let index = match self.valid_index(&request) {
                    Ok(index) => index,
                    Err(status) => {
                        response.status = status;
                        return ManagerDecision { response, action: ManagerAction::None };
                    }
                };
                response.status = ManagerStatus::Ok;
                response.record = self.slots[index].record(index);
            }
            ManagerOperation::Start => {
                let index = match self.valid_index(&request) {
                    Ok(index) => index,
                    Err(status) => {
                        response.status = status;
                        return ManagerDecision { response, action: ManagerAction::None };
                    }
                };
                if self.slots[index].state != ManagerState::Stopped {
                    response.status = ManagerStatus::InvalidState;
                } else if !self.dependencies_running(self.slots[index].dependencies) {
                    response.status = ManagerStatus::Dependency;
                } else {
                    self.slots[index].state = ManagerState::Starting;
                    response.status = ManagerStatus::Accepted;
                    response.record = self.slots[index].record(index);
                    return ManagerDecision {
                        response,
                        action: ManagerAction::Start(self.slots[index].service.unwrap()),
                    };
                }
            }
            ManagerOperation::Stop => {
                let index = match self.valid_index(&request) {
                    Ok(index) => index,
                    Err(status) => {
                        response.status = status;
                        return ManagerDecision { response, action: ManagerAction::None };
                    }
                };
                if self.has_active_dependents(index) {
                    response.status = ManagerStatus::Dependency;
                } else if self.slots[index].state != ManagerState::Running {
                    response.status = ManagerStatus::InvalidState;
                } else {
                    self.slots[index].state = ManagerState::Stopping;
                    response.status = ManagerStatus::Accepted;
                    response.record = self.slots[index].record(index);
                    return ManagerDecision {
                        response,
                        action: ManagerAction::Stop(self.slots[index].service.unwrap()),
                    };
                }
            }
            ManagerOperation::Restart => {
                let index = match self.valid_index(&request) {
                    Ok(index) => index,
                    Err(status) => {
                        response.status = status;
                        return ManagerDecision { response, action: ManagerAction::None };
                    }
                };
                if self.slots[index].state != ManagerState::Running {
                    response.status = ManagerStatus::InvalidState;
                } else if self.has_transitional_dependents(index) {
                    response.status = ManagerStatus::Busy;
                } else {
                    let mut services = [logos_abi::ServiceId::Input; MAX_SERVICE_SLOTS];
                    let Some(count) = self.restart_closure(index, &mut services) else {
                        response.status = ManagerStatus::Dependency;
                        return ManagerDecision { response, action: ManagerAction::None };
                    };
                    response.status = ManagerStatus::Accepted;
                    response.record = self.slots[index].record(index);
                    return ManagerDecision {
                        response,
                        action: ManagerAction::Restart(services, count),
                    };
                }
            }
        }
        if response.status == ManagerStatus::Malformed {
            response.status = ManagerStatus::InvalidState;
        }
        ManagerDecision { response, action: ManagerAction::None }
    }

    fn valid_index(&self, request: &ManagerRequest) -> Result<usize, ManagerStatus> {
        let index = request.slot as usize;
        if index >= self.slots.len() || self.slots[index].service.is_none() {
            Err(ManagerStatus::Unsupported)
        } else if self.slots[index].generation != request.generation {
            Err(ManagerStatus::Stale)
        } else {
            Ok(index)
        }
    }

    fn next_record(&self, cursor: usize) -> Option<(usize, ServiceManagerRecord)> {
        (cursor..self.slots.len()).find_map(|index| {
            self.slots[index].service.map(|_| (index, self.slots[index].record(index)))
        })
    }

    fn dependencies_running(&self, dependencies: u8) -> bool {
        (0..SERVICE_IMAGES.len()).all(|index| {
            dependencies & (1 << index) == 0 || self.slots[index].state == ManagerState::Running
        })
    }

    fn has_active_dependents(&self, index: usize) -> bool {
        self.slots.iter().any(|slot| {
            slot.service.is_some()
                && slot.dependencies & (1 << index) != 0
                && matches!(
                    slot.state,
                    ManagerState::Starting | ManagerState::Running | ManagerState::Stopping
                )
        })
    }

    fn has_transitional_dependents(&self, index: usize) -> bool {
        let mut included = 1u8 << index;
        let mut changed = true;
        while changed {
            changed = false;
            for slot in &self.slots[..SERVICE_IMAGES.len()] {
                let Some(service) = slot.service else {
                    continue;
                };
                let bit = 1 << service.index();
                if included & bit == 0 && slot.dependencies & included != 0 {
                    if matches!(slot.state, ManagerState::Starting | ManagerState::Stopping) {
                        return true;
                    }
                    included |= bit;
                    changed = true;
                }
            }
        }
        false
    }

    fn restart_closure(
        &self,
        index: usize,
        output: &mut [logos_abi::ServiceId; MAX_SERVICE_SLOTS],
    ) -> Option<usize> {
        let mut included = 0u8;
        let mut count = 0;
        let mut changed = true;
        included |= 1 << index;
        while changed {
            changed = false;
            for slot in &self.slots[..SERVICE_IMAGES.len()] {
                if let Some(service) = slot.service {
                    if slot.state == ManagerState::Running
                        && slot.dependencies & included != 0
                        && included & (1 << service.index()) == 0
                    {
                        included |= 1 << service.index();
                        changed = true;
                    }
                }
            }
        }
        let mut order = [logos_abi::ServiceId::Input; MAX_SERVICE_SLOTS];
        let mut order_count = 0;
        let mut remaining = included;
        while remaining != 0 {
            let mut advanced = false;
            for spec in SERVICE_IMAGES {
                let service = spec.service();
                let bit = 1 << service.index();
                if remaining & bit != 0 && spec.dependencies() & remaining == 0 {
                    order[order_count] = service;
                    order_count += 1;
                    remaining &= !bit;
                    advanced = true;
                    break;
                }
            }
            if !advanced {
                return None;
            }
        }
        while order_count != 0 {
            order_count -= 1;
            output[count] = order[order_count];
            count += 1;
        }
        Some(count)
    }
}

impl Default for ServiceManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logos_abi::ServiceId;

    fn manager() -> ServiceManager {
        let mut manager = ServiceManager::new();
        manager.initialize_running();
        manager
    }

    fn request(operation: ManagerOperation, slot: usize, generation: u32) -> ManagerRequest {
        let mut request = ManagerRequest::new(operation, 1);
        request.slot = slot as u8;
        request.generation = generation;
        request
    }

    #[test]
    fn list_is_bounded_and_cursored() {
        let mut manager = manager();
        let mut request = ManagerRequest::new(ManagerOperation::List, 1);
        let first = manager.request(request, ManagerRights::INSPECT).response;
        assert_eq!(first.status, ManagerStatus::Ok);
        assert_eq!(&first.record.name[..first.record.name_len as usize], b"input");
        request.cursor = first.cursor;
        let second = manager.request(request, ManagerRights::INSPECT).response;
        assert_eq!(&second.record.name[..second.record.name_len as usize], b"display");
    }

    #[test]
    fn disabled_network_remains_visible_but_cannot_start() {
        let mut manager = ServiceManager::new();
        manager.set_network_enabled(false);
        manager.initialize_running();
        let record = manager.record(ServiceId::Network.index()).unwrap();
        assert_eq!(record.state, ManagerState::Disabled);
        let handle = manager.handle(ServiceId::Network.index()).unwrap();
        let response = manager
            .request(
                request(ManagerOperation::Start, handle.slot(), handle.generation()),
                ManagerRights::ALL,
            )
            .response;
        assert_eq!(response.status, ManagerStatus::InvalidState);
    }

    #[test]
    fn stop_rejects_running_dependents() {
        let mut manager = manager();
        let handle = manager.handle(ServiceId::Session.index()).unwrap();
        let response = manager
            .request(
                request(ManagerOperation::Stop, handle.slot(), handle.generation()),
                ManagerRights::ALL,
            )
            .response;
        assert_eq!(response.status, ManagerStatus::Dependency);
    }

    #[test]
    fn stop_rejects_running_storage_dependents() {
        let mut manager = manager();
        let handle = manager.handle(ServiceId::Storage.index()).unwrap();
        let response = manager
            .request(
                request(ManagerOperation::Stop, handle.slot(), handle.generation()),
                ManagerRights::ALL,
            )
            .response;
        assert_eq!(response.status, ManagerStatus::Dependency);
    }

    #[test]
    fn stop_rejects_transitional_dependents() {
        let mut manager = manager();
        manager.mark_starting(ServiceId::Flow);
        let handle = manager.handle(ServiceId::Session.index()).unwrap();
        let response = manager
            .request(
                request(ManagerOperation::Stop, handle.slot(), handle.generation()),
                ManagerRights::ALL,
            )
            .response;
        assert_eq!(response.status, ManagerStatus::Dependency);
    }

    #[test]
    fn stale_handle_is_rejected() {
        let mut manager = manager();
        let handle = manager.handle(ServiceId::Storage.index()).unwrap();
        let mut stale = request(ManagerOperation::Status, handle.slot(), handle.generation() + 1);
        stale.request_id = 9;
        assert_eq!(
            manager.request(stale, ManagerRights::INSPECT).response.status,
            ManagerStatus::Stale
        );
        manager.mark_stopped(ServiceId::Storage);
        let mut stopped = request(ManagerOperation::Status, handle.slot(), handle.generation());
        stopped.request_id = 10;
        assert_eq!(
            manager.request(stopped, ManagerRights::INSPECT).response.status,
            ManagerStatus::Stale
        );
    }

    #[test]
    fn malformed_requests_and_unsupported_images_are_rejected() {
        let mut manager = manager();
        let mut malformed = ManagerRequest::new(ManagerOperation::List, 1);
        malformed.reserved = 1;
        assert_eq!(
            manager.request(malformed, ManagerRights::INSPECT).response.status,
            ManagerStatus::Malformed
        );
        malformed = ManagerRequest::new(ManagerOperation::List, 2);
        malformed.cursor = u8::MAX;
        assert_eq!(
            manager.request(malformed, ManagerRights::INSPECT).response.status,
            ManagerStatus::Malformed
        );
        let zero_id = ManagerRequest::new(ManagerOperation::List, 0);
        assert_eq!(
            manager.request(zero_id, ManagerRights::INSPECT).response.status,
            ManagerStatus::Malformed
        );
        assert_eq!(
            manager
                .request(request(ManagerOperation::Start, 0, 1), ManagerRights::INSPECT,)
                .response
                .status,
            ManagerStatus::Unauthorized
        );
        assert_eq!(
            manager
                .request(request(ManagerOperation::Start, 8, 1), ManagerRights::ALL)
                .response
                .status,
            ManagerStatus::Unsupported
        );
        assert_eq!(manager.image_source(ServiceId::Input), Some(ServiceImageSource::Predeclared));
    }

    #[test]
    fn graph_restart_invalidates_existing_handles() {
        let mut manager = manager();
        let handle = manager.handle(ServiceId::Input.index()).unwrap();
        manager.prepare_graph_restart();
        let response = manager
            .request(
                request(ManagerOperation::Status, handle.slot(), handle.generation()),
                ManagerRights::INSPECT,
            )
            .response;
        assert_eq!(response.status, ManagerStatus::Stale);
    }

    #[test]
    fn restart_includes_dependents_in_reverse_dependency_order() {
        let mut manager = manager();
        let handle = manager.handle(ServiceId::Session.index()).unwrap();
        let decision = manager.request(
            request(ManagerOperation::Restart, handle.slot(), handle.generation()),
            ManagerRights::ALL,
        );
        assert_eq!(decision.response.status, ManagerStatus::Accepted);
        assert_eq!(
            decision.action,
            ManagerAction::Restart(
                [
                    ServiceId::Fetch,
                    ServiceId::Flow,
                    ServiceId::Session,
                    ServiceId::Input,
                    ServiceId::Input,
                    ServiceId::Input,
                    ServiceId::Input,
                    ServiceId::Input,
                ],
                3,
            )
        );
    }

    #[test]
    fn restart_rejects_transitional_transitive_dependents() {
        let mut manager = manager();
        manager.mark_starting(ServiceId::Session);
        let handle = manager.handle(ServiceId::Input.index()).unwrap();
        let response = manager
            .request(
                request(ManagerOperation::Restart, handle.slot(), handle.generation()),
                ManagerRights::ALL,
            )
            .response;
        assert_eq!(response.status, ManagerStatus::Busy);
    }
}
