//! Bounded program lifecycle state and manager decision shapes.

use alloc::vec::Vec;

use logos_abi::{
    MAX_PACKAGE_NAME_BYTES, ManagerOperation, ManagerRequest, ManagerRights, ManagerState,
    ManagerStatus, ManagerTargetKind, ServiceManagerRecord,
};

pub const MAX_PROGRAM_SLOTS: usize = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagerAction {
    None,
    Start(logos_abi::ServiceId),
    Stop(logos_abi::ServiceId),
    Restart(Vec<logos_abi::ServiceId>),
    ProgramStart(usize),
    ProgramStop(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagerDecision {
    pub response: logos_abi::ManagerResponse,
    pub action: ManagerAction,
}

#[derive(Clone, Copy)]
struct ProgramSlot {
    name: [u8; MAX_PACKAGE_NAME_BYTES],
    name_len: u8,
    generation: u32,
    state: ManagerState,
}

impl ProgramSlot {
    const EMPTY: Self = Self {
        name: [0; MAX_PACKAGE_NAME_BYTES],
        name_len: 0,
        generation: 1,
        state: ManagerState::Vacant,
    };

    const fn record(self, slot: usize) -> ServiceManagerRecord {
        ServiceManagerRecord {
            service: logos_abi::ServiceHandle::EMPTY,
            state: self.state,
            restarts: 0,
            name_len: self.name_len,
            dependencies: 0,
            reserved: [0; 2],
            program_slot: slot as u8,
            reserved_program: [0; 3],
            program_generation: self.generation,
            name: self.name,
        }
    }
}

pub struct ProgramManager {
    programs: [ProgramSlot; MAX_PROGRAM_SLOTS],
}

impl ProgramManager {
    pub const fn new() -> Self {
        Self { programs: [ProgramSlot::EMPTY; MAX_PROGRAM_SLOTS] }
    }

    pub fn program_record(&self, slot: usize) -> Option<ServiceManagerRecord> {
        if slot >= self.programs.len() || self.programs[slot].state == ManagerState::Vacant {
            None
        } else {
            Some(self.programs[slot].record(slot))
        }
    }

    pub fn mark_program_running(&mut self, slot: usize, generation: u32) -> bool {
        let Some(program) = self.programs.get_mut(slot) else { return false };
        if program.generation != generation || program.state != ManagerState::Starting {
            return false;
        }
        program.state = ManagerState::Running;
        true
    }

    pub fn mark_program_stopping(&mut self, slot: usize, generation: u32) -> bool {
        let Some(program) = self.programs.get_mut(slot) else { return false };
        if program.generation != generation || program.state != ManagerState::Running {
            return false;
        }
        program.state = ManagerState::Stopping;
        true
    }

    pub fn mark_program_terminal(
        &mut self,
        slot: usize,
        generation: u32,
        state: ManagerState,
    ) -> bool {
        let Some(program) = self.programs.get_mut(slot) else { return false };
        if program.generation != generation
            || !matches!(program.state, ManagerState::Running | ManagerState::Stopping)
        {
            return false;
        }
        program.state = state;
        true
    }

    pub fn mark_program_failed(&mut self, slot: usize, generation: u32) -> bool {
        let Some(program) = self.programs.get_mut(slot) else { return false };
        if program.generation != generation || program.state != ManagerState::Starting {
            return false;
        }
        program.state = ManagerState::Faulted;
        true
    }

    pub fn mark_program_stopped(&mut self, slot: usize, generation: u32) -> bool {
        let Some(program) = self.programs.get_mut(slot) else { return false };
        if program.generation != generation
            || !matches!(program.state, ManagerState::Stopping | ManagerState::Running)
        {
            return false;
        }
        program.state = ManagerState::Stopped;
        program.generation = program.generation.wrapping_add(1).max(1);
        true
    }

    pub fn request(&mut self, request: ManagerRequest, rights: ManagerRights) -> ManagerDecision {
        let mut response = logos_abi::ManagerResponse::new(
            request.operation,
            ManagerStatus::Malformed,
            request.request_id,
        );
        if request.abi_version != logos_abi::MANAGER_ABI_VERSION
            || request.request_id == 0
            || !rights.contains(if request.operation.requires_lifecycle() {
                ManagerRights::LIFECYCLE
            } else {
                ManagerRights::INSPECT
            })
            || request.target_kind != ManagerTargetKind::Program
            || request.service != logos_abi::ServiceHandle::EMPTY
            || request.cursor != 0
            || request.name_len as usize > request.name.len()
            || request.name[request.name_len as usize..].iter().any(|byte| *byte != 0)
            || logos_abi::PackageTarget::program(request.name()).is_none()
        {
            response.status = if request.abi_version == logos_abi::MANAGER_ABI_VERSION
                && request.request_id != 0
                && rights.contains(ManagerRights::INSPECT)
            {
                ManagerStatus::Malformed
            } else {
                ManagerStatus::Unauthorized
            };
            return ManagerDecision { response, action: ManagerAction::None };
        }
        match request.operation {
            ManagerOperation::ProgramStart => {
                if request.program_slot != u8::MAX || request.program_generation != 0 {
                    return ManagerDecision { response, action: ManagerAction::None };
                }
                let slot = match self.find_program(request.name()) {
                    Some(slot) => slot,
                    None => {
                        let Some(slot) = self
                            .programs
                            .iter()
                            .position(|program| program.state == ManagerState::Vacant)
                        else {
                            response.status = ManagerStatus::Capacity;
                            return ManagerDecision { response, action: ManagerAction::None };
                        };
                        self.programs[slot].name[..request.name_len as usize]
                            .copy_from_slice(request.name());
                        self.programs[slot].name_len = request.name_len;
                        slot
                    }
                };
                if !matches!(
                    self.programs[slot].state,
                    ManagerState::Vacant
                        | ManagerState::Stopped
                        | ManagerState::Exited
                        | ManagerState::Faulted
                ) {
                    response.status = ManagerStatus::InvalidState;
                } else {
                    if matches!(
                        self.programs[slot].state,
                        ManagerState::Exited | ManagerState::Faulted
                    ) {
                        self.programs[slot].generation =
                            self.programs[slot].generation.wrapping_add(1).max(1);
                    }
                    self.programs[slot].state = ManagerState::Starting;
                    response.status = ManagerStatus::Accepted;
                    response.record = self.programs[slot].record(slot);
                    return ManagerDecision { response, action: ManagerAction::ProgramStart(slot) };
                }
            }
            ManagerOperation::ProgramStatus => {
                let index = match self.valid_program_index(&request) {
                    Ok(index) => index,
                    Err(status) => {
                        response.status = status;
                        return ManagerDecision { response, action: ManagerAction::None };
                    }
                };
                response.status = ManagerStatus::Ok;
                response.record = self.programs[index].record(index);
            }
            ManagerOperation::ProgramStop => {
                let index = match self.valid_program_index(&request) {
                    Ok(index) => index,
                    Err(status) => {
                        response.status = status;
                        return ManagerDecision { response, action: ManagerAction::None };
                    }
                };
                if self.programs[index].state != ManagerState::Running {
                    response.status = ManagerStatus::InvalidState;
                } else {
                    self.programs[index].state = ManagerState::Stopping;
                    response.status = ManagerStatus::Accepted;
                    response.record = self.programs[index].record(index);
                    return ManagerDecision { response, action: ManagerAction::ProgramStop(index) };
                }
            }
            _ => response.status = ManagerStatus::Malformed,
        }
        ManagerDecision { response, action: ManagerAction::None }
    }

    fn find_program(&self, name: &[u8]) -> Option<usize> {
        self.programs.iter().position(|program| {
            program.state != ManagerState::Vacant
                && &program.name[..program.name_len as usize] == name
        })
    }

    fn valid_program_index(&self, request: &ManagerRequest) -> Result<usize, ManagerStatus> {
        if request.program_slot == u8::MAX {
            return self.find_program(request.name()).ok_or(ManagerStatus::NotFound);
        }
        let index = request.program_slot as usize;
        let Some(program) = self.programs.get(index) else {
            return Err(ManagerStatus::NotFound);
        };
        if program.state == ManagerState::Vacant
            || program.name_len != request.name_len
            || &program.name[..program.name_len as usize] != request.name()
        {
            Err(ManagerStatus::NotFound)
        } else if program.generation != request.program_generation {
            Err(ManagerStatus::Stale)
        } else {
            Ok(index)
        }
    }
}

impl Default for ProgramManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_manager_keeps_generation_safe_bounded_lifecycle() {
        let mut manager = ProgramManager::new();
        let request = ManagerRequest::new(ManagerOperation::ProgramStart, 1)
            .with_program_name(b"demo")
            .unwrap();
        let decision = manager.request(request, ManagerRights::ALL);
        assert_eq!(decision.response.status, ManagerStatus::Accepted);
        assert!(manager.mark_program_running(0, 1));

        let mut status = ManagerRequest::new(ManagerOperation::ProgramStatus, 2)
            .with_program_name(b"demo")
            .unwrap();
        status.program_slot = 0;
        status.program_generation = 1;
        assert_eq!(
            manager.request(status, ManagerRights::INSPECT).response.status,
            ManagerStatus::Ok
        );
        status.program_generation = 2;
        assert_eq!(
            manager.request(status, ManagerRights::INSPECT).response.status,
            ManagerStatus::Stale
        );
    }
}
