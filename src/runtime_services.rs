//! Dynamic service records used by the v5 manager migration.

use alloc::vec::Vec;

use logos_abi::{
    DIRECTORY_FLAG_MORE, DIRECTORY_RECORDS_PER_PAGE, DirectoryOperation, DirectoryRecord,
    DirectoryResponse, DirectoryStatus, MAX_PACKAGE_NAME_BYTES, MAX_SERVICE_NAME_BYTES,
    ManagerOperation, ManagerRequest, ManagerResponse, ManagerRights, ManagerState, ManagerStatus,
    SERVICE_HEAP_MAX_PAGES, ServiceHandle, ServiceManagerRecord,
};

struct Slot {
    generation: u32,
    value: Option<ServiceRecord>,
}

impl Slot {
    fn with_generation(generation: u32) -> Self {
        Self { generation: generation.max(1), value: None }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceState {
    Disabled,
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceImageSource {
    Builtin,
    FilesystemPackage,
}

struct ServiceRecord {
    handle: ServiceHandle,
    name: Vec<u8>,
    image: Vec<u8>,
    dependencies: Vec<ServiceHandle>,
    state: ServiceState,
    epoch: u64,
    restarts: u8,
    heap_quota_pages: usize,
    manager_rights: ManagerRights,
    image_source: ServiceImageSource,
    ownership: ServiceOwnership,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceOwnership {
    pub process: u64,
    pub address_space: u64,
    pub task: u64,
    pub heap_pages: usize,
    pub heartbeat: u64,
    pub ipc_endpoints: usize,
    pub capabilities: usize,
    pub events: usize,
}

impl ServiceOwnership {
    const EMPTY: Self = Self {
        process: 0,
        address_space: 0,
        task: 0,
        heap_pages: 0,
        heartbeat: 0,
        ipc_endpoints: 0,
        capabilities: 0,
        events: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceRegistryError {
    Stale,
    InvalidName,
    InvalidImage,
    InvalidDependency,
    DependencyCycle,
    Capacity,
    RunningDependents,
}

#[derive(Debug)]
pub enum RuntimeLifecycleAction {
    Start(ServiceHandle),
    Stop(ServiceHandle),
    Restart(Vec<ServiceHandle>),
}

pub fn service_request_shape_valid(request: ManagerRequest) -> bool {
    if request.abi_version != logos_abi::MANAGER_ABI_VERSION
        || request.request_id == 0
        || request.target_kind != logos_abi::ManagerTargetKind::Service
        || request.reserved_tail != [0; 2]
        || request.name_len as usize > request.name.len()
        || request.name[request.name_len as usize..].iter().any(|byte| *byte != 0)
    {
        return false;
    }
    match request.operation {
        ManagerOperation::List => {
            request.service == ServiceHandle::EMPTY
                && request.program_generation == 0
                && request.program_slot == u8::MAX
        }
        ManagerOperation::Status
        | ManagerOperation::Start
        | ManagerOperation::Stop
        | ManagerOperation::Restart => {
            request.service.is_valid()
                && request.program_slot == u8::MAX
                && request.program_generation == 0
                && request.cursor == 0
        }
        _ => false,
    }
}

pub struct RuntimeServiceRegistry {
    slots: Vec<Slot>,
    generation_seed: u32,
}

impl RuntimeServiceRegistry {
    pub fn new() -> Self {
        Self::new_with_generation(1)
    }

    pub fn new_with_generation(generation: u32) -> Self {
        Self { slots: Vec::new(), generation_seed: generation.max(1) }
    }

    pub fn register(
        &mut self,
        name: &[u8],
        image: &[u8],
        dependencies: &[ServiceHandle],
    ) -> Result<ServiceHandle, ServiceRegistryError> {
        self.register_with_quota(name, image, dependencies, SERVICE_HEAP_MAX_PAGES)
    }

    pub fn register_with_quota(
        &mut self,
        name: &[u8],
        image: &[u8],
        dependencies: &[ServiceHandle],
        heap_quota_pages: usize,
    ) -> Result<ServiceHandle, ServiceRegistryError> {
        if name.is_empty() || name.len() > MAX_SERVICE_NAME_BYTES {
            return Err(ServiceRegistryError::InvalidName);
        }
        if image.is_empty() || heap_quota_pages == 0 {
            return Err(ServiceRegistryError::InvalidImage);
        }
        for dependency in dependencies {
            if self.service(*dependency).is_err() {
                return Err(ServiceRegistryError::InvalidDependency);
            }
        }
        let index = self.allocate_slot()?;
        let handle = ServiceHandle::new(index as u32, self.slots[index].generation)
            .ok_or(ServiceRegistryError::Capacity)?;
        let mut name_copy = Vec::new();
        name_copy.try_reserve(name.len()).map_err(|_| ServiceRegistryError::Capacity)?;
        name_copy.extend_from_slice(name);
        let mut image_copy = Vec::new();
        image_copy.try_reserve(image.len()).map_err(|_| ServiceRegistryError::Capacity)?;
        image_copy.extend_from_slice(image);
        let mut deps = Vec::new();
        deps.try_reserve(dependencies.len()).map_err(|_| ServiceRegistryError::Capacity)?;
        deps.extend_from_slice(dependencies);
        self.slots[index].value = Some(ServiceRecord {
            handle,
            name: name_copy,
            image: image_copy,
            dependencies: deps,
            state: ServiceState::Stopped,
            epoch: 1,
            restarts: 0,
            heap_quota_pages,
            manager_rights: ManagerRights::NONE,
            image_source: ServiceImageSource::Builtin,
            ownership: ServiceOwnership::EMPTY,
        });
        Ok(handle)
    }

    pub fn start(&mut self, handle: ServiceHandle) -> Result<(), ServiceRegistryError> {
        let mut visiting = Vec::new();
        let mut order = Vec::new();
        self.collect_start_order(handle, &mut visiting, &mut order)?;
        self.apply_start_order(&order)
    }

    pub fn set_dependencies(
        &mut self,
        handle: ServiceHandle,
        dependencies: &[ServiceHandle],
    ) -> Result<(), ServiceRegistryError> {
        for dependency in dependencies {
            if self.service(*dependency).is_err() {
                return Err(ServiceRegistryError::InvalidDependency);
            }
        }
        let service = self.service_mut(handle)?;
        service
            .dependencies
            .try_reserve(dependencies.len().saturating_sub(service.dependencies.len()))
            .map_err(|_| ServiceRegistryError::Capacity)?;
        service.dependencies.clear();
        service.dependencies.extend_from_slice(dependencies);
        Ok(())
    }

    pub fn disable(&mut self, handle: ServiceHandle) -> Result<(), ServiceRegistryError> {
        let service = self.service_mut(handle)?;
        service.state = ServiceState::Disabled;
        Ok(())
    }

    pub fn stop(&mut self, handle: ServiceHandle) -> Result<(), ServiceRegistryError> {
        let index = self.index(handle)?;
        if self.slots.iter().filter_map(|slot| slot.value.as_ref()).any(|service| {
            service.state == ServiceState::Running && service.dependencies.contains(&handle)
        }) {
            return Err(ServiceRegistryError::RunningDependents);
        }
        let service = self.slots[index].value.as_mut().ok_or(ServiceRegistryError::Stale)?;
        service.state = ServiceState::Stopped;
        service.epoch = next_epoch(service.epoch);
        Ok(())
    }

    pub fn fail(&mut self, handle: ServiceHandle) -> Result<(), ServiceRegistryError> {
        self.service_mut(handle)?.state = ServiceState::Failed;
        Ok(())
    }

    pub fn mark_stopping(&mut self, handle: ServiceHandle) -> Result<(), ServiceRegistryError> {
        let service = self.service_mut(handle)?;
        if service.state == ServiceState::Running {
            service.state = ServiceState::Stopping;
        }
        Ok(())
    }

    pub fn restart(&mut self, handle: ServiceHandle) -> Result<(), ServiceRegistryError> {
        let index = self.index(handle)?;
        let dependencies = self.service(handle)?.dependencies.clone();
        let mut visiting = Vec::new();
        visiting.push(handle);
        let mut order = Vec::new();
        for dependency in dependencies {
            self.collect_start_order(dependency, &mut visiting, &mut order)?;
        }
        visiting.pop();
        self.apply_start_order(&order)?;
        let service = self.slots[index].value.as_mut().ok_or(ServiceRegistryError::Stale)?;
        service.epoch = next_epoch(service.epoch);
        service.state = ServiceState::Running;
        service.restarts = service.restarts.saturating_add(1);
        Ok(())
    }

    pub fn record_restart(&mut self, handle: ServiceHandle) -> Result<(), ServiceRegistryError> {
        let service = self.service_mut(handle)?;
        service.epoch = next_epoch(service.epoch);
        service.state = ServiceState::Running;
        service.restarts = service.restarts.saturating_add(1);
        Ok(())
    }

    pub fn remove(&mut self, handle: ServiceHandle) -> Result<(), ServiceRegistryError> {
        let index = self.index(handle)?;
        if self
            .slots
            .iter()
            .filter_map(|slot| slot.value.as_ref())
            .any(|service| service.dependencies.contains(&handle))
        {
            return Err(ServiceRegistryError::RunningDependents);
        }
        self.slots[index].value = None;
        self.slots[index].generation = next_generation(self.slots[index].generation);
        Ok(())
    }

    pub fn state(&self, handle: ServiceHandle) -> Result<ServiceState, ServiceRegistryError> {
        Ok(self.service(handle)?.state)
    }

    pub fn epoch(&self, handle: ServiceHandle) -> Result<u64, ServiceRegistryError> {
        Ok(self.service(handle)?.epoch)
    }

    pub fn image_len(&self, handle: ServiceHandle) -> Result<usize, ServiceRegistryError> {
        Ok(self.service(handle)?.image.len())
    }

    pub fn set_image(
        &mut self,
        handle: ServiceHandle,
        image: &[u8],
    ) -> Result<(), ServiceRegistryError> {
        if image.is_empty() {
            return Err(ServiceRegistryError::InvalidImage);
        }
        let service = self.service_mut(handle)?;
        let mut replacement = Vec::new();
        replacement.try_reserve(image.len()).map_err(|_| ServiceRegistryError::Capacity)?;
        replacement.extend_from_slice(image);
        service.image = replacement;
        Ok(())
    }

    pub fn heap_quota_pages(&self, handle: ServiceHandle) -> Result<usize, ServiceRegistryError> {
        Ok(self.service(handle)?.heap_quota_pages)
    }

    pub fn set_manager_rights(
        &mut self,
        handle: ServiceHandle,
        rights: ManagerRights,
    ) -> Result<(), ServiceRegistryError> {
        self.service_mut(handle)?.manager_rights = rights;
        Ok(())
    }

    pub fn manager_rights(
        &self,
        handle: ServiceHandle,
    ) -> Result<ManagerRights, ServiceRegistryError> {
        Ok(self.service(handle)?.manager_rights)
    }

    pub fn set_image_source(
        &mut self,
        handle: ServiceHandle,
        source: ServiceImageSource,
    ) -> Result<(), ServiceRegistryError> {
        self.service_mut(handle)?.image_source = source;
        Ok(())
    }

    pub fn image_source(
        &self,
        handle: ServiceHandle,
    ) -> Result<ServiceImageSource, ServiceRegistryError> {
        Ok(self.service(handle)?.image_source)
    }

    pub fn set_runtime_ownership(
        &mut self,
        handle: ServiceHandle,
        process: u64,
        address_space: u64,
        task: u64,
        heap_pages: usize,
    ) -> Result<(), ServiceRegistryError> {
        if process == 0 || address_space == 0 || task == 0 || heap_pages == 0 {
            return Err(ServiceRegistryError::Capacity);
        }
        let service = self.service_mut(handle)?;
        service.ownership.process = process;
        service.ownership.address_space = address_space;
        service.ownership.task = task;
        service.ownership.heap_pages = heap_pages;
        Ok(())
    }

    pub fn set_heartbeat(
        &mut self,
        handle: ServiceHandle,
        heartbeat: u64,
    ) -> Result<(), ServiceRegistryError> {
        self.service_mut(handle)?.ownership.heartbeat = heartbeat;
        Ok(())
    }

    pub fn set_runtime_counts(
        &mut self,
        handle: ServiceHandle,
        ipc_endpoints: usize,
        capabilities: usize,
        events: usize,
    ) -> Result<(), ServiceRegistryError> {
        let service = self.service_mut(handle)?;
        service.ownership.ipc_endpoints = ipc_endpoints;
        service.ownership.capabilities = capabilities;
        service.ownership.events = events;
        Ok(())
    }

    pub fn clear_runtime_ownership(
        &mut self,
        handle: ServiceHandle,
    ) -> Result<(), ServiceRegistryError> {
        self.service_mut(handle)?.ownership = ServiceOwnership::EMPTY;
        Ok(())
    }

    pub fn clear_execution_ownership(
        &mut self,
        handle: ServiceHandle,
    ) -> Result<(), ServiceRegistryError> {
        let ownership = &mut self.service_mut(handle)?.ownership;
        ownership.process = 0;
        ownership.address_space = 0;
        ownership.task = 0;
        ownership.heap_pages = 0;
        ownership.heartbeat = 0;
        Ok(())
    }

    pub fn ownership(
        &self,
        handle: ServiceHandle,
    ) -> Result<ServiceOwnership, ServiceRegistryError> {
        Ok(self.service(handle)?.ownership)
    }

    pub fn validate_lifecycle_handle(
        &self,
        handle: ServiceHandle,
    ) -> Result<(), ServiceRegistryError> {
        self.service(handle).map(|_| ())
    }

    pub fn lifecycle_status(
        &self,
        operation: ManagerOperation,
        handle: ServiceHandle,
    ) -> ManagerStatus {
        let Ok(service) = self.service(handle) else { return ManagerStatus::Stale };
        match operation {
            ManagerOperation::Start => {
                if service.state != ServiceState::Stopped {
                    return if matches!(
                        service.state,
                        ServiceState::Starting | ServiceState::Stopping
                    ) {
                        ManagerStatus::Busy
                    } else {
                        ManagerStatus::InvalidState
                    };
                }
                if service
                    .dependencies
                    .iter()
                    .any(|dependency| self.state(*dependency) != Ok(ServiceState::Running))
                {
                    ManagerStatus::Dependency
                } else {
                    ManagerStatus::Ok
                }
            }
            ManagerOperation::Stop => {
                if service.state != ServiceState::Running {
                    return if matches!(
                        service.state,
                        ServiceState::Starting | ServiceState::Stopping
                    ) {
                        ManagerStatus::Busy
                    } else {
                        ManagerStatus::InvalidState
                    };
                }
                if self.slots.iter().filter_map(|slot| slot.value.as_ref()).any(|dependent| {
                    dependent.dependencies.contains(&handle)
                        && dependent.state != ServiceState::Stopped
                }) {
                    ManagerStatus::Dependency
                } else {
                    ManagerStatus::Ok
                }
            }
            ManagerOperation::Restart => {
                if service.state == ServiceState::Running {
                    ManagerStatus::Ok
                } else if matches!(service.state, ServiceState::Starting | ServiceState::Stopping) {
                    ManagerStatus::Busy
                } else {
                    ManagerStatus::InvalidState
                }
            }
            _ => ManagerStatus::Unsupported,
        }
    }

    pub fn begin_lifecycle(
        &mut self,
        operation: ManagerOperation,
        handle: ServiceHandle,
    ) -> ManagerStatus {
        let status = self.lifecycle_status(operation, handle);
        if status != ManagerStatus::Ok {
            return status;
        }
        let Ok(service) = self.service_mut(handle) else { return ManagerStatus::Stale };
        service.state = match operation {
            ManagerOperation::Start => ServiceState::Starting,
            ManagerOperation::Stop | ManagerOperation::Restart => ServiceState::Stopping,
            _ => return ManagerStatus::Unsupported,
        };
        ManagerStatus::Accepted
    }

    pub fn begin_lifecycle_action(
        &mut self,
        operation: ManagerOperation,
        handle: ServiceHandle,
    ) -> Result<RuntimeLifecycleAction, ManagerStatus> {
        let status = self.lifecycle_status(operation, handle);
        if status != ManagerStatus::Ok {
            return Err(status);
        }
        match operation {
            ManagerOperation::Start => {
                self.service_mut(handle).map_err(|_| ManagerStatus::Stale)?.state =
                    ServiceState::Starting;
                Ok(RuntimeLifecycleAction::Start(handle))
            }
            ManagerOperation::Stop => {
                self.service_mut(handle).map_err(|_| ManagerStatus::Stale)?.state =
                    ServiceState::Stopping;
                Ok(RuntimeLifecycleAction::Stop(handle))
            }
            ManagerOperation::Restart => {
                let closure =
                    self.restart_closure(handle).map_err(|_| ManagerStatus::Dependency)?;
                for member in &closure {
                    self.service_mut(*member).map_err(|_| ManagerStatus::Stale)?.state =
                        ServiceState::Stopping;
                }
                Ok(RuntimeLifecycleAction::Restart(closure))
            }
            _ => Err(ManagerStatus::Unsupported),
        }
    }

    pub fn abort_lifecycle(
        &mut self,
        operation: ManagerOperation,
        handle: ServiceHandle,
    ) -> ManagerStatus {
        let Ok(service) = self.service_mut(handle) else { return ManagerStatus::Stale };
        let expected_state = match operation {
            ManagerOperation::Start => ServiceState::Starting,
            ManagerOperation::Stop | ManagerOperation::Restart => ServiceState::Stopping,
            _ => return ManagerStatus::Unsupported,
        };
        if service.state != expected_state {
            return ManagerStatus::InvalidState;
        }
        service.state = match operation {
            ManagerOperation::Start => ServiceState::Stopped,
            ManagerOperation::Stop | ManagerOperation::Restart => ServiceState::Running,
            _ => return ManagerStatus::Unsupported,
        };
        ManagerStatus::Ok
    }

    pub fn abort_lifecycle_members(&mut self, members: &[ServiceHandle]) -> ManagerStatus {
        for handle in members {
            let Ok(service) = self.service_mut(*handle) else {
                return ManagerStatus::Stale;
            };
            if service.state != ServiceState::Stopping {
                return ManagerStatus::InvalidState;
            }
        }
        for handle in members {
            if let Ok(service) = self.service_mut(*handle) {
                service.state = ServiceState::Running;
            }
        }
        ManagerStatus::Ok
    }

    pub fn manager_request(&self, request: ManagerRequest) -> ManagerResponse {
        let mut response =
            ManagerResponse::new(request.operation, ManagerStatus::Malformed, request.request_id);
        if !service_request_shape_valid(request) {
            return response;
        }
        match request.operation {
            ManagerOperation::List => {
                response.status = ManagerStatus::Ok;
                let cursor = usize::try_from(request.cursor).unwrap_or(usize::MAX);
                let Some((next, record)) = self.next_manager_record(cursor) else {
                    response.cursor = u64::MAX;
                    return response;
                };
                response.cursor = next as u64;
                response.record = record;
            }
            ManagerOperation::Status => {
                let Ok(record) = self.manager_record(request.service) else {
                    response.status = ManagerStatus::Stale;
                    return response;
                };
                response.status = ManagerStatus::Ok;
                response.record = record;
            }
            _ => response.status = ManagerStatus::Unsupported,
        }
        response
    }

    pub fn list(
        &self,
        cursor: u64,
        response: &mut DirectoryResponse,
        request_id: u32,
    ) -> DirectoryStatus {
        if request_id == 0 {
            return DirectoryStatus::Malformed;
        }
        *response =
            DirectoryResponse::empty(DirectoryOperation::Services, DirectoryStatus::Ok, request_id);
        let start = usize::try_from(cursor).unwrap_or(usize::MAX);
        let mut written = 0usize;
        for (index, service) in self
            .slots
            .iter()
            .enumerate()
            .skip(start)
            .filter_map(|(index, slot)| slot.value.as_ref().map(|service| (index, service)))
        {
            if written == DIRECTORY_RECORDS_PER_PAGE {
                response.flags |= DIRECTORY_FLAG_MORE;
                response.cursor = index as u64;
                break;
            }
            let mut record = DirectoryRecord::service(service.handle, &service.name)
                .expect("registry names are validated at registration");
            record.flags = match service.state {
                ServiceState::Disabled => 2,
                ServiceState::Stopped => 0,
                ServiceState::Starting => 1,
                ServiceState::Running => 1,
                ServiceState::Stopping => 0,
                ServiceState::Failed => 0,
            };
            response.records[written] = record;
            written += 1;
        }
        response.count = written as u8;
        if response.flags & DIRECTORY_FLAG_MORE == 0 {
            response.cursor = u64::MAX;
        }
        DirectoryStatus::Ok
    }

    fn collect_start_order(
        &mut self,
        handle: ServiceHandle,
        visiting: &mut Vec<ServiceHandle>,
        order: &mut Vec<ServiceHandle>,
    ) -> Result<(), ServiceRegistryError> {
        if visiting.contains(&handle) {
            return Err(ServiceRegistryError::DependencyCycle);
        }
        let dependencies = self.service(handle)?.dependencies.clone();
        if self.state(handle)? == ServiceState::Running || order.contains(&handle) {
            return Ok(());
        }
        visiting.push(handle);
        for dependency in dependencies {
            self.collect_start_order(dependency, visiting, order)?;
        }
        visiting.pop();
        order.push(handle);
        Ok(())
    }

    fn apply_start_order(&mut self, order: &[ServiceHandle]) -> Result<(), ServiceRegistryError> {
        for handle in order {
            let service = self.service_mut(*handle)?;
            service.state = ServiceState::Running;
        }
        Ok(())
    }

    fn restart_closure(
        &self,
        handle: ServiceHandle,
    ) -> Result<Vec<ServiceHandle>, ServiceRegistryError> {
        self.service(handle)?;
        let mut included = Vec::new();
        included.try_reserve(1).map_err(|_| ServiceRegistryError::Capacity)?;
        included.push(handle);
        let mut changed = true;
        while changed {
            changed = false;
            for service in self.slots.iter().filter_map(|slot| slot.value.as_ref()) {
                if service.state == ServiceState::Running
                    && !included.contains(&service.handle)
                    && service.dependencies.iter().any(|dependency| included.contains(dependency))
                {
                    included.try_reserve(1).map_err(|_| ServiceRegistryError::Capacity)?;
                    included.push(service.handle);
                    changed = true;
                }
            }
        }
        let mut order = Vec::new();
        order.try_reserve(included.len()).map_err(|_| ServiceRegistryError::Capacity)?;
        let mut remaining = included;
        while !remaining.is_empty() {
            let position = remaining.iter().position(|candidate| {
                self.service(*candidate).is_ok_and(|service| {
                    !service.dependencies.iter().any(|dependency| remaining.contains(dependency))
                })
            });
            let Some(position) = position else {
                return Err(ServiceRegistryError::DependencyCycle);
            };
            order.push(remaining.swap_remove(position));
        }
        order.reverse();
        Ok(order)
    }

    fn allocate_slot(&mut self) -> Result<usize, ServiceRegistryError> {
        if let Some((index, _)) =
            self.slots.iter().enumerate().find(|(_, slot)| slot.value.is_none())
        {
            return Ok(index);
        }
        self.slots.try_reserve(1).map_err(|_| ServiceRegistryError::Capacity)?;
        self.slots.push(Slot::with_generation(self.generation_seed));
        Ok(self.slots.len() - 1)
    }

    fn index(&self, handle: ServiceHandle) -> Result<usize, ServiceRegistryError> {
        let index = handle.index() as usize;
        let Some(slot) = self.slots.get(index) else { return Err(ServiceRegistryError::Stale) };
        if slot.generation != handle.generation() || slot.value.is_none() {
            return Err(ServiceRegistryError::Stale);
        }
        Ok(index)
    }

    fn service(&self, handle: ServiceHandle) -> Result<&ServiceRecord, ServiceRegistryError> {
        let index = self.index(handle)?;
        self.slots[index].value.as_ref().ok_or(ServiceRegistryError::Stale)
    }

    fn service_mut(
        &mut self,
        handle: ServiceHandle,
    ) -> Result<&mut ServiceRecord, ServiceRegistryError> {
        let index = self.index(handle)?;
        self.slots[index].value.as_mut().ok_or(ServiceRegistryError::Stale)
    }

    fn manager_record(
        &self,
        handle: ServiceHandle,
    ) -> Result<ServiceManagerRecord, ServiceRegistryError> {
        let service = self.service(handle)?;
        let mut name = [0; MAX_PACKAGE_NAME_BYTES];
        let name_len = service.name.len().min(name.len());
        name[..name_len].copy_from_slice(&service.name[..name_len]);
        Ok(ServiceManagerRecord {
            service: service.handle,
            state: match service.state {
                ServiceState::Disabled => ManagerState::Disabled,
                ServiceState::Stopped => ManagerState::Stopped,
                ServiceState::Starting => ManagerState::Starting,
                ServiceState::Running => ManagerState::Running,
                ServiceState::Stopping => ManagerState::Stopping,
                ServiceState::Failed => ManagerState::Failed,
            },
            restarts: service.restarts,
            name_len: name_len as u8,
            dependencies: service.dependencies.len().min(u16::MAX as usize) as u16,
            reserved: [0; 2],
            program_slot: u8::MAX,
            reserved_program: [0; 3],
            program_generation: 0,
            name,
        })
    }

    fn next_manager_record(&self, cursor: usize) -> Option<(usize, ServiceManagerRecord)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| slot.value.as_ref().map(|_| index))
            .skip(cursor)
            .find_map(|index| {
                self.manager_record(self.slots[index].value.as_ref()?.handle)
                    .ok()
                    .map(|record| (index + 1, record))
            })
    }
}

impl Default for RuntimeServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn next_generation(current: u32) -> u32 {
    current.wrapping_add(1).max(1)
}

fn next_epoch(current: u64) -> u64 {
    current.wrapping_add(1).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_not_limited_to_the_builtin_service_count() {
        let mut registry = RuntimeServiceRegistry::new();
        let mut handles = Vec::new();
        for index in 0..(DIRECTORY_RECORDS_PER_PAGE + 4) {
            handles.push(registry.register(format_name(index).as_slice(), b"image", &[]).unwrap());
        }
        let mut response =
            DirectoryResponse::empty(DirectoryOperation::Services, DirectoryStatus::Malformed, 1);
        registry.list(0, &mut response, 1);
        assert_eq!(response.count as usize, DIRECTORY_RECORDS_PER_PAGE);
        assert_eq!(response.cursor, DIRECTORY_RECORDS_PER_PAGE as u64);
        registry.remove(handles[0]).unwrap();
        let cursor = response.cursor;
        registry.list(cursor, &mut response, 2);
        assert_eq!(response.records[0].handle, handles[DIRECTORY_RECORDS_PER_PAGE].raw());
        assert_eq!(
            registry.state(handles[DIRECTORY_RECORDS_PER_PAGE + 3]),
            Ok(ServiceState::Stopped)
        );
    }

    #[test]
    fn dependency_start_and_stale_reuse_are_generation_safe() {
        let mut registry = RuntimeServiceRegistry::new();
        let dependency = registry.register(b"dep", b"image", &[]).unwrap();
        let service = registry.register(b"svc", b"image", &[dependency]).unwrap();
        registry.start(service).unwrap();
        assert_eq!(registry.state(dependency), Ok(ServiceState::Running));
        registry.remove(service).unwrap();
        assert_eq!(registry.state(service), Err(ServiceRegistryError::Stale));
        let replacement = registry.register(b"new", b"image", &[]).unwrap();
        assert_ne!(service, replacement);
    }

    #[test]
    fn failed_dependency_start_does_not_leave_partial_running_state() {
        let mut registry = RuntimeServiceRegistry::new();
        let first = registry.register(b"first", b"image", &[]).unwrap();
        let second = registry.register(b"second", b"image", &[]).unwrap();
        let root = registry.register(b"root", b"image", &[first, second]).unwrap();
        registry.set_dependencies(second, &[root]).unwrap();

        assert_eq!(registry.start(root), Err(ServiceRegistryError::DependencyCycle));
        assert_eq!(registry.state(first), Ok(ServiceState::Stopped));
        assert_eq!(registry.state(second), Ok(ServiceState::Stopped));
        assert_eq!(registry.state(root), Ok(ServiceState::Stopped));
    }

    #[test]
    fn manager_requests_use_dynamic_records_and_opaque_cursors() {
        let mut registry = RuntimeServiceRegistry::new();
        let mut handles = Vec::new();
        for index in 0..12 {
            handles.push(registry.register(format_name(index).as_slice(), b"image", &[]).unwrap());
        }
        let mut request = ManagerRequest::new(ManagerOperation::List, 1);
        let mut listed = 0;
        loop {
            let response = registry.manager_request(request);
            assert_eq!(response.status, ManagerStatus::Ok);
            listed += usize::from(response.record.name_len != 0);
            if response.cursor == u64::MAX {
                break;
            }
            request.cursor = response.cursor;
        }
        assert_eq!(listed, handles.len());

        let mut status = ManagerRequest::new(ManagerOperation::Status, 2);
        status.service = handles[11];
        assert_eq!(registry.manager_request(status).status, ManagerStatus::Ok);
        registry.remove(handles[11]).unwrap();
        assert_eq!(registry.manager_request(status).status, ManagerStatus::Stale);
    }

    #[test]
    fn lifecycle_status_uses_dynamic_state_and_dependencies() {
        let mut registry = RuntimeServiceRegistry::new();
        let dependency = registry.register(b"dep", b"image", &[]).unwrap();
        let service = registry.register(b"svc", b"image", &[dependency]).unwrap();

        assert_eq!(
            registry.lifecycle_status(ManagerOperation::Start, service),
            ManagerStatus::Dependency
        );
        registry.start(dependency).unwrap();
        assert_eq!(registry.lifecycle_status(ManagerOperation::Start, service), ManagerStatus::Ok);
        assert_eq!(
            registry.begin_lifecycle(ManagerOperation::Start, service),
            ManagerStatus::Accepted
        );
        assert_eq!(
            registry.lifecycle_status(ManagerOperation::Start, service),
            ManagerStatus::Busy
        );
        registry.start(service).unwrap();
        assert_eq!(
            registry.lifecycle_status(ManagerOperation::Start, service),
            ManagerStatus::InvalidState
        );
        assert_eq!(
            registry.lifecycle_status(ManagerOperation::Stop, dependency),
            ManagerStatus::Dependency
        );
        assert_eq!(registry.lifecycle_status(ManagerOperation::Stop, service), ManagerStatus::Ok);
        registry.stop(service).unwrap();
        assert_eq!(
            registry.lifecycle_status(ManagerOperation::Restart, service),
            ManagerStatus::InvalidState
        );
    }

    #[test]
    fn failed_lifecycle_admission_is_rolled_back() {
        let mut registry = RuntimeServiceRegistry::new();
        let service = registry.register(b"service", b"image", &[]).unwrap();

        assert_eq!(
            registry.begin_lifecycle(ManagerOperation::Start, service),
            ManagerStatus::Accepted
        );
        assert_eq!(registry.abort_lifecycle(ManagerOperation::Start, service), ManagerStatus::Ok);
        assert_eq!(registry.state(service), Ok(ServiceState::Stopped));

        registry.start(service).unwrap();
        assert_eq!(
            registry.begin_lifecycle(ManagerOperation::Restart, service),
            ManagerStatus::Accepted
        );
        assert_eq!(registry.abort_lifecycle(ManagerOperation::Restart, service), ManagerStatus::Ok);
        assert_eq!(registry.state(service), Ok(ServiceState::Running));
    }

    #[test]
    fn lifecycle_action_builds_dynamic_restart_closure() {
        let mut registry = RuntimeServiceRegistry::new();
        let dependency = registry.register(b"dependency", b"image", &[]).unwrap();
        let service = registry.register(b"service", b"image", &[dependency]).unwrap();
        let dependent = registry.register(b"dependent", b"image", &[service]).unwrap();
        registry.start(dependent).unwrap();

        let action = registry.begin_lifecycle_action(ManagerOperation::Restart, service).unwrap();
        let RuntimeLifecycleAction::Restart(order) = action else {
            panic!("restart must return a closure");
        };
        assert_eq!(order.as_slice(), &[dependent, service]);
        assert_eq!(registry.state(dependent), Ok(ServiceState::Stopping));
        assert_eq!(registry.state(service), Ok(ServiceState::Stopping));
        assert_eq!(registry.state(dependency), Ok(ServiceState::Running));
    }

    #[test]
    fn lifecycle_action_rollback_restores_restart_members() {
        let mut registry = RuntimeServiceRegistry::new();
        let service = registry.register(b"service", b"image", &[]).unwrap();
        registry.start(service).unwrap();
        let action = registry.begin_lifecycle_action(ManagerOperation::Restart, service).unwrap();
        let RuntimeLifecycleAction::Restart(members) = action else {
            panic!("restart must return members");
        };
        assert_eq!(registry.abort_lifecycle_members(&members), ManagerStatus::Ok);
        assert_eq!(registry.state(service), Ok(ServiceState::Running));
    }

    #[test]
    fn heap_quota_is_recorded_with_the_service_handle() {
        let mut registry = RuntimeServiceRegistry::new();
        let handle = registry.register_with_quota(b"quota", b"image", &[], 7).unwrap();
        assert_eq!(registry.heap_quota_pages(handle), Ok(7));
        assert_eq!(
            registry.register_with_quota(b"zero", b"image", &[], 0),
            Err(ServiceRegistryError::InvalidImage)
        );
    }

    #[test]
    fn manager_rights_are_recorded_on_the_dynamic_service() {
        let mut registry = RuntimeServiceRegistry::new();
        let handle = registry.register(b"service", b"image", &[]).unwrap();
        assert_eq!(registry.manager_rights(handle), Ok(ManagerRights::NONE));
        registry.set_manager_rights(handle, ManagerRights::ALL).unwrap();
        assert_eq!(registry.manager_rights(handle), Ok(ManagerRights::ALL));
    }

    #[test]
    fn image_source_is_recorded_on_the_dynamic_service() {
        let mut registry = RuntimeServiceRegistry::new();
        let handle = registry.register(b"service", b"image", &[]).unwrap();
        assert_eq!(registry.image_source(handle), Ok(ServiceImageSource::Builtin));
        registry.set_image_source(handle, ServiceImageSource::FilesystemPackage).unwrap();
        assert_eq!(registry.image_source(handle), Ok(ServiceImageSource::FilesystemPackage));
    }

    #[test]
    fn runtime_ownership_is_handle_scoped_and_clearable() {
        let mut registry = RuntimeServiceRegistry::new();
        let handle = registry.register(b"service", b"image", &[]).unwrap();
        assert_eq!(registry.set_runtime_ownership(handle, 11, 22, 33, 2), Ok(()));
        registry.set_heartbeat(handle, 44).unwrap();
        assert_eq!(
            registry.ownership(handle),
            Ok(ServiceOwnership {
                process: 11,
                address_space: 22,
                task: 33,
                heap_pages: 2,
                heartbeat: 44,
                ipc_endpoints: 0,
                capabilities: 0,
                events: 0,
            })
        );
        assert_eq!(
            registry.set_runtime_ownership(handle, 0, 22, 33, 2),
            Err(ServiceRegistryError::Capacity)
        );
        registry.set_runtime_counts(handle, 3, 4, 5).unwrap();
        registry.clear_execution_ownership(handle).unwrap();
        assert_eq!(
            registry.ownership(handle),
            Ok(ServiceOwnership {
                process: 0,
                address_space: 0,
                task: 0,
                heap_pages: 0,
                heartbeat: 0,
                ipc_endpoints: 3,
                capabilities: 4,
                events: 5,
            })
        );
        registry.clear_runtime_ownership(handle).unwrap();
        assert_eq!(registry.ownership(handle), Ok(ServiceOwnership::EMPTY));
        registry.remove(handle).unwrap();
        assert_eq!(registry.ownership(handle), Err(ServiceRegistryError::Stale));
    }

    #[test]
    fn image_metadata_updates_without_replacing_the_service_handle() {
        let mut registry = RuntimeServiceRegistry::new();
        let handle = registry.register(b"service", b"builtin", &[]).unwrap();
        assert_eq!(registry.image_len(handle), Ok(7));
        registry.set_image(handle, b"package").unwrap();
        assert_eq!(registry.image_len(handle), Ok(7));
        assert_eq!(registry.set_image(handle, b""), Err(ServiceRegistryError::InvalidImage));
        assert_eq!(registry.image_len(handle), Ok(7));
    }

    #[test]
    fn record_restart_updates_epoch_and_manager_state() {
        let mut registry = RuntimeServiceRegistry::new();
        let handle = registry.register(b"service", b"image", &[]).unwrap();
        registry.start(handle).unwrap();
        assert_eq!(registry.epoch(handle), Ok(1));
        registry.record_restart(handle).unwrap();
        assert_eq!(registry.epoch(handle), Ok(2));
        let mut request = ManagerRequest::new(ManagerOperation::Status, 1);
        request.service = handle;
        let response = registry.manager_request(request);
        assert_eq!(response.status, ManagerStatus::Ok);
        assert_eq!(response.record.restarts, 1);
        assert_eq!(response.record.state, ManagerState::Running);
    }

    #[test]
    fn failed_service_state_is_dynamic_and_generation_checked() {
        let mut registry = RuntimeServiceRegistry::new();
        let handle = registry.register(b"service", b"image", &[]).unwrap();
        registry.start(handle).unwrap();
        registry.fail(handle).unwrap();

        let mut request = ManagerRequest::new(ManagerOperation::Status, 1);
        request.service = handle;
        let response = registry.manager_request(request);
        assert_eq!(response.status, ManagerStatus::Ok);
        assert_eq!(response.record.state, ManagerState::Failed);

        let stale =
            ServiceHandle::new(handle.index(), handle.generation().wrapping_add(1)).unwrap();
        request.service = stale;
        assert_eq!(registry.manager_request(request).status, ManagerStatus::Stale);
    }

    #[test]
    fn registry_generation_seed_rejects_previous_runtime_handles() {
        let mut first = RuntimeServiceRegistry::new_with_generation(3);
        let old = first.register(b"old", b"image", &[]).unwrap();
        let mut replacement = RuntimeServiceRegistry::new_with_generation(4);
        let current = replacement.register(b"new", b"image", &[]).unwrap();
        assert_eq!(old.generation(), 3);
        assert_eq!(current.generation(), 4);
        assert_eq!(replacement.state(old), Err(ServiceRegistryError::Stale));
    }

    fn format_name(index: usize) -> Vec<u8> {
        let mut name = b"service".to_vec();
        name.push(b'a' + index as u8);
        name
    }
}
