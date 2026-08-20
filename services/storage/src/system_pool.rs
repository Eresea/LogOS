//! Validated allocation classes for the post-v4 storage format.
//!
//! This module only describes the layout. v4 still uses its existing arena
//! boundaries; a future format root will persist these values and hand the
//! classes to its allocator.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoragePoolError {
    InvalidVolumeRange,
    InvalidPackageRange,
    InvalidSystemReserve,
    NoUserSpace,
}

/// Disjoint block ranges for system metadata, user content, and packages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoragePoolLayout {
    system_start: u64,
    system_end: u64,
    user_start: u64,
    user_end: u64,
    package_start: u64,
    package_end: u64,
}

impl StoragePoolLayout {
    /// Build a layout from the data arena and the format's package boundary.
    ///
    /// The system pool is a prefix of the data arena, the user pool follows
    /// it, and the package pool begins at `package_start`.
    pub fn new(
        data_start: u64,
        data_end: u64,
        package_start: u64,
        system_blocks: u64,
    ) -> Result<Self, StoragePoolError> {
        if data_start >= data_end {
            return Err(StoragePoolError::InvalidVolumeRange);
        }
        if package_start <= data_start || package_start > data_end {
            return Err(StoragePoolError::InvalidPackageRange);
        }
        if system_blocks == 0 {
            return Err(StoragePoolError::InvalidSystemReserve);
        }
        let system_end =
            data_start.checked_add(system_blocks).ok_or(StoragePoolError::InvalidSystemReserve)?;
        if system_end > package_start {
            return Err(StoragePoolError::InvalidSystemReserve);
        }
        if system_end == package_start {
            return Err(StoragePoolError::NoUserSpace);
        }

        Ok(Self {
            system_start: data_start,
            system_end,
            user_start: system_end,
            user_end: package_start,
            package_start,
            package_end: data_end,
        })
    }

    pub const fn system_arena(self) -> (u64, u64) {
        (self.system_start, self.system_end)
    }

    pub const fn user_arena(self) -> (u64, u64) {
        (self.user_start, self.user_end)
    }

    pub const fn package_arena(self) -> (u64, u64) {
        (self.package_start, self.package_end)
    }

    pub const fn system_blocks(self) -> u64 {
        self.system_end - self.system_start
    }

    pub const fn user_blocks(self) -> u64 {
        self.user_end - self.user_start
    }

    pub const fn package_blocks(self) -> u64 {
        self.package_end - self.package_start
    }

    pub const fn contains_system(self, block: u64) -> bool {
        block >= self.system_start && block < self.system_end
    }

    pub const fn contains_user(self, block: u64) -> bool {
        block >= self.user_start && block < self.user_end
    }

    pub const fn contains_package(self, block: u64) -> bool {
        block >= self.package_start && block < self.package_end
    }
}

#[cfg(test)]
mod tests {
    use super::{StoragePoolError, StoragePoolLayout};

    #[test]
    fn ranges_are_disjoint_and_contiguous() {
        let layout = StoragePoolLayout::new(4, 64, 20, 4).unwrap();

        assert_eq!(layout.system_arena(), (4, 8));
        assert_eq!(layout.user_arena(), (8, 20));
        assert_eq!(layout.package_arena(), (20, 64));
        assert_eq!(layout.system_blocks(), 4);
        assert_eq!(layout.user_blocks(), 12);
        assert_eq!(layout.package_blocks(), 44);
        assert!(layout.contains_system(4));
        assert!(!layout.contains_system(8));
        assert!(layout.contains_user(8));
        assert!(!layout.contains_user(20));
        assert!(layout.contains_package(20));
        assert!(!layout.contains_package(64));
    }

    #[test]
    fn invalid_layouts_fail_closed() {
        assert_eq!(StoragePoolLayout::new(8, 8, 9, 1), Err(StoragePoolError::InvalidVolumeRange));
        assert_eq!(StoragePoolLayout::new(4, 64, 4, 1), Err(StoragePoolError::InvalidPackageRange));
        assert_eq!(
            StoragePoolLayout::new(4, 64, 20, 0),
            Err(StoragePoolError::InvalidSystemReserve)
        );
        assert_eq!(
            StoragePoolLayout::new(4, 64, 20, 17),
            Err(StoragePoolError::InvalidSystemReserve)
        );
        assert_eq!(StoragePoolLayout::new(4, 64, 8, 4), Err(StoragePoolError::NoUserSpace));
    }

    #[test]
    fn package_boundary_may_reach_volume_end_but_user_space_may_not_be_empty() {
        let layout = StoragePoolLayout::new(4, 20, 20, 4).unwrap();
        assert_eq!(layout.package_arena(), (20, 20));
        assert_eq!(layout.package_blocks(), 0);
        assert_eq!(layout.user_arena(), (8, 20));
    }
}
