//! Fixed user-process and ELF admission model.

pub const MAX_USER_PROCESSES: usize = 16;
pub const MAX_ADDRESS_SPACES: usize = MAX_USER_PROCESSES;
pub const MAX_MAPPINGS_PER_ADDRESS_SPACE: usize = 64;
pub const USER_STACK_PAGES: usize = 8;
/// Storage uses bounded journal replay and transaction shadow state.
pub const STORAGE_STACK_PAGES: usize = 128;
/// Network protocol polling uses bounded smoltcp packet-processing frames.
pub const NETWORK_STACK_PAGES: usize = 128;
/// Flow parsing and type checking use bounded arenas with a deeper call chain.
pub const FLOW_STACK_PAGES: usize = 128;
pub const MAX_IMAGE_BYTES: usize = 512 * 1024;
pub const MAX_PROGRAM_HEADERS: usize = 16;

const ELF_MAGIC: &[u8; 4] = b"\x7fELF";

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
    AddressSpace,
    ReadFailure,
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

    pub const fn raw(self) -> u64 {
        (self.generation << 8) | self.slot as u64
    }

    pub const fn from_raw(raw: u64) -> Self {
        Self { slot: raw as u8, generation: raw >> 8 }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddressSpaceHandle {
    slot: u8,
    generation: u64,
}

impl AddressSpaceHandle {
    pub const fn slot(self) -> usize {
        self.slot as usize
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddressSpaceRoot(usize);

impl AddressSpaceRoot {
    pub const fn new(raw: usize) -> Option<Self> {
        if raw != 0 && raw & 0xfff == 0 { Some(Self(raw)) } else { None }
    }

    pub const fn raw(self) -> usize {
        self.0
    }
}

/// Immutable register metadata needed to enter a loaded user image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserLaunch {
    entry: usize,
    stack_top: usize,
    address_space_root: AddressSpaceRoot,
}

impl UserLaunch {
    pub const fn new(
        entry: usize,
        stack_top: usize,
        address_space_root: AddressSpaceRoot,
    ) -> Option<Self> {
        if entry == 0
            || entry >= 0x0000_8000_0000_0000
            || stack_top == 0
            || stack_top & 0xfff != 0
            || stack_top > 0x0000_8000_0000_0000
        {
            return None;
        }
        Some(Self { entry, stack_top, address_space_root })
    }

    pub const fn entry(self) -> usize {
        self.entry
    }

    pub const fn stack_top(self) -> usize {
        self.stack_top
    }

    pub const fn address_space_root(self) -> AddressSpaceRoot {
        self.address_space_root
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MappingFlags {
    pub user: bool,
    pub writable: bool,
    pub executable: bool,
}

impl MappingFlags {
    pub const CODE: Self = Self { user: true, writable: false, executable: true };
    pub const DATA: Self = Self { user: true, writable: true, executable: false };
    pub const READ_ONLY_DATA: Self = Self { user: true, writable: false, executable: false };

    const fn valid(self) -> bool {
        self.user && !(self.writable && self.executable)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtualMapping {
    virtual_address: usize,
    physical_address: usize,
    pages: usize,
    flags: MappingFlags,
}

impl VirtualMapping {
    pub const fn new(
        virtual_address: usize,
        physical_address: usize,
        pages: usize,
        flags: MappingFlags,
    ) -> Option<Self> {
        Self::new_with_limit(
            virtual_address,
            physical_address,
            pages,
            flags,
            MAX_IMAGE_BYTES / 0x1000,
        )
    }

    /// Create a device mapping using the bounded framebuffer range.
    pub const fn new_device(
        virtual_address: usize,
        physical_address: usize,
        pages: usize,
        flags: MappingFlags,
    ) -> Option<Self> {
        Self::new_with_limit(
            virtual_address,
            physical_address,
            pages,
            flags,
            logos_abi::MAX_FRAMEBUFFER_BYTES / 0x1000,
        )
    }

    const fn new_with_limit(
        virtual_address: usize,
        physical_address: usize,
        pages: usize,
        flags: MappingFlags,
        max_pages: usize,
    ) -> Option<Self> {
        let Some(bytes) = pages.checked_mul(0x1000) else {
            return None;
        };
        let Some(virtual_end) = virtual_address.checked_add(bytes) else {
            return None;
        };
        if virtual_address == 0
            || physical_address == 0
            || virtual_address & 0xfff != 0
            || physical_address & 0xfff != 0
            || pages == 0
            || pages > max_pages
            || virtual_address >= 0x0000_8000_0000_0000
            || virtual_end > 0x0000_8000_0000_0000
            || physical_address.checked_add(bytes).is_none()
            || !flags.valid()
        {
            return None;
        }
        Some(Self { virtual_address, physical_address, pages, flags })
    }

    pub const fn virtual_address(self) -> usize {
        self.virtual_address
    }

    pub const fn physical_address(self) -> usize {
        self.physical_address
    }

    pub const fn pages(self) -> usize {
        self.pages
    }

    pub const fn flags(self) -> MappingFlags {
        self.flags
    }

    const fn virtual_end(self) -> usize {
        self.virtual_address + self.pages * 0x1000
    }

    const fn overlaps(self, other: Self) -> bool {
        self.virtual_address < other.virtual_end() && other.virtual_address < self.virtual_end()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AddressSpaceState {
    Vacant,
    Reserved,
}

#[derive(Clone, Copy)]
struct AddressSpaceSlot {
    generation: u64,
    state: AddressSpaceState,
    root: Option<AddressSpaceRoot>,
    mappings: [Option<VirtualMapping>; MAX_MAPPINGS_PER_ADDRESS_SPACE],
}

impl AddressSpaceSlot {
    const EMPTY: Self = Self {
        generation: 1,
        state: AddressSpaceState::Vacant,
        root: None,
        mappings: [None; MAX_MAPPINGS_PER_ADDRESS_SPACE],
    };
}

pub struct AddressSpaceTable {
    slots: [AddressSpaceSlot; MAX_ADDRESS_SPACES],
}

impl AddressSpaceTable {
    pub const fn new() -> Self {
        Self { slots: [AddressSpaceSlot::EMPTY; MAX_ADDRESS_SPACES] }
    }

    pub fn reserve(&mut self) -> Result<AddressSpaceHandle, ProcessError> {
        let Some((slot, address_space)) = self
            .slots
            .iter_mut()
            .enumerate()
            .find(|(_, address_space)| address_space.state == AddressSpaceState::Vacant)
        else {
            return Err(ProcessError::Capacity);
        };
        address_space.state = AddressSpaceState::Reserved;
        address_space.root = None;
        address_space.mappings = [None; MAX_MAPPINGS_PER_ADDRESS_SPACE];
        Ok(AddressSpaceHandle { slot: slot as u8, generation: address_space.generation })
    }

    pub fn bind_root(
        &mut self,
        handle: AddressSpaceHandle,
        root: AddressSpaceRoot,
    ) -> Result<(), ProcessError> {
        let address_space = self.current_mut(handle)?;
        if address_space.root.is_some() {
            return Err(ProcessError::AddressSpace);
        }
        address_space.root = Some(root);
        Ok(())
    }

    pub fn map(
        &mut self,
        handle: AddressSpaceHandle,
        mapping: VirtualMapping,
    ) -> Result<(), ProcessError> {
        let address_space = self.current_mut(handle)?;
        if address_space.root.is_none() {
            return Err(ProcessError::AddressSpace);
        }
        if address_space.mappings.iter().flatten().any(|existing| existing.overlaps(mapping)) {
            return Err(ProcessError::AddressSpace);
        }
        let Some(slot) = address_space.mappings.iter_mut().find(|slot| slot.is_none()) else {
            return Err(ProcessError::Capacity);
        };
        *slot = Some(mapping);
        Ok(())
    }

    pub fn mapping(&self, handle: AddressSpaceHandle, index: usize) -> Option<VirtualMapping> {
        let address_space = self.slots.get(handle.slot as usize)?;
        if address_space.generation != handle.generation
            || address_space.state != AddressSpaceState::Reserved
        {
            return None;
        }
        address_space.mappings.get(index).copied().flatten()
    }

    pub fn root(&self, handle: AddressSpaceHandle) -> Option<AddressSpaceRoot> {
        let address_space = self.slots.get(handle.slot as usize)?;
        (address_space.generation == handle.generation
            && address_space.state == AddressSpaceState::Reserved)
            .then_some(address_space.root)
            .flatten()
    }

    pub fn release(&mut self, handle: AddressSpaceHandle) -> Result<(), ProcessError> {
        let address_space = self.current_mut(handle)?;
        address_space.state = AddressSpaceState::Vacant;
        address_space.root = None;
        address_space.mappings = [None; MAX_MAPPINGS_PER_ADDRESS_SPACE];
        address_space.generation = next_generation(address_space.generation);
        Ok(())
    }

    fn current_mut(
        &mut self,
        handle: AddressSpaceHandle,
    ) -> Result<&mut AddressSpaceSlot, ProcessError> {
        let Some(address_space) = self.slots.get_mut(handle.slot as usize) else {
            return Err(ProcessError::InvalidHandle);
        };
        if address_space.generation != handle.generation
            || address_space.state != AddressSpaceState::Reserved
        {
            return Err(ProcessError::InvalidHandle);
        }
        Ok(address_space)
    }
}

impl Default for AddressSpaceTable {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
struct ProcessSlot {
    generation: u64,
    state: ProcessState,
    entry: u64,
    address_space: Option<AddressSpaceHandle>,
}

impl ProcessSlot {
    const EMPTY: Self =
        Self { generation: 1, state: ProcessState::Vacant, entry: 0, address_space: None };
}

pub struct ProcessTable {
    slots: [ProcessSlot; MAX_USER_PROCESSES],
    address_spaces: AddressSpaceTable,
}

impl ProcessTable {
    pub const fn new() -> Self {
        Self {
            slots: [ProcessSlot::EMPTY; MAX_USER_PROCESSES],
            address_spaces: AddressSpaceTable::new(),
        }
    }

    pub fn start(&mut self, image: &[u8]) -> Result<ProcessHandle, ProcessError> {
        self.start_plan(ElfLoadPlan::parse(image)?)
    }

    pub fn start_plan(&mut self, plan: ElfLoadPlan) -> Result<ProcessHandle, ProcessError> {
        let entry = plan.entry() as u64;
        let Some(slot) =
            self.slots.iter_mut().enumerate().find_map(|(slot, process)| {
                (process.state == ProcessState::Vacant).then_some(slot)
            })
        else {
            return Err(ProcessError::Capacity);
        };
        let address_space = self.address_spaces.reserve()?;
        let process = &mut self.slots[slot];
        process.state = ProcessState::Starting;
        process.entry = entry;
        process.address_space = Some(address_space);
        process.state = ProcessState::Running;
        Ok(ProcessHandle { slot: slot as u8, generation: process.generation })
    }

    pub fn state(&self, handle: ProcessHandle) -> Option<ProcessState> {
        let process = self.slots.get(handle.slot as usize)?;
        (process.generation == handle.generation).then_some(process.state)
    }

    pub fn address_space(&self, handle: ProcessHandle) -> Option<AddressSpaceHandle> {
        let process = self.slots.get(handle.slot as usize)?;
        (process.generation == handle.generation).then_some(process.address_space).flatten()
    }

    pub fn bind_address_space_root(
        &mut self,
        handle: ProcessHandle,
        root: AddressSpaceRoot,
    ) -> Result<(), ProcessError> {
        let process = self.current_mut(handle)?;
        if process.state != ProcessState::Running {
            return Err(ProcessError::NotRunning);
        }
        let Some(address_space) = process.address_space else {
            return Err(ProcessError::AddressSpace);
        };
        self.address_spaces.bind_root(address_space, root)
    }

    pub fn address_space_root(&self, handle: ProcessHandle) -> Option<AddressSpaceRoot> {
        let address_space = self.address_space(handle)?;
        self.address_spaces.root(address_space)
    }

    /// Produce launch metadata only after the process has a bound root.
    pub fn user_launch(
        &self,
        handle: ProcessHandle,
        entry: usize,
        stack_top: usize,
    ) -> Result<UserLaunch, ProcessError> {
        let process = self.slots.get(handle.slot as usize).ok_or(ProcessError::InvalidHandle)?;
        if process.generation != handle.generation {
            return Err(ProcessError::InvalidHandle);
        }
        if process.state != ProcessState::Running {
            return Err(ProcessError::NotRunning);
        }
        let root = self.address_space_root(handle).ok_or(ProcessError::AddressSpace)?;
        UserLaunch::new(entry, stack_top, root).ok_or(ProcessError::AddressSpace)
    }

    pub fn map(
        &mut self,
        handle: ProcessHandle,
        mapping: VirtualMapping,
    ) -> Result<(), ProcessError> {
        let process = self.current_mut(handle)?;
        if process.state != ProcessState::Running {
            return Err(ProcessError::NotRunning);
        }
        let Some(address_space) = process.address_space else {
            return Err(ProcessError::AddressSpace);
        };
        self.address_spaces.map(address_space, mapping)
    }

    pub fn mapping(&self, handle: ProcessHandle, index: usize) -> Option<VirtualMapping> {
        let address_space = self.address_space(handle)?;
        self.address_spaces.mapping(address_space, index)
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
        let address_space = {
            let process = self.current_mut(handle)?;
            if !matches!(process.state, ProcessState::Exited(_) | ProcessState::Faulted(_)) {
                return Err(ProcessError::NotRunning);
            }
            process.address_space
        };
        if let Some(address_space) = address_space {
            self.address_spaces.release(address_space)?;
        }
        let process = self.current_mut(handle)?;
        if !matches!(process.state, ProcessState::Exited(_) | ProcessState::Faulted(_)) {
            return Err(ProcessError::NotRunning);
        }
        process.state = ProcessState::Vacant;
        process.entry = 0;

        process.address_space = None;
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
    Ok(ElfLoadPlan::parse(image)?.entry() as u64)
}

pub trait ImageReader {
    fn len(&self) -> usize;
    fn read(&mut self, offset: usize, output: &mut [u8]) -> Result<usize, ProcessError>;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

struct SliceImageReader<'a> {
    image: &'a [u8],
}

impl ImageReader for SliceImageReader<'_> {
    fn len(&self) -> usize {
        self.image.len()
    }

    fn read(&mut self, offset: usize, output: &mut [u8]) -> Result<usize, ProcessError> {
        let end = offset.checked_add(output.len()).ok_or(ProcessError::ReadFailure)?;
        let Some(bytes) = self.image.get(offset..end) else {
            return Err(ProcessError::ReadFailure);
        };
        output.copy_from_slice(bytes);
        Ok(output.len())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadSegment {
    file_offset: usize,
    virtual_address: usize,
    file_size: usize,
    memory_size: usize,
    flags: MappingFlags,
}

impl LoadSegment {
    pub const fn file_offset(self) -> usize {
        self.file_offset
    }

    pub const fn virtual_address(self) -> usize {
        self.virtual_address
    }

    pub const fn file_size(self) -> usize {
        self.file_size
    }

    pub const fn memory_size(self) -> usize {
        self.memory_size
    }

    pub const fn flags(self) -> MappingFlags {
        self.flags
    }

    pub fn file_bytes(self, image: &[u8]) -> Option<&[u8]> {
        image.get(self.file_offset..self.file_offset + self.file_size)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ElfLoadPlan {
    entry: usize,
    segments: [Option<LoadSegment>; MAX_PROGRAM_HEADERS],
    count: usize,
    memory_bytes: usize,
}

impl ElfLoadPlan {
    pub fn parse(image: &[u8]) -> Result<Self, ProcessError> {
        let mut reader = SliceImageReader { image };
        Self::parse_reader(&mut reader)
    }

    pub fn parse_reader<R: ImageReader>(reader: &mut R) -> Result<Self, ProcessError> {
        if reader.len() < 64 || reader.len() > MAX_IMAGE_BYTES {
            return Err(ProcessError::InvalidImage);
        }
        let mut header = [0; 64];
        read_exact(reader, 0, &mut header)?;
        if &header[..4] != ELF_MAGIC
            || header[4] != 2
            || header[5] != 1
            || read_u16_at(&header, 18) != Some(0x3e)
            || read_u16_at(&header, 16) != Some(2)
        {
            return Err(ProcessError::InvalidImage);
        }
        let entry = usize_from_u64(read_u64_at(&header, 24).ok_or(ProcessError::InvalidImage)?)?;
        let ph_offset =
            usize_from_u64(read_u64_at(&header, 32).ok_or(ProcessError::InvalidImage)?)?;
        let ph_entry_size = read_u16_at(&header, 54).ok_or(ProcessError::InvalidImage)? as usize;
        let ph_count = read_u16_at(&header, 56).ok_or(ProcessError::InvalidImage)? as usize;
        let ph_bytes = ph_entry_size.checked_mul(ph_count).ok_or(ProcessError::InvalidImage)?;
        if entry == 0
            || ph_entry_size < 56
            || ph_count == 0
            || ph_count > MAX_PROGRAM_HEADERS
            || ph_offset.checked_add(ph_bytes).is_none()
            || ph_offset + ph_bytes > reader.len()
        {
            return Err(ProcessError::InvalidImage);
        }
        let mut plan =
            Self { entry, segments: [None; MAX_PROGRAM_HEADERS], count: 0, memory_bytes: 0 };
        let mut entry_executable = false;
        let mut program_headers = [0; MAX_PROGRAM_HEADERS * 56];
        if ph_entry_size == 56 {
            read_exact(reader, ph_offset, &mut program_headers[..ph_bytes])?;
        }
        for index in 0..ph_count {
            let offset = ph_offset + index * ph_entry_size;
            let mut program_header = [0; 56];
            if ph_entry_size == 56 {
                let start = index * 56;
                program_header.copy_from_slice(&program_headers[start..start + 56]);
            } else {
                read_exact(reader, offset, &mut program_header)?;
            }
            let kind = read_u32_at(&program_header, 0).ok_or(ProcessError::InvalidImage)?;
            if kind != 1 {
                continue;
            }
            let flags = read_u32_at(&program_header, 4).ok_or(ProcessError::InvalidImage)?;
            let file_offset =
                usize_from_u64(read_u64_at(&program_header, 8).ok_or(ProcessError::InvalidImage)?)?;
            let virtual_address = usize_from_u64(
                read_u64_at(&program_header, 16).ok_or(ProcessError::InvalidImage)?,
            )?;
            let file_size = usize_from_u64(
                read_u64_at(&program_header, 32).ok_or(ProcessError::InvalidImage)?,
            )?;
            let memory_size = usize_from_u64(
                read_u64_at(&program_header, 40).ok_or(ProcessError::InvalidImage)?,
            )?;
            let Some(file_end) = file_offset.checked_add(file_size) else {
                return Err(ProcessError::InvalidImage);
            };
            let Some(virtual_end) = virtual_address.checked_add(memory_size) else {
                return Err(ProcessError::InvalidImage);
            };
            if memory_size == 0
                || file_size > memory_size
                || file_end > reader.len()
                || virtual_address < 0x1000
                || virtual_end > 0x0000_8000_0000_0000
                || memory_size > MAX_IMAGE_BYTES
                || flags & 0x1 != 0 && flags & 0x2 != 0
                || plan.memory_bytes.checked_add(memory_size).is_none()
                || plan.memory_bytes + memory_size > MAX_IMAGE_BYTES
            {
                return Err(ProcessError::InvalidImage);
            }
            let segment = LoadSegment {
                file_offset,
                virtual_address,
                file_size,
                memory_size,
                flags: MappingFlags {
                    user: true,
                    writable: flags & 0x2 != 0,
                    executable: flags & 0x1 != 0,
                },
            };
            if plan.count == MAX_PROGRAM_HEADERS {
                return Err(ProcessError::InvalidImage);
            }
            entry_executable |=
                segment.flags.executable && segment.virtual_address <= entry && entry < virtual_end;
            plan.segments[plan.count] = Some(segment);
            plan.count += 1;
            plan.memory_bytes += memory_size;
        }
        if plan.count == 0 || !entry_executable {
            return Err(ProcessError::InvalidImage);
        }
        Ok(plan)
    }

    pub const fn entry(self) -> usize {
        self.entry
    }

    pub const fn segment_count(self) -> usize {
        self.count
    }

    pub const fn memory_bytes(self) -> usize {
        self.memory_bytes
    }

    pub const fn segment(self, index: usize) -> Option<LoadSegment> {
        if index < self.count { self.segments[index] } else { None }
    }
}

fn read_exact<R: ImageReader>(
    reader: &mut R,
    offset: usize,
    output: &mut [u8],
) -> Result<(), ProcessError> {
    let end = offset.checked_add(output.len()).ok_or(ProcessError::InvalidImage)?;
    if end > reader.len() {
        return Err(ProcessError::InvalidImage);
    }
    let amount = reader.read(offset, output)?;
    (amount == output.len()).then_some(()).ok_or(ProcessError::ReadFailure)
}

fn usize_from_u64(value: u64) -> Result<usize, ProcessError> {
    (value <= usize::MAX as u64).then_some(value as usize).ok_or(ProcessError::InvalidImage)
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
        image[68..72].copy_from_slice(&5u32.to_le_bytes());
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
        let process = table.start(&image).unwrap();
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

    #[test]
    fn elf_plan_exposes_bounded_segment_metadata() {
        let image = image();
        let plan = ElfLoadPlan::parse(&image).unwrap();
        assert_eq!(plan.entry(), 0x1000);
        assert_eq!(plan.segment_count(), 1);
        assert_eq!(plan.memory_bytes(), 0x1000);
        let segment = plan.segment(0).unwrap();
        assert_eq!(segment.virtual_address(), 0x1000);
        assert_eq!(segment.memory_size(), 0x1000);
        assert_eq!(segment.file_bytes(&image).unwrap().len(), 1);
        assert_eq!(plan.segment(1), None);
    }

    #[test]
    fn elf_entry_must_land_in_executable_segment() {
        let mut image = image();
        image[68..72].copy_from_slice(&4u32.to_le_bytes());
        assert_eq!(ElfLoadPlan::parse(&image), Err(ProcessError::InvalidImage));
    }

    #[test]
    fn address_space_identity_is_bound_once_and_released_with_process() {
        let image = image();
        let mut table = ProcessTable::new();
        let process = table.start(&image).unwrap();
        let address_space = table.address_space(process).unwrap();
        let root = AddressSpaceRoot::new(0x20_000).unwrap();
        assert_eq!(table.address_space_root(process), None);
        assert!(table.bind_address_space_root(process, root).is_ok());
        assert_eq!(table.address_space_root(process), Some(root));
        assert_eq!(
            table.bind_address_space_root(process, AddressSpaceRoot::new(0x30_000).unwrap()),
            Err(ProcessError::AddressSpace)
        );
        assert!(table.fault(process, 14).is_ok());
        assert!(table.reclaim(process).is_ok());
        assert_eq!(table.state(process), None);

        let replacement = table.start(&image).unwrap();
        assert_eq!(replacement.slot(), process.slot());
        assert_ne!(replacement.generation(), process.generation());
        assert_ne!(
            table.address_space(replacement).unwrap().generation(),
            address_space.generation()
        );
        assert!(table.bind_address_space_root(process, root).is_err());
    }

    #[test]
    fn address_space_roots_are_page_aligned_and_nonzero() {
        assert_eq!(AddressSpaceRoot::new(0), None);
        assert_eq!(AddressSpaceRoot::new(0x123), None);
        assert_eq!(AddressSpaceRoot::new(0x12_000).unwrap().raw(), 0x12_000);
    }

    #[test]
    fn user_launch_requires_a_bound_root_and_preserves_register_metadata() {
        let image = image();
        let mut table = ProcessTable::new();
        let process = table.start(&image).unwrap();
        assert_eq!(table.user_launch(process, 0x1000, 0x8000), Err(ProcessError::AddressSpace));
        let root = AddressSpaceRoot::new(0x20_000).unwrap();
        table.bind_address_space_root(process, root).unwrap();
        let launch = table.user_launch(process, 0x1000, 0x8000).unwrap();
        assert_eq!(launch.entry(), 0x1000);
        assert_eq!(launch.stack_top(), 0x8000);
        assert_eq!(launch.address_space_root(), root);
    }

    #[test]
    fn mappings_require_a_root_and_reject_overlap_or_wx_pages() {
        let image = image();
        let mut table = ProcessTable::new();
        let process = table.start(&image).unwrap();
        let code = VirtualMapping::new(0x40_000, 0x80_000, 2, MappingFlags::CODE).unwrap();
        let data = VirtualMapping::new(0x41_000, 0x90_000, 1, MappingFlags::DATA).unwrap();
        assert_eq!(table.map(process, code), Err(ProcessError::AddressSpace));
        assert!(
            table
                .bind_address_space_root(process, AddressSpaceRoot::new(0x20_000).unwrap())
                .is_ok()
        );
        assert!(table.map(process, code).is_ok());
        assert_eq!(table.map(process, data), Err(ProcessError::AddressSpace));
        assert_eq!(table.mapping(process, 0), Some(code));
        assert_eq!(
            VirtualMapping::new(
                0x50_000,
                0xa0_000,
                1,
                MappingFlags { user: true, writable: true, executable: true },
            ),
            None
        );
        assert!(VirtualMapping::new(0x51_000, 0xb0_000, 1, MappingFlags::DATA).is_some());
        assert!(
            VirtualMapping::new_device(
                logos_abi::DISPLAY_FRAMEBUFFER_BASE,
                0x100_000,
                logos_abi::MAX_FRAMEBUFFER_BYTES / 0x1000,
                MappingFlags::DATA,
            )
            .is_some()
        );
        assert!(
            VirtualMapping::new_device(
                logos_abi::DISPLAY_FRAMEBUFFER_BASE,
                0x100_000,
                logos_abi::MAX_FRAMEBUFFER_BYTES / 0x1000 + 1,
                MappingFlags::DATA,
            )
            .is_none()
        );
    }
}
