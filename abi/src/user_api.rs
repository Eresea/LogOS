use super::MAX_IPC_BYTES;

pub const USER_ABI_VERSION: u16 = 1;
pub const USER_MAX_USER_NAME_BYTES: usize = 32;
pub const USER_MAX_ROLE_NAME_BYTES: usize = 32;
pub const USER_MAX_PASSWORD_BYTES: usize = 128;
pub const USER_ARGON2_SALT_BYTES: usize = 16;
pub const USER_ARGON2_OUTPUT_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct UserId {
    pub value: u64,
    pub generation: u32,
}

impl UserId {
    pub const EMPTY: Self = Self { value: 0, generation: 0 };

    pub const fn new(value: u64, generation: u32) -> Option<Self> {
        if value == 0 || generation == 0 { None } else { Some(Self { value, generation }) }
    }

    pub const fn is_valid(self) -> bool {
        self.value != 0 && self.generation != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct RoleId {
    pub value: u64,
    pub generation: u32,
}

impl RoleId {
    pub const EMPTY: Self = Self { value: 0, generation: 0 };

    pub const fn new(value: u64, generation: u32) -> Option<Self> {
        if value == 0 || generation == 0 { None } else { Some(Self { value, generation }) }
    }

    pub const fn is_valid(self) -> bool {
        self.value != 0 && self.generation != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct SessionHandle {
    pub slot: u32,
    pub generation: u32,
}

impl SessionHandle {
    pub const EMPTY: Self = Self { slot: u32::MAX, generation: 0 };

    pub const fn new(slot: u32, generation: u32) -> Option<Self> {
        if generation == 0 { None } else { Some(Self { slot, generation }) }
    }

    pub const fn is_valid(self) -> bool {
        self.slot != u32::MAX && self.generation != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct CapabilityHandle {
    pub slot: u32,
    pub generation: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct UserAdminCapability {
    pub generation: u32,
    pub lineage: u64,
}

impl UserAdminCapability {
    pub const EMPTY: Self = Self { generation: 0, lineage: 0 };

    pub const fn is_valid(self) -> bool {
        self.generation != 0 && self.lineage != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct NamespaceCapability {
    pub root: NamespaceRoot,
    pub rights: NamespaceRights,
    pub lineage: u64,
}

impl NamespaceCapability {
    pub const EMPTY: Self =
        Self { root: NamespaceRoot::EMPTY, rights: NamespaceRights::NONE, lineage: 0 };

    pub const fn is_valid(self) -> bool {
        self.root.is_valid() && self.rights.is_valid() && self.lineage != 0
    }
}

impl CapabilityHandle {
    pub const EMPTY: Self = Self { slot: u32::MAX, generation: 0 };

    pub const fn new(slot: u32, generation: u32) -> Option<Self> {
        if generation == 0 { None } else { Some(Self { slot, generation }) }
    }

    pub const fn is_valid(self) -> bool {
        self.slot != u32::MAX && self.generation != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct NamespaceRoot {
    pub object: u64,
    pub generation: u32,
}

impl NamespaceRoot {
    pub const EMPTY: Self = Self { object: 0, generation: 0 };

    pub const fn new(object: u64, generation: u32) -> Option<Self> {
        if object == 0 || generation == 0 { None } else { Some(Self { object, generation }) }
    }

    pub const fn is_valid(self) -> bool {
        self.object != 0 && self.generation != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct NamespaceRights(pub u8);

impl NamespaceRights {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const DERIVE: Self = Self(1 << 2);
    pub const VALID: u8 = Self::READ.0 | Self::WRITE.0 | Self::DERIVE.0;

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_valid(self) -> bool {
        self.0 != 0 && self.0 & !Self::VALID == 0
    }

    pub const fn attenuate(self, requested: Self) -> Option<Self> {
        if self.contains(requested) { Some(requested) } else { None }
    }
}

impl core::ops::BitOr for NamespaceRights {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum UserOperation {
    Claim = 1,
    Create = 2,
    Rename = 3,
    SetPassword = 4,
    Login = 5,
    Logout = 6,
    RevokeSession = 7,
    Derive = 8,
    RevokeCapability = 9,
    CreateRole = 10,
    AssignRole = 11,
}

impl UserOperation {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Claim),
            2 => Some(Self::Create),
            3 => Some(Self::Rename),
            4 => Some(Self::SetPassword),
            5 => Some(Self::Login),
            6 => Some(Self::Logout),
            7 => Some(Self::RevokeSession),
            8 => Some(Self::Derive),
            9 => Some(Self::RevokeCapability),
            10 => Some(Self::CreateRole),
            11 => Some(Self::AssignRole),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum UserStatus {
    Ok = 0,
    Invalid = 1,
    Unclaimed = 2,
    AlreadyClaimed = 3,
    NotFound = 4,
    Unauthorized = 5,
    BadCredentials = 6,
    Stale = 7,
    Revoked = 8,
    Capacity = 9,
    Corrupt = 10,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct UserRequest {
    pub operation: UserOperation,
    pub request_id: u32,
    pub user: UserId,
    pub role: RoleId,
    pub session: SessionHandle,
    pub capability: CapabilityHandle,
    pub root: NamespaceRoot,
    pub rights: NamespaceRights,
    pub name_len: u8,
    pub name: [u8; USER_MAX_USER_NAME_BYTES],
    pub password_len: u8,
    pub password: [u8; USER_MAX_PASSWORD_BYTES],
}

impl UserRequest {
    pub const fn new(operation: UserOperation, request_id: u32) -> Self {
        Self {
            operation,
            request_id,
            user: UserId::EMPTY,
            role: RoleId::EMPTY,
            session: SessionHandle::EMPTY,
            capability: CapabilityHandle::EMPTY,
            root: NamespaceRoot::EMPTY,
            rights: NamespaceRights::NONE,
            name_len: 0,
            name: [0; USER_MAX_USER_NAME_BYTES],
            password_len: 0,
            password: [0; USER_MAX_PASSWORD_BYTES],
        }
    }

    pub fn set_name(&mut self, name: &[u8]) -> bool {
        if name.is_empty() || name.len() > self.name.len() {
            return false;
        }
        self.name = [0; USER_MAX_USER_NAME_BYTES];
        self.name[..name.len()].copy_from_slice(name);
        self.name_len = name.len() as u8;
        true
    }

    pub fn set_password(&mut self, password: &[u8]) -> bool {
        if password.is_empty() || password.len() > self.password.len() {
            return false;
        }
        self.password = [0; USER_MAX_PASSWORD_BYTES];
        self.password[..password.len()].copy_from_slice(password);
        self.password_len = password.len() as u8;
        true
    }

    pub const fn is_valid(self) -> bool {
        self.request_id != 0
            && self.name_len as usize <= self.name.len()
            && self.password_len as usize <= self.password.len()
            && self.rights.0 & !NamespaceRights::VALID == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct UserResponse {
    pub operation: UserOperation,
    pub status: UserStatus,
    pub request_id: u32,
    pub user: UserId,
    pub role: RoleId,
    pub session: SessionHandle,
    pub capability: CapabilityHandle,
    pub root: NamespaceRoot,
    pub rights: NamespaceRights,
}

impl UserResponse {
    pub const fn new(request: UserRequest, status: UserStatus) -> Self {
        Self {
            operation: request.operation,
            status,
            request_id: request.request_id,
            user: UserId::EMPTY,
            role: RoleId::EMPTY,
            session: SessionHandle::EMPTY,
            capability: CapabilityHandle::EMPTY,
            root: NamespaceRoot::EMPTY,
            rights: NamespaceRights::NONE,
        }
    }

    pub const fn is_valid_for(self, request: UserRequest) -> bool {
        self.operation as u8 == request.operation as u8 && self.request_id == request.request_id
    }
}

const _: () = assert!(core::mem::size_of::<UserRequest>() <= MAX_IPC_BYTES);
const _: () = assert!(core::mem::size_of::<UserResponse>() <= MAX_IPC_BYTES);
