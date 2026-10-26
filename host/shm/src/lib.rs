//! Shared-memory windows for the hosted Core (ADR-0096).
//!
//! On bare metal, Core maps fixed pages into each service's address space. On a
//! host OS, Core creates one shared mapping per window and the service process
//! maps it at the same fixed ABI address before entering `_start`, so service
//! code that dereferences `logos_abi::*_BASE` constants is unchanged.

use std::io;

/// One fixed-address window of the service ABI.
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub name: &'static str,
    pub base: usize,
    pub bytes: usize,
}

const PAGE: usize = 4096;

/// Every window the hosted Core can hand to a service. The child maps each one
/// whose `LOGOS_HOST_MAP_<NAME>` environment variable is set.
pub const REGIONS: &[Region] = &[
    Region { name: "bootstrap", base: logos_abi::SERVICE_BOOTSTRAP_BASE, bytes: PAGE },
    // Staging, storage data/cache, capability pages, network config and packet pages.
    Region { name: "ipc", base: logos_abi::SERVICE_IPC_BASE, bytes: 0x70_000 },
    Region {
        name: "display",
        base: logos_abi::DISPLAY_FRAMEBUFFER_BASE,
        bytes: logos_abi::DEFAULT_SCREEN_WIDTH * logos_abi::DEFAULT_SCREEN_HEIGHT * 4,
    },
    Region { name: "display_config", base: logos_abi::DISPLAY_CONFIG_BASE, bytes: 2 * PAGE },
    // Keyboard ring page plus the pointer ring page 64 KiB above it.
    Region {
        name: "input",
        base: logos_abi::INPUT_KEYBOARD_RING_BASE,
        bytes: logos_abi::INPUT_POINTER_RING_BASE - logos_abi::INPUT_KEYBOARD_RING_BASE + PAGE,
    },
];

pub fn region(name: &str) -> Option<Region> {
    REGIONS.iter().copied().find(|region| region.name == name)
}

pub fn env_name(region: &str) -> String {
    format!("LOGOS_HOST_MAP_{}", region.to_ascii_uppercase())
}

/// A mapped shared window. `token()` is what a child needs to open it.
pub struct Mapping {
    ptr: *mut u8,
    bytes: usize,
    token: String,
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

// The mapping is shared memory by design; callers synchronize through the ABI's atomics.
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Mapping {
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

#[cfg(windows)]
mod imp {
    use super::Mapping;
    use std::io;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Memory::{
        CreateFileMappingW, FILE_MAP_ALL_ACCESS, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFileEx,
        OpenFileMappingW, PAGE_READWRITE, UnmapViewOfFile,
    };

    fn wide(name: &str) -> Vec<u16> {
        name.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn view(handle: HANDLE, bytes: usize, base: usize) -> io::Result<*mut u8> {
        let view =
            unsafe { MapViewOfFileEx(handle, FILE_MAP_ALL_ACCESS, 0, 0, bytes, base as *const _) };
        if view.Value.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(view.Value as *mut u8)
    }

    pub fn create(name: &str, bytes: usize) -> io::Result<Mapping> {
        let wide_name = wide(name);
        let handle = unsafe {
            CreateFileMappingW(
                -1isize as HANDLE,
                std::ptr::null(),
                PAGE_READWRITE,
                (bytes as u64 >> 32) as u32,
                bytes as u32,
                wide_name.as_ptr(),
            )
        };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let ptr = view(handle, bytes, 0)?;
        Ok(Mapping { ptr, bytes, token: name.to_owned(), handle })
    }

    pub fn open_at(token: &str, base: usize, bytes: usize) -> io::Result<Mapping> {
        let wide_name = wide(token);
        let handle = unsafe { OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, wide_name.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let ptr = view(handle, bytes, base)?;
        Ok(Mapping { ptr, bytes, token: token.to_owned(), handle })
    }

    impl Drop for Mapping {
        fn drop(&mut self) {
            unsafe {
                UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: self.ptr as *mut _ });
                CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(unix)]
mod imp {
    use super::Mapping;
    use std::io;

    fn map(fd: i32, bytes: usize, base: usize) -> io::Result<*mut u8> {
        let flags = libc::MAP_SHARED | if base != 0 { libc::MAP_FIXED_NOREPLACE } else { 0 };
        let ptr = unsafe {
            libc::mmap(base as *mut _, bytes, libc::PROT_READ | libc::PROT_WRITE, flags, fd, 0)
        };
        if ptr == libc::MAP_FAILED || (base != 0 && ptr as usize != base) {
            return Err(io::Error::last_os_error());
        }
        Ok(ptr as *mut u8)
    }

    pub fn create(name: &str, bytes: usize) -> io::Result<Mapping> {
        let c_name = std::ffi::CString::new(name).map_err(io::Error::other)?;
        // No MFD_CLOEXEC: the descriptor is inherited by the service child on purpose.
        let fd = unsafe { libc::memfd_create(c_name.as_ptr(), 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::ftruncate(fd, bytes as libc::off_t) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let ptr = map(fd, bytes, 0)?;
        Ok(Mapping { ptr, bytes, token: fd.to_string() })
    }

    pub fn open_at(token: &str, base: usize, bytes: usize) -> io::Result<Mapping> {
        let fd: i32 = token.parse().map_err(|_| io::Error::other("bad mapping token"))?;
        let ptr = map(fd, bytes, base)?;
        Ok(Mapping { ptr, bytes, token: token.to_owned() })
    }

    impl Drop for Mapping {
        fn drop(&mut self) {
            unsafe {
                libc::munmap(self.ptr as *mut _, self.bytes);
            }
        }
    }
}

/// Core side: create a shared window and map it anywhere in this process.
pub fn create(name: &str, bytes: usize) -> io::Result<Mapping> {
    imp::create(name, bytes)
}

/// Service side: map an existing window at its fixed ABI address.
pub fn open_at(token: &str, base: usize, bytes: usize) -> io::Result<Mapping> {
    imp::open_at(token, base, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_are_aligned_and_disjoint() {
        for (index, region) in REGIONS.iter().enumerate() {
            assert_eq!(region.base % 0x1_0000, 0, "{} must be 64 KiB aligned", region.name);
            assert!(region.bytes > 0);
            for other in &REGIONS[index + 1..] {
                let disjoint = region.base + region.bytes <= other.base
                    || other.base + other.bytes <= region.base;
                assert!(disjoint, "{} overlaps {}", region.name, other.name);
            }
        }
        let ipc = region("ipc").unwrap();
        assert!(logos_abi::NETWORK_PACKET_BASE + 32 * PAGE <= ipc.base + ipc.bytes);
        assert!(logos_abi::IPC_STAGING_BASE + PAGE <= ipc.base + ipc.bytes);
    }

    #[test]
    fn shared_window_round_trips_between_mappings() {
        let name = format!("Local\\logos-shm-test-{}", std::process::id());
        let writer = create(&name, PAGE).unwrap();
        let reader = open_at(writer.token(), 0, PAGE).unwrap();
        unsafe {
            writer.as_ptr().write(0xA5);
            assert_eq!(reader.as_ptr().read(), 0xA5);
        }
    }
}
