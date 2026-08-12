//! Fixed user-process and ELF admission model.

pub const MAX_USER_PROCESSES: usize = 16;
pub const MAX_SERVICE_PROCESSES: usize = 5;
pub const MAX_COMMAND_PROCESSES: usize = 8;
pub const MAX_RESERVED_PROCESSES: usize = 2;
pub const USER_STACK_PAGES: usize = 8;
pub const MAX_IMAGE_BYTES: usize = 512 * 1024;
pub const MAX_PROGRAM_HEADERS: usize = 16;

const ELF_MAGIC: &[u8; 4] = b"\x7fELF";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessKind {
    Input,
    Display,
    Terminal,
    Session,
    Command,
    Supervisor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessState {
    Vacant,
    Starting,
    Running,
    Exited(u8),
    Faulted(u8),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessError {
    Capacity,
    InvalidImage,
    InvalidHandle,
    NotRunning,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessHandle {
    slot: u8,
    generation: u64,
}

impl ProcessHandle {
    pub const fn slot(self) -> usize {
        self.slot as usize
    }
    pub const fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capabilities {
    pub input: bool,
    pub display: bool,
    pub endpoints: bool,
    pub process_control: bool,
}

impl Capabilities {
    pub const NONE: Self =
        Self { input: false, display: false, endpoints: false, process_control: false };
    pub const SERVICE: Self =
        Self { input: true, display: true, endpoints: true, process_control: false };
    pub const SESSION: Self =
        Self { input: false, display: false, endpoints: true, process_control: true };
    pub const COMMAND: Self =
        Self { input: false, display: false, endpoints: true, process_control: false };
}

#[derive(Clone, Copy)]
struct ProcessSlot {
    generation: u64,
    state: ProcessState,
    kind: ProcessKind,
    capabilities: Capabilities,
    entry: u64,
}

impl ProcessSlot {
    const EMPTY: Self = Self {
        generation: 1,
        state: ProcessState::Vacant,
        kind: ProcessKind::Command,
        capabilities: Capabilities::NONE,
        entry: 0,
    };
}

pub struct ProcessTable {
    slots: [ProcessSlot; MAX_USER_PROCESSES],
}

impl ProcessTable {
    pub const fn new() -> Self {
        Self { slots: [ProcessSlot::EMPTY; MAX_USER_PROCESSES] }
    }

    pub fn start(
        &mut self,
        image: &[u8],
        kind: ProcessKind,
        capabilities: Capabilities,
    ) -> Result<ProcessHandle, ProcessError> {
        let entry = validate_elf(image)?;
        let Some((slot, process)) = self
            .slots
            .iter_mut()
            .enumerate()
            .find(|(_, process)| process.state == ProcessState::Vacant)
        else {
            return Err(ProcessError::Capacity);
        };
        process.state = ProcessState::Starting;
        process.kind = kind;
        process.capabilities = capabilities;
        process.entry = entry;
        process.state = ProcessState::Running;
        Ok(ProcessHandle { slot: slot as u8, generation: process.generation })
    }

    pub fn state(&self, handle: ProcessHandle) -> Option<ProcessState> {
        let process = self.slots.get(handle.slot as usize)?;
        (process.generation == handle.generation).then_some(process.state)
    }

    pub fn capabilities(&self, handle: ProcessHandle) -> Option<Capabilities> {
        let process = self.slots.get(handle.slot as usize)?;
        (process.generation == handle.generation).then_some(process.capabilities)
    }

    pub fn exit(&mut self, handle: ProcessHandle, status: u8) -> Result<(), ProcessError> {
        let process = self.current_mut(handle)?;
        if process.state != ProcessState::Running {
            return Err(ProcessError::NotRunning);
        }
        process.state = ProcessState::Exited(status);
        Ok(())
    }

    pub fn fault(&mut self, handle: ProcessHandle, vector: u8) -> Result<(), ProcessError> {
        let process = self.current_mut(handle)?;
        if process.state != ProcessState::Running {
            return Err(ProcessError::NotRunning);
        }
        process.state = ProcessState::Faulted(vector);
        Ok(())
    }

    pub fn reclaim(&mut self, handle: ProcessHandle) -> Result<(), ProcessError> {
        let process = self.current_mut(handle)?;
        if !matches!(process.state, ProcessState::Exited(_) | ProcessState::Faulted(_)) {
            return Err(ProcessError::NotRunning);
        }
        process.state = ProcessState::Vacant;
        process.entry = 0;
        process.capabilities = Capabilities::NONE;
        process.generation = next_generation(process.generation);
        Ok(())
    }

    fn current_mut(&mut self, handle: ProcessHandle) -> Result<&mut ProcessSlot, ProcessError> {
        let Some(process) = self.slots.get_mut(handle.slot as usize) else {
            return Err(ProcessError::InvalidHandle);
        };
        if process.generation != handle.generation {
            return Err(ProcessError::InvalidHandle);
        }
        Ok(process)
    }
}

impl Default for ProcessTable {
    fn default() -> Self {
        Self::new()
    }
}

pub fn validate_elf(image: &[u8]) -> Result<u64, ProcessError> {
    if image.len() < 64
        || image.len() > MAX_IMAGE_BYTES
        || &image[..4] != ELF_MAGIC
        || image[4] != 2
        || image[5] != 1
        || read_u16(image, 18) != Some(0x3e)
        || read_u16(image, 16) != Some(2)
    {
        return Err(ProcessError::InvalidImage);
    }
    let entry = read_u64(image, 24).ok_or(ProcessError::InvalidImage)?;
    let ph_offset = read_u64(image, 32).ok_or(ProcessError::InvalidImage)? as usize;
    let ph_entry_size = read_u16(image, 54).ok_or(ProcessError::InvalidImage)? as usize;
    let ph_count = read_u16(image, 56).ok_or(ProcessError::InvalidImage)? as usize;
    if entry == 0
        || ph_entry_size < 56
        || ph_count == 0
        || ph_count > MAX_PROGRAM_HEADERS
        || ph_offset.checked_add(ph_entry_size * ph_count).is_none()
        || ph_offset + ph_entry_size * ph_count > image.len()
    {
        return Err(ProcessError::InvalidImage);
    }
    let mut load_segments = 0;
    for index in 0..ph_count {
        let offset = ph_offset + index * ph_entry_size;
        let kind = read_u32_at(image, offset).ok_or(ProcessError::InvalidImage)?;
        if kind != 1 {
            continue;
        }
        load_segments += 1;
        let flags = read_u32_at(image, offset + 4).ok_or(ProcessError::InvalidImage)?;
        let file_offset =
            read_u64_at(image, offset + 8).ok_or(ProcessError::InvalidImage)? as usize;
        let virtual_address = read_u64_at(image, offset + 16).ok_or(ProcessError::InvalidImage)?;
        let file_size = read_u64_at(image, offset + 32).ok_or(ProcessError::InvalidImage)? as usize;
        let memory_size =
            read_u64_at(image, offset + 40).ok_or(ProcessError::InvalidImage)? as usize;
        if memory_size == 0
            || file_size > memory_size
            || file_offset.checked_add(file_size).is_none()
            || file_offset + file_size > image.len()
            || virtual_address < 0x1000
            || virtual_address.checked_add(memory_size as u64).is_none()
            || flags & 0x1 != 0 && flags & 0x2 != 0
        {
            return Err(ProcessError::InvalidImage);
        }
    }
    (load_segments > 0).then_some(entry).ok_or(ProcessError::InvalidImage)
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    read_u16_at(bytes, offset)
}
fn read_u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*bytes.get(offset)?, *bytes.get(offset + 1)?]))
}
fn read_u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *bytes.get(offset)?,
        *bytes.get(offset + 1)?,
        *bytes.get(offset + 2)?,
        *bytes.get(offset + 3)?,
    ]))
}
fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    read_u64_at(bytes, offset)
}
fn read_u64_at(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes([
        *bytes.get(offset)?,
        *bytes.get(offset + 1)?,
        *bytes.get(offset + 2)?,
        *bytes.get(offset + 3)?,
        *bytes.get(offset + 4)?,
        *bytes.get(offset + 5)?,
        *bytes.get(offset + 6)?,
        *bytes.get(offset + 7)?,
    ]))
}
fn next_generation(generation: u64) -> u64 {
    generation.wrapping_add(1).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> [u8; 128] {
        let mut image = [0; 128];
        image[..4].copy_from_slice(b"\x7fELF");
        image[4] = 2;
        image[5] = 1;
        image[16..18].copy_from_slice(&2u16.to_le_bytes());
        image[18..20].copy_from_slice(&0x3eu16.to_le_bytes());
        image[24..32].copy_from_slice(&0x1000u64.to_le_bytes());
        image[32..40].copy_from_slice(&64u64.to_le_bytes());
        image[54..56].copy_from_slice(&56u16.to_le_bytes());
        image[56..58].copy_from_slice(&1u16.to_le_bytes());
        image[64..68].copy_from_slice(&1u32.to_le_bytes());
        image[68..72].copy_from_slice(&4u32.to_le_bytes());
        image[72..80].copy_from_slice(&0u64.to_le_bytes());
        image[80..88].copy_from_slice(&0x1000u64.to_le_bytes());
        image[96..104].copy_from_slice(&1u64.to_le_bytes());
        image[104..112].copy_from_slice(&0x1000u64.to_le_bytes());
        image
    }

    #[test]
    fn elf_validation_is_bounded_and_process_faults_are_contained() {
        let image = image();
        assert_eq!(validate_elf(&image), Ok(0x1000));
        let mut table = ProcessTable::new();
        let process = table.start(&image, ProcessKind::Terminal, Capabilities::SERVICE).unwrap();
        assert_eq!(table.state(process), Some(ProcessState::Running));
        assert!(table.fault(process, 14).is_ok());
        assert_eq!(table.state(process), Some(ProcessState::Faulted(14)));
        assert!(table.reclaim(process).is_ok());
        assert_eq!(table.state(process), None);
    }

    #[test]
    fn writable_executable_segments_are_rejected() {
        let mut image = image();
        image[68..72].copy_from_slice(&7u32.to_le_bytes());
        assert_eq!(validate_elf(&image), Err(ProcessError::InvalidImage));
    }
}
