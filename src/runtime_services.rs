//! Dynamic service records used by the v5 manager migration.

use alloc::vec::Vec;

use logos_abi::{
    DIRECTORY_FLAG_MORE, DIRECTORY_RECORDS_PER_PAGE, DirectoryOperation, DirectoryRecord,
    DirectoryResponse, DirectoryStatus, MAX_PACKAGE_NAME_BYTES, MAX_SERVICE_NAME_BYTES,
    ManagerOperation, ManagerRequest, ManagerResponse, ManagerState, ManagerStatus,
    SERVICE_HEAP_MAX_PAGES, ServiceHandle, ServiceManagerRecord,
};

struct Slot {
    generation: u32,
    value: Option<ServiceRecord>,
}

impl Slot {
    fn empty() -> Self {
        Self { generation: 1, value: None }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceState {
    Disabled,
    Stopped,
    Running,
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

pub struct RuntimeServiceRegistry {
    slots: Vec<Slot>,
}

impl RuntimeServiceRegistry {
    pub fn new() -> Self {
        Self { slots: Vec::new() }
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
        });
        Ok(handle)
    }

    pub fn start(&mut self, handle: ServiceHandle) -> Result<(), ServiceRegistryError> {
        let mut visiting = Vec::new();
        self.start_inner(handle, &mut visiting)
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

    pub fn restart(&mut self, handle: ServiceHandle) -> Result<(), ServiceRegistryError> {
        let index = self.index(handle)?;
        let dependencies = self.service(handle)?.dependencies.clone();
        let mut visiting = Vec::new();
        for dependency in dependencies {
            self.start_inner(dependency, &mut visiting)?;
        }
        let service = self.slots[index].value.as_mut().ok_or(ServiceRegistryError::Stale)?;
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

    pub fn heap_quota_pages(&self, handle: ServiceHandle) -> Result<usize, ServiceRegistryError> {
        Ok(self.service(handle)?.heap_quota_pages)
    }

    pub fn manager_request(&self, request: ManagerRequest) -> ManagerResponse {
        let mut response =
            ManagerResponse::new(request.operation, ManagerStatus::Malformed, request.request_id);
        if request.request_id == 0 || request.abi_version != logos_abi::MANAGER_ABI_VERSION {
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
        let mut seen = 0u64;
        let mut written = 0usize;
        for service in self.slots.iter().filter_map(|slot| slot.value.as_ref()) {
            if seen < cursor {
                seen += 1;
                continue;
            }
            if written == DIRECTORY_RECORDS_PER_PAGE {
                response.flags |= DIRECTORY_FLAG_MORE;
                response.cursor = cursor + written as u64;
                break;
            }
            let mut record = DirectoryRecord::service(service.handle, &service.name)
                .expect("registry names are validated at registration");
            record.flags = match service.state {
                ServiceState::Disabled => 2,
                ServiceState::Stopped => 0,
                ServiceState::Running => 1,
            };
            response.records[written] = record;
            written += 1;
            seen += 1;
        }
        response.count = written as u8;
        if response.flags & DIRECTORY_FLAG_MORE == 0 {
            response.cursor = cursor + written as u64;
        }
        DirectoryStatus::Ok
    }

    fn start_inner(
        &mut self,
        handle: ServiceHandle,
        visiting: &mut Vec<ServiceHandle>,
    ) -> Result<(), ServiceRegistryError> {
        if visiting.contains(&handle) {
            return Err(ServiceRegistryError::DependencyCycle);
        }
        let dependencies = self.service(handle)?.dependencies.clone();
        if self.state(handle)? == ServiceState::Running {
            return Ok(());
        }
        visiting.push(handle);
        for dependency in dependencies {
            self.start_inner(dependency, visiting)?;
        }
        visiting.pop();
        let service = self.service_mut(handle)?;
        service.state = ServiceState::Running;
        Ok(())
    }

    fn allocate_slot(&mut self) -> Result<usize, ServiceRegistryError> {
        if let Some((index, _)) =
            self.slots.iter().enumerate().find(|(_, slot)| slot.value.is_none())
        {
            return Ok(index);
        }
        self.slots.try_reserve(1).map_err(|_| ServiceRegistryError::Capacity)?;
        self.slots.push(Slot::empty());
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
                ServiceState::Running => ManagerState::Running,
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
        for index in 0..12 {
            handles.push(registry.register(format_name(index).as_slice(), b"image", &[]).unwrap());
        }
        let mut response =
            DirectoryResponse::empty(DirectoryOperation::Services, DirectoryStatus::Malformed, 1);
        registry.list(0, &mut response, 1);
        assert_eq!(response.count as usize, DIRECTORY_RECORDS_PER_PAGE.min(12));
        assert_eq!(registry.state(handles[11]), Ok(ServiceState::Stopped));
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
    fn heap_quota_is_recorded_with_the_service_handle() {
        let mut registry = RuntimeServiceRegistry::new();
        let handle = registry.register_with_quota(b"quota", b"image", &[], 7).unwrap();
        assert_eq!(registry.heap_quota_pages(handle), Ok(7));
        assert_eq!(
            registry.register_with_quota(b"zero", b"image", &[], 0),
            Err(ServiceRegistryError::InvalidImage)
        );
    }

    fn format_name(index: usize) -> Vec<u8> {
        let mut name = b"service".to_vec();
        name.push(b'a' + index as u8);
        name
    }
}
