#![no_std]

#[cfg(test)]
extern crate std;

#[cfg(all(feature = "password-kdf", target_os = "none"))]
use argon2::Block;
#[cfg(feature = "password-kdf")]
use argon2::{Algorithm, Argon2, Params, Version};
use logos_abi::{
    NamespaceCapabilityHandle, NamespaceRights, NamespaceRoot, RoleId, SessionHandle,
    USER_ARGON2_OUTPUT_BYTES, USER_ARGON2_SALT_BYTES, USER_MAX_PASSWORD_BYTES,
    USER_MAX_ROLE_NAME_BYTES, USER_MAX_USER_NAME_BYTES, UserId, UserOperation, UserRequest,
    UserResponse, UserStatus,
};
#[cfg(all(feature = "password-kdf", target_os = "none"))]
use logos_abi::{USER_KDF_WORKSPACE_BASE, USER_KDF_WORKSPACE_BYTES};

pub const MAX_USERS: usize = 32;
pub const MAX_ROLES: usize = 16;
pub const MAX_SESSIONS: usize = 32;
pub const MAX_CAPABILITIES_PER_SESSION: usize = 16;
pub const MAX_ROLE_GRANTS: usize = 8;
pub const MAX_ROLE_TEMPLATES: usize = 8;
pub const USER_SNAPSHOT_BYTES: usize = 12 * 1024;
const SNAPSHOT_MAGIC: [u8; 8] = *b"LOGUSR01";
const SNAPSHOT_VERSION: u16 = 1;

#[cfg(feature = "password-kdf")]
const ARGON2_MEMORY_KIB: u32 = 1024;
#[cfg(feature = "password-kdf")]
const ARGON2_TIME_COST: u32 = 1;
#[cfg(feature = "password-kdf")]
const ARGON2_LANES: u32 = 1;
const ARGON2_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserError {
    InvalidName,
    InvalidPassword,
    AlreadyClaimed,
    NotClaimed,
    Capacity,
    NotFound,
    Unauthorized,
    BadCredentials,
    Stale,
    Revoked,
    Corrupt,
    Crypto,
    Entropy,
    Persistence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserName {
    bytes: [u8; USER_MAX_USER_NAME_BYTES],
    len: u8,
}

impl UserName {
    pub fn parse(input: &[u8]) -> Result<Self, UserError> {
        if input.is_empty() || input.len() > USER_MAX_USER_NAME_BYTES {
            return Err(UserError::InvalidName);
        }
        if input.iter().any(|byte| {
            !byte.is_ascii_lowercase()
                && !byte.is_ascii_digit()
                && *byte != b'-'
                && *byte != b'_'
                && *byte != b'.'
        }) {
            return Err(UserError::InvalidName);
        }
        let mut bytes = [0; USER_MAX_USER_NAME_BYTES];
        bytes[..input.len()].copy_from_slice(input);
        Ok(Self { bytes, len: input.len() as u8 })
    }

    pub const fn as_bytes(self) -> [u8; USER_MAX_USER_NAME_BYTES] {
        self.bytes
    }

    pub const fn len(self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    pub fn slice(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoleName {
    bytes: [u8; USER_MAX_ROLE_NAME_BYTES],
    len: u8,
}

impl RoleName {
    pub fn parse(input: &[u8]) -> Result<Self, UserError> {
        if input.is_empty() || input.len() > USER_MAX_ROLE_NAME_BYTES {
            return Err(UserError::InvalidName);
        }
        if input.iter().any(|byte| {
            !byte.is_ascii_lowercase()
                && !byte.is_ascii_digit()
                && *byte != b'-'
                && *byte != b'_'
                && *byte != b'.'
        }) {
            return Err(UserError::InvalidName);
        }
        let mut bytes = [0; USER_MAX_ROLE_NAME_BYTES];
        bytes[..input.len()].copy_from_slice(input);
        Ok(Self { bytes, len: input.len() as u8 })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PasswordVerifier {
    pub version: u8,
    pub salt: [u8; USER_ARGON2_SALT_BYTES],
    pub output: [u8; USER_ARGON2_OUTPUT_BYTES],
}

impl PasswordVerifier {
    #[allow(clippy::needless_return)]
    pub fn create(password: &[u8], salt: [u8; USER_ARGON2_SALT_BYTES]) -> Result<Self, UserError> {
        if password.is_empty() || password.len() > USER_MAX_PASSWORD_BYTES {
            return Err(UserError::InvalidPassword);
        }
        #[cfg(not(feature = "password-kdf"))]
        {
            let _ = salt;
            return Err(UserError::Crypto);
        }
        #[cfg(feature = "password-kdf")]
        {
            let params = Params::new(
                ARGON2_MEMORY_KIB,
                ARGON2_TIME_COST,
                ARGON2_LANES,
                Some(USER_ARGON2_OUTPUT_BYTES),
            )
            .map_err(|_| UserError::Crypto)?;
            let algorithm = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
            let mut output = [0; USER_ARGON2_OUTPUT_BYTES];
            hash_password_into(&algorithm, password, &salt, &mut output)?;
            Ok(Self { version: ARGON2_VERSION, salt, output })
        }
    }

    pub fn verify(self, password: &[u8]) -> Result<(), UserError> {
        if self.version != ARGON2_VERSION
            || password.is_empty()
            || password.len() > USER_MAX_PASSWORD_BYTES
        {
            return Err(UserError::BadCredentials);
        }
        let candidate = Self::create(password, self.salt)?;
        if constant_time_eq(&candidate.output, &self.output) {
            Ok(())
        } else {
            Err(UserError::BadCredentials)
        }
    }
}

#[cfg(all(feature = "password-kdf", target_os = "none"))]
fn hash_password_into(
    algorithm: &Argon2<'_>,
    password: &[u8],
    salt: &[u8],
    output: &mut [u8],
) -> Result<(), UserError> {
    let block_count = algorithm.params().block_count();
    if block_count > USER_KDF_WORKSPACE_BYTES / Block::SIZE {
        return Err(UserError::Crypto);
    }
    // Core maps this fixed, service-private workspace before User starts.
    let memory = unsafe {
        core::slice::from_raw_parts_mut(USER_KDF_WORKSPACE_BASE as *mut Block, block_count)
    };
    algorithm
        .hash_password_into_with_memory(password, salt, output, memory)
        .map_err(|_| UserError::Crypto)
}

#[cfg(all(feature = "password-kdf", not(target_os = "none")))]
fn hash_password_into(
    algorithm: &Argon2<'_>,
    password: &[u8],
    salt: &[u8],
    output: &mut [u8],
) -> Result<(), UserError> {
    algorithm.hash_password_into(password, salt, output).map_err(|_| UserError::Crypto)
}

pub trait EntropySource {
    fn fill(&mut self, output: &mut [u8]) -> Result<(), UserError>;
}

/// Storage-owned persistence boundary for the durable User catalog.
///
/// Implementations own the path, system-pool allocation, and atomic commit.
/// User only supplies and consumes the bounded canonical snapshot.
pub trait UserCatalogStore {
    fn load(&mut self, output: &mut [u8]) -> Result<usize, UserError>;
    fn save(&mut self, snapshot: &[u8]) -> Result<(), UserError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityTemplate {
    pub root: NamespaceRoot,
    pub rights: NamespaceRights,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RoleRecord {
    id: RoleId,
    name: RoleName,
    templates: [Option<CapabilityTemplate>; MAX_ROLE_TEMPLATES],
    template_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct UserRecord {
    id: UserId,
    name: UserName,
    verifier: PasswordVerifier,
    home: NamespaceRoot,
    roles: [RoleId; MAX_ROLE_GRANTS],
    role_count: usize,
    next_lineage: u64,
    admin: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CapabilityRecord {
    handle: NamespaceCapabilityHandle,
    root: NamespaceRoot,
    rights: NamespaceRights,
    lineage: u64,
    parent_lineage: u64,
    revoked: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SessionRecord {
    handle: SessionHandle,
    user: UserId,
    lineage: u64,
    revoked: bool,
    capabilities: [Option<CapabilityRecord>; MAX_CAPABILITIES_PER_SESSION],
    capability_generations: [u32; MAX_CAPABILITIES_PER_SESSION],
}

pub struct UserCatalog {
    claimed: bool,
    next_user: u64,
    next_role: u64,
    next_lineage: u64,
    users: [Option<UserRecord>; MAX_USERS],
    roles: [Option<RoleRecord>; MAX_ROLES],
    sessions: [Option<SessionRecord>; MAX_SESSIONS],
}

pub struct UserService<E> {
    catalog: UserCatalog,
    entropy: E,
}

impl<E: EntropySource> UserService<E> {
    pub const fn new(entropy: E) -> Self {
        Self { catalog: UserCatalog::new(), entropy }
    }

    pub const fn catalog(&self) -> &UserCatalog {
        &self.catalog
    }

    pub fn catalog_mut(&mut self) -> &mut UserCatalog {
        &mut self.catalog
    }

    pub fn handle(&mut self, mut request: UserRequest) -> UserResponse {
        if request.root == NamespaceRoot::EMPTY
            && matches!(request.operation, UserOperation::Claim | UserOperation::Create)
        {
            request.root = self.catalog.default_home_root();
        }
        let mut response = UserResponse::new(request, UserStatus::Invalid);
        let result = if request.is_valid() {
            match request.operation {
                UserOperation::Claim => self
                    .catalog
                    .claim(
                        &request.name[..request.name_len as usize],
                        &request.password[..request.password_len as usize],
                        request.root,
                        &mut self.entropy,
                    )
                    .map(|user| response.user = user),
                UserOperation::Create
                    if self.catalog.is_admin_session(request.session).unwrap_or(false) =>
                {
                    self.catalog
                        .create_user(
                            &request.name[..request.name_len as usize],
                            &request.password[..request.password_len as usize],
                            request.root,
                            &mut self.entropy,
                        )
                        .map(|user| response.user = user)
                }
                UserOperation::Rename
                    if self.catalog.is_admin_session(request.session).unwrap_or(false) =>
                {
                    self.catalog
                        .rename_user(request.user, &request.name[..request.name_len as usize])
                }
                UserOperation::SetPassword
                    if self.catalog.is_admin_session(request.session).unwrap_or(false) =>
                {
                    self.catalog.set_password(
                        request.user,
                        &request.password[..request.password_len as usize],
                        &mut self.entropy,
                    )
                }
                UserOperation::Create | UserOperation::Rename | UserOperation::SetPassword => {
                    Err(UserError::Unauthorized)
                }
                UserOperation::CreateRole
                    if self.catalog.is_admin_session(request.session).unwrap_or(false) =>
                {
                    self.catalog
                        .create_role(
                            &request.name[..request.name_len as usize],
                            CapabilityTemplate { root: request.root, rights: request.rights },
                        )
                        .map(|role| response.role = role)
                }
                UserOperation::AssignRole
                    if self.catalog.is_admin_session(request.session).unwrap_or(false) =>
                {
                    self.catalog.assign_role(request.user, request.role)
                }
                UserOperation::Login => self
                    .catalog
                    .login(
                        &request.name[..request.name_len as usize],
                        &request.password[..request.password_len as usize],
                    )
                    .map(|(user, session)| {
                        response.user = user;
                        response.session = session;
                    }),
                UserOperation::Logout | UserOperation::RevokeSession => {
                    self.catalog.logout(request.session)
                }
                UserOperation::Derive => self
                    .catalog
                    .derive(request.session, request.capability, request.root, request.rights)
                    .map(|capability| {
                        response.capability = capability;
                        response.root = request.root;
                        response.rights = request.rights;
                    }),
                UserOperation::RevokeCapability => {
                    self.catalog.revoke_capability(request.session, request.capability)
                }
                UserOperation::CreateRole | UserOperation::AssignRole => {
                    Err(UserError::Unauthorized)
                }
            }
        } else {
            Err(UserError::InvalidName)
        };
        request.password.fill(0);
        response.status = match result {
            Ok(()) => UserStatus::Ok,
            Err(error) => map_error(error),
        };
        if response.status == UserStatus::Ok && response.operation == UserOperation::Login {
            if let Ok((capability, root, rights)) = self.catalog.first_capability(response.session)
            {
                response.capability = capability;
                response.root = root;
                response.rights = rights;
            }
        }
        response
    }
}

fn map_error(error: UserError) -> UserStatus {
    match error {
        UserError::AlreadyClaimed => UserStatus::AlreadyClaimed,
        UserError::NotClaimed => UserStatus::Unclaimed,
        UserError::NotFound => UserStatus::NotFound,
        UserError::Unauthorized => UserStatus::Unauthorized,
        UserError::BadCredentials => UserStatus::BadCredentials,
        UserError::Stale => UserStatus::Stale,
        UserError::Revoked => UserStatus::Revoked,
        UserError::Capacity => UserStatus::Capacity,
        UserError::Corrupt => UserStatus::Corrupt,
        UserError::InvalidName
        | UserError::InvalidPassword
        | UserError::Crypto
        | UserError::Entropy
        | UserError::Persistence => UserStatus::Invalid,
    }
}

impl UserCatalog {
    pub const fn new() -> Self {
        Self {
            claimed: false,
            next_user: 1,
            next_role: 1,
            next_lineage: 1,
            users: [None; MAX_USERS],
            roles: [None; MAX_ROLES],
            sessions: [None; MAX_SESSIONS],
        }
    }

    pub const fn is_claimed(&self) -> bool {
        self.claimed
    }

    pub const fn default_home_root(&self) -> NamespaceRoot {
        NamespaceRoot::new(0x1000 + self.next_user, 1).unwrap()
    }

    pub fn claim<E: EntropySource>(
        &mut self,
        name: &[u8],
        password: &[u8],
        home: NamespaceRoot,
        entropy: &mut E,
    ) -> Result<UserId, UserError> {
        if self.claimed {
            return Err(UserError::AlreadyClaimed);
        }
        let user = self.create_record(name, password, home, entropy)?;
        self.user_mut(user)?.admin = true;
        let admin = self.ensure_builtin_role(
            b"system-admin",
            CapabilityTemplate {
                root: home,
                rights: NamespaceRights::READ | NamespaceRights::WRITE | NamespaceRights::DERIVE,
            },
        )?;
        let user_role = self.ensure_builtin_role(
            b"user",
            CapabilityTemplate {
                root: NamespaceRoot::EMPTY,
                rights: NamespaceRights::READ | NamespaceRights::WRITE,
            },
        )?;
        self.assign_role(user, admin)?;
        self.assign_role(user, user_role)?;
        self.claimed = true;
        Ok(user)
    }

    pub fn create_user<E: EntropySource>(
        &mut self,
        name: &[u8],
        password: &[u8],
        home: NamespaceRoot,
        entropy: &mut E,
    ) -> Result<UserId, UserError> {
        if !self.claimed {
            return Err(UserError::NotClaimed);
        }
        let user = self.create_record(name, password, home, entropy)?;
        let role = self.ensure_builtin_role(
            b"user",
            CapabilityTemplate {
                root: NamespaceRoot::EMPTY,
                rights: NamespaceRights::READ | NamespaceRights::WRITE,
            },
        )?;
        self.assign_role(user, role)?;
        Ok(user)
    }

    /// Rename is intentionally a policy operation. The caller must already
    /// hold the typed administration capability at the service boundary.
    pub fn rename_user(&mut self, user: UserId, name: &[u8]) -> Result<(), UserError> {
        let name = UserName::parse(name)?;
        if self.users.iter().flatten().any(|record| record.id != user && record.name == name) {
            return Err(UserError::Capacity);
        }
        self.user_mut(user)?.name = name;
        Ok(())
    }

    /// Password replacement returns only a verifier; plaintext is not retained.
    pub fn set_password<E: EntropySource>(
        &mut self,
        user: UserId,
        password: &[u8],
        entropy: &mut E,
    ) -> Result<(), UserError> {
        let mut salt = [0; USER_ARGON2_SALT_BYTES];
        entropy.fill(&mut salt)?;
        let verifier = PasswordVerifier::create(password, salt)?;
        self.user_mut(user)?.verifier = verifier;
        Ok(())
    }

    pub fn is_admin_session(&self, session: SessionHandle) -> Result<bool, UserError> {
        let session = self.session(session)?;
        Ok(self.user_ref(session.user)?.admin)
    }

    pub fn login(
        &mut self,
        name: &[u8],
        password: &[u8],
    ) -> Result<(UserId, SessionHandle), UserError> {
        if !self.claimed {
            return Err(UserError::NotClaimed);
        }
        let name = UserName::parse(name)?;
        let index = self.find_user(name).ok_or(UserError::NotFound)?;
        let user = self.users[index].ok_or(UserError::Corrupt)?;
        user.verifier.verify(password)?;
        let slot = self.sessions.iter().position(Option::is_none).ok_or(UserError::Capacity)?;
        let lineage = self.next_lineage();
        let generation = 1;
        let handle = SessionHandle::new(slot as u32, generation).ok_or(UserError::Corrupt)?;
        self.sessions[slot] = Some(SessionRecord {
            handle,
            user: user.id,
            lineage,
            revoked: false,
            capabilities: [None; MAX_CAPABILITIES_PER_SESSION],
            capability_generations: [1; MAX_CAPABILITIES_PER_SESSION],
        });
        let mut templates = [None; MAX_ROLE_GRANTS * MAX_ROLE_TEMPLATES];
        let mut template_count = 0;
        for role_id in user.roles[..user.role_count].iter().copied() {
            if let Some(role) = self.roles.iter().flatten().find(|role| role.id == role_id) {
                for template in role.templates[..role.template_count].iter().flatten().copied() {
                    if template_count == templates.len() {
                        return Err(UserError::Capacity);
                    }
                    templates[template_count] = Some(template);
                    template_count += 1;
                }
            }
        }
        for template in templates[..template_count].iter().flatten().copied() {
            let root = if template.root.is_valid() { template.root } else { user.home };
            self.attach_capability(handle, root, template.rights)?;
        }
        Ok((user.id, handle))
    }

    pub fn create_role(
        &mut self,
        name: &[u8],
        template: CapabilityTemplate,
    ) -> Result<RoleId, UserError> {
        if !template.rights.is_valid() || !template.root.is_valid() {
            return Err(UserError::InvalidName);
        }
        let name = RoleName::parse(name)?;
        if self.roles.iter().flatten().any(|role| role.name == name) {
            return Err(UserError::Capacity);
        }
        let slot = self.roles.iter().position(Option::is_none).ok_or(UserError::Capacity)?;
        let id = RoleId::new(self.next_role, 1).ok_or(UserError::Corrupt)?;
        self.next_role = self.next_role.checked_add(1).ok_or(UserError::Capacity)?;
        let mut templates = [None; MAX_ROLE_TEMPLATES];
        templates[0] = Some(template);
        self.roles[slot] = Some(RoleRecord { id, name, templates, template_count: 1 });
        Ok(id)
    }

    pub fn assign_role(&mut self, user: UserId, role: RoleId) -> Result<(), UserError> {
        if !self.roles.iter().flatten().any(|record| record.id == role) {
            return Err(UserError::NotFound);
        }
        self.assign_role_inner(user, role)
    }

    pub fn encode_snapshot(&self, output: &mut [u8]) -> Result<usize, UserError> {
        if output.len() < USER_SNAPSHOT_BYTES {
            return Err(UserError::Capacity);
        }
        output[..USER_SNAPSHOT_BYTES].fill(0);
        output[..8].copy_from_slice(&SNAPSHOT_MAGIC);
        put_u16(output, 8, SNAPSHOT_VERSION);
        output[10] = u8::from(self.claimed);
        put_u64(output, 16, self.next_user);
        put_u64(output, 24, self.next_role);
        put_u64(output, 32, self.next_lineage);
        let mut offset = 40;
        for user in self.users {
            output[offset] = u8::from(user.is_some());
            offset += 1;
            if let Some(user) = user {
                encode_user(output, &mut offset, user);
            } else {
                offset += encoded_user_bytes();
            }
        }
        for role in self.roles {
            output[offset] = u8::from(role.is_some());
            offset += 1;
            if let Some(role) = role {
                encode_role(output, &mut offset, role);
            } else {
                offset += encoded_role_bytes();
            }
        }
        Ok(offset)
    }

    pub fn restore_snapshot(&mut self, input: &[u8]) -> Result<(), UserError> {
        if input.len() < 40 || input[..8] != SNAPSHOT_MAGIC {
            return Err(UserError::Corrupt);
        }
        let mut header_offset = 8;
        if read_u16(input, &mut header_offset)? != SNAPSHOT_VERSION {
            return Err(UserError::Corrupt);
        }
        let claimed = input[10] != 0;
        let mut counter_offset = 16;
        let next_user = read_u64(input, &mut counter_offset)?;
        let next_role = read_u64(input, &mut counter_offset)?;
        let next_lineage = read_u64(input, &mut counter_offset)?;
        if next_user == 0 || next_role == 0 || next_lineage == 0 {
            return Err(UserError::Corrupt);
        }
        let mut restored = Self::new();
        restored.claimed = claimed;
        restored.next_user = next_user;
        restored.next_role = next_role;
        restored.next_lineage = next_lineage;
        let mut offset = 40;
        for slot in 0..MAX_USERS {
            let present = *input.get(offset).ok_or(UserError::Corrupt)?;
            offset += 1;
            if present > 1 {
                return Err(UserError::Corrupt);
            }
            if present == 1 {
                restored.users[slot] = Some(decode_user(input, &mut offset)?);
            } else {
                offset += encoded_user_bytes();
            }
        }
        for slot in 0..MAX_ROLES {
            let present = *input.get(offset).ok_or(UserError::Corrupt)?;
            offset += 1;
            if present > 1 {
                return Err(UserError::Corrupt);
            }
            if present == 1 {
                restored.roles[slot] = Some(decode_role(input, &mut offset)?);
            } else {
                offset += encoded_role_bytes();
            }
        }
        self.claimed = restored.claimed;
        self.next_user = restored.next_user;
        self.next_role = restored.next_role;
        self.next_lineage = restored.next_lineage;
        self.users = restored.users;
        self.roles = restored.roles;
        self.sessions = [None; MAX_SESSIONS];
        Ok(())
    }

    pub fn load_from<S: UserCatalogStore>(
        &mut self,
        store: &mut S,
        buffer: &mut [u8],
    ) -> Result<(), UserError> {
        let length = match store.load(buffer) {
            Ok(length) => length,
            Err(UserError::NotFound) => {
                *self = Self::new();
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        if length > buffer.len() {
            return Err(UserError::Persistence);
        }
        self.restore_snapshot(&buffer[..length])
    }

    pub fn save_to<S: UserCatalogStore>(
        &self,
        store: &mut S,
        buffer: &mut [u8],
    ) -> Result<(), UserError> {
        let length = self.encode_snapshot(buffer)?;
        store.save(&buffer[..length])
    }

    pub fn logout(&mut self, session: SessionHandle) -> Result<(), UserError> {
        let record = self.session_mut(session)?;
        record.revoked = true;
        Ok(())
    }

    pub fn derive(
        &mut self,
        session: SessionHandle,
        parent: NamespaceCapabilityHandle,
        root: NamespaceRoot,
        rights: NamespaceRights,
    ) -> Result<NamespaceCapabilityHandle, UserError> {
        if !rights.is_valid() {
            return Err(UserError::Unauthorized);
        }
        let lineage = self.next_lineage();
        let record = self.session_mut(session)?;
        let parent_record = record
            .capabilities
            .iter()
            .flatten()
            .find(|capability| capability.handle == parent && !capability.revoked)
            .copied()
            .ok_or(UserError::Stale)?;
        if !parent_record.rights.contains(NamespaceRights::DERIVE)
            || parent_record.root != root
            || !parent_record.rights.contains(rights)
        {
            return Err(UserError::Unauthorized);
        }
        let slot =
            record.capabilities.iter().position(Option::is_none).ok_or(UserError::Capacity)?;
        let generation = record.capability_generations[slot];
        let handle =
            NamespaceCapabilityHandle::new(slot as u32, generation).ok_or(UserError::Corrupt)?;
        record.capabilities[slot] = Some(CapabilityRecord {
            handle,
            root,
            rights,
            lineage,
            parent_lineage: parent_record.lineage,
            revoked: false,
        });
        Ok(handle)
    }

    pub fn revoke_capability(
        &mut self,
        session: SessionHandle,
        capability: NamespaceCapabilityHandle,
    ) -> Result<(), UserError> {
        let record = self.session_mut(session)?;
        let target = record
            .capabilities
            .iter()
            .flatten()
            .find(|entry| entry.handle == capability)
            .map(|entry| entry.lineage)
            .ok_or(UserError::Stale)?;
        let mut revoked = [0u64; MAX_CAPABILITIES_PER_SESSION];
        let mut revoked_count = 1;
        revoked[0] = target;
        for _ in 0..MAX_CAPABILITIES_PER_SESSION {
            let mut changed = false;
            for entry in record.capabilities.iter_mut().flatten() {
                let ancestor = entry.lineage == target
                    || revoked[..revoked_count].contains(&entry.parent_lineage);
                if ancestor && !entry.revoked {
                    entry.revoked = true;
                    if revoked_count < revoked.len() {
                        revoked[revoked_count] = entry.lineage;
                        revoked_count += 1;
                    }
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        Ok(())
    }

    pub fn attach_capability(
        &mut self,
        session: SessionHandle,
        root: NamespaceRoot,
        rights: NamespaceRights,
    ) -> Result<NamespaceCapabilityHandle, UserError> {
        if !rights.is_valid() {
            return Err(UserError::Unauthorized);
        }
        let lineage = self.next_lineage();
        let record = self.session_mut(session)?;
        let slot =
            record.capabilities.iter().position(Option::is_none).ok_or(UserError::Capacity)?;
        let generation = record.capability_generations[slot];
        let handle =
            NamespaceCapabilityHandle::new(slot as u32, generation).ok_or(UserError::Corrupt)?;
        record.capabilities[slot] = Some(CapabilityRecord {
            handle,
            root,
            rights,
            lineage,
            parent_lineage: record.lineage,
            revoked: false,
        });
        Ok(handle)
    }

    pub fn capability(
        &self,
        session: SessionHandle,
        capability: NamespaceCapabilityHandle,
    ) -> Result<(NamespaceRoot, NamespaceRights), UserError> {
        let record = self.session(session)?;
        let entry = record
            .capabilities
            .iter()
            .flatten()
            .find(|entry| entry.handle == capability)
            .ok_or(UserError::Stale)?;
        if entry.revoked || record.revoked {
            return Err(UserError::Revoked);
        }
        Ok((entry.root, entry.rights))
    }

    pub fn first_capability(
        &self,
        session: SessionHandle,
    ) -> Result<(NamespaceCapabilityHandle, NamespaceRoot, NamespaceRights), UserError> {
        let record = self.session(session)?;
        let capability = record.capabilities.iter().flatten().next().ok_or(UserError::NotFound)?;
        if capability.revoked || record.revoked {
            return Err(UserError::Revoked);
        }
        Ok((capability.handle, capability.root, capability.rights))
    }

    fn create_record<E: EntropySource>(
        &mut self,
        name: &[u8],
        password: &[u8],
        home: NamespaceRoot,
        entropy: &mut E,
    ) -> Result<UserId, UserError> {
        let name = UserName::parse(name)?;
        if self.find_user(name).is_some() {
            return Err(UserError::Capacity);
        }
        let slot = self.users.iter().position(Option::is_none).ok_or(UserError::Capacity)?;
        let id = UserId::new(self.next_user, 1).ok_or(UserError::Corrupt)?;
        self.next_user = self.next_user.checked_add(1).ok_or(UserError::Capacity)?;
        let mut salt = [0; USER_ARGON2_SALT_BYTES];
        entropy.fill(&mut salt)?;
        let verifier = PasswordVerifier::create(password, salt)?;
        self.users[slot] = Some(UserRecord {
            id,
            name,
            verifier,
            home,
            roles: [RoleId::EMPTY; MAX_ROLE_GRANTS],
            role_count: 0,
            next_lineage: 1,
            admin: false,
        });
        Ok(id)
    }

    fn ensure_builtin_role(
        &mut self,
        name: &[u8],
        template: CapabilityTemplate,
    ) -> Result<RoleId, UserError> {
        let name = RoleName::parse(name)?;
        if let Some(role) = self.roles.iter().flatten().find(|role| role.name == name) {
            return Ok(role.id);
        }
        let slot = self.roles.iter().position(Option::is_none).ok_or(UserError::Capacity)?;
        let id = RoleId::new(self.next_role, 1).ok_or(UserError::Corrupt)?;
        self.next_role = self.next_role.checked_add(1).ok_or(UserError::Capacity)?;
        let mut templates = [None; MAX_ROLE_TEMPLATES];
        templates[0] = Some(template);
        self.roles[slot] = Some(RoleRecord { id, name, templates, template_count: 1 });
        Ok(id)
    }

    fn assign_role_inner(&mut self, user: UserId, role: RoleId) -> Result<(), UserError> {
        let record = self.user_mut(user)?;
        if record.role_count == MAX_ROLE_GRANTS {
            return Err(UserError::Capacity);
        }
        record.roles[record.role_count] = role;
        record.role_count += 1;
        Ok(())
    }

    fn find_user(&self, name: UserName) -> Option<usize> {
        self.users.iter().position(|user| user.is_some_and(|user| user.name == name))
    }

    fn user_mut(&mut self, user: UserId) -> Result<&mut UserRecord, UserError> {
        self.users.iter_mut().flatten().find(|record| record.id == user).ok_or(UserError::NotFound)
    }

    fn user_ref(&self, user: UserId) -> Result<&UserRecord, UserError> {
        self.users.iter().flatten().find(|record| record.id == user).ok_or(UserError::NotFound)
    }

    fn session(&self, handle: SessionHandle) -> Result<&SessionRecord, UserError> {
        let record = self
            .sessions
            .get(handle.slot as usize)
            .and_then(Option::as_ref)
            .ok_or(UserError::Stale)?;
        if record.handle != handle {
            return Err(UserError::Stale);
        }
        if record.revoked {
            return Err(UserError::Revoked);
        }
        Ok(record)
    }

    fn session_mut(&mut self, handle: SessionHandle) -> Result<&mut SessionRecord, UserError> {
        let record = self
            .sessions
            .get_mut(handle.slot as usize)
            .and_then(Option::as_mut)
            .ok_or(UserError::Stale)?;
        if record.handle != handle {
            return Err(UserError::Stale);
        }
        if record.revoked {
            return Err(UserError::Revoked);
        }
        Ok(record)
    }

    fn next_lineage(&mut self) -> u64 {
        let value = self.next_lineage;
        self.next_lineage = self.next_lineage.wrapping_add(1).max(1);
        value
    }
}

impl Default for UserCatalog {
    fn default() -> Self {
        Self::new()
    }
}

const fn encoded_user_bytes() -> usize {
    212
}
const fn encoded_role_bytes() -> usize {
    158
}

fn encode_user(output: &mut [u8], offset: &mut usize, user: UserRecord) {
    put_u64(output, *offset, user.id.value);
    *offset += 8;
    put_u32(output, *offset, user.id.generation);
    *offset += 4;
    output[*offset] = user.name.len;
    *offset += 1;
    output[*offset..*offset + user.name.bytes.len()].copy_from_slice(&user.name.bytes);
    *offset += user.name.bytes.len();
    output[*offset] = user.verifier.version;
    *offset += 1;
    output[*offset..*offset + user.verifier.salt.len()].copy_from_slice(&user.verifier.salt);
    *offset += user.verifier.salt.len();
    output[*offset..*offset + user.verifier.output.len()].copy_from_slice(&user.verifier.output);
    *offset += user.verifier.output.len();
    put_u64(output, *offset, user.home.object);
    *offset += 8;
    put_u32(output, *offset, user.home.generation);
    *offset += 4;
    output[*offset] = user.role_count as u8;
    *offset += 1;
    output[*offset] = u8::from(user.admin);
    *offset += 1;
    for role in user.roles {
        put_u64(output, *offset, role.value);
        *offset += 8;
        put_u32(output, *offset, role.generation);
        *offset += 4;
    }
    put_u64(output, *offset, user.next_lineage);
    *offset += 8;
}

fn decode_user(input: &[u8], offset: &mut usize) -> Result<UserRecord, UserError> {
    let id = UserId::new(read_u64(input, offset)?, read_u32(input, offset)?)
        .ok_or(UserError::Corrupt)?;
    let name_len = read_u8(input, offset)? as usize;
    let name_bytes = read_array::<USER_MAX_USER_NAME_BYTES>(input, offset)?;
    let name = UserName::parse(&name_bytes[..name_len])?;
    let version = read_u8(input, offset)?;
    let salt = read_array::<USER_ARGON2_SALT_BYTES>(input, offset)?;
    let output = read_array::<USER_ARGON2_OUTPUT_BYTES>(input, offset)?;
    let home = NamespaceRoot::new(read_u64(input, offset)?, read_u32(input, offset)?)
        .ok_or(UserError::Corrupt)?;
    let role_count = read_u8(input, offset)? as usize;
    if role_count > MAX_ROLE_GRANTS {
        return Err(UserError::Corrupt);
    }
    let admin = match read_u8(input, offset)? {
        0 => false,
        1 => true,
        _ => return Err(UserError::Corrupt),
    };
    let mut roles = [RoleId::EMPTY; MAX_ROLE_GRANTS];
    for role in &mut roles {
        let value = read_u64(input, offset)?;
        let generation = read_u32(input, offset)?;
        *role = RoleId::new(value, generation).unwrap_or(RoleId::EMPTY);
    }
    let next_lineage = read_u64(input, offset)?;
    if next_lineage == 0 {
        return Err(UserError::Corrupt);
    }
    Ok(UserRecord {
        id,
        name,
        verifier: PasswordVerifier { version, salt, output },
        home,
        roles,
        role_count,
        next_lineage,
        admin,
    })
}

fn encode_role(output: &mut [u8], offset: &mut usize, role: RoleRecord) {
    put_u64(output, *offset, role.id.value);
    *offset += 8;
    put_u32(output, *offset, role.id.generation);
    *offset += 4;
    output[*offset] = role.name.len;
    *offset += 1;
    output[*offset..*offset + role.name.bytes.len()].copy_from_slice(&role.name.bytes);
    *offset += role.name.bytes.len();
    output[*offset] = role.template_count as u8;
    *offset += 1;
    for template in role.templates {
        output[*offset] = u8::from(template.is_some());
        *offset += 1;
        let template = template.unwrap_or(CapabilityTemplate {
            root: NamespaceRoot::EMPTY,
            rights: NamespaceRights::NONE,
        });
        put_u64(output, *offset, template.root.object);
        *offset += 8;
        put_u32(output, *offset, template.root.generation);
        *offset += 4;
        output[*offset] = template.rights.0;
        *offset += 1;
    }
}

fn decode_role(input: &[u8], offset: &mut usize) -> Result<RoleRecord, UserError> {
    let id = RoleId::new(read_u64(input, offset)?, read_u32(input, offset)?)
        .ok_or(UserError::Corrupt)?;
    let name_len = read_u8(input, offset)? as usize;
    let name_bytes = read_array::<USER_MAX_ROLE_NAME_BYTES>(input, offset)?;
    let name = RoleName::parse(&name_bytes[..name_len])?;
    let template_count = read_u8(input, offset)? as usize;
    if template_count > MAX_ROLE_TEMPLATES {
        return Err(UserError::Corrupt);
    }
    let mut templates = [None; MAX_ROLE_TEMPLATES];
    for template in &mut templates {
        let present = read_u8(input, offset)?;
        let root_object = read_u64(input, offset)?;
        let root_generation = read_u32(input, offset)?;
        let rights = NamespaceRights(read_u8(input, offset)?);
        if present > 1
            || (present == 1
                && (!rights.is_valid()
                    || (root_object != 0 && root_generation == 0)
                    || (root_object == 0 && root_generation != 0)))
        {
            return Err(UserError::Corrupt);
        }
        if present == 1 {
            let root = if root_object == 0 {
                NamespaceRoot::EMPTY
            } else {
                NamespaceRoot::new(root_object, root_generation).ok_or(UserError::Corrupt)?
            };
            *template = Some(CapabilityTemplate { root, rights });
        }
    }
    Ok(RoleRecord { id, name, templates, template_count })
}

fn put_u16(output: &mut [u8], offset: usize, value: u16) {
    output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn read_u16(input: &[u8], offset: &mut usize) -> Result<u16, UserError> {
    let end = (*offset).checked_add(2).ok_or(UserError::Corrupt)?;
    let bytes = input.get(*offset..end).ok_or(UserError::Corrupt)?;
    *offset = end;
    Ok(u16::from_le_bytes(bytes.try_into().map_err(|_| UserError::Corrupt)?))
}

fn put_u32(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(output: &mut [u8], offset: usize, value: u64) {
    output[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn read_u8(input: &[u8], offset: &mut usize) -> Result<u8, UserError> {
    let value = *input.get(*offset).ok_or(UserError::Corrupt)?;
    *offset += 1;
    Ok(value)
}

fn read_u32(input: &[u8], offset: &mut usize) -> Result<u32, UserError> {
    let end = (*offset).checked_add(4).ok_or(UserError::Corrupt)?;
    let bytes = input.get(*offset..end).ok_or(UserError::Corrupt)?;
    *offset = end;
    Ok(u32::from_le_bytes(bytes.try_into().map_err(|_| UserError::Corrupt)?))
}

fn read_u64(input: &[u8], offset: &mut usize) -> Result<u64, UserError> {
    let end = (*offset).checked_add(8).ok_or(UserError::Corrupt)?;
    let bytes = input.get(*offset..end).ok_or(UserError::Corrupt)?;
    *offset = end;
    Ok(u64::from_le_bytes(bytes.try_into().map_err(|_| UserError::Corrupt)?))
}

fn read_array<const N: usize>(input: &[u8], offset: &mut usize) -> Result<[u8; N], UserError> {
    let end = (*offset).checked_add(N).ok_or(UserError::Corrupt)?;
    let bytes = input.get(*offset..end).ok_or(UserError::Corrupt)?;
    *offset = end;
    bytes.try_into().map_err(|_| UserError::Corrupt)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in left.iter().zip(right) {
        difference |= left ^ right;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Entropy(u8);

    impl EntropySource for Entropy {
        fn fill(&mut self, output: &mut [u8]) -> Result<(), UserError> {
            for byte in output {
                *byte = self.0;
                self.0 = self.0.wrapping_add(1);
            }
            Ok(())
        }
    }

    struct CatalogStore {
        data: [u8; USER_SNAPSHOT_BYTES],
        length: usize,
        present: bool,
    }

    impl CatalogStore {
        const fn new() -> Self {
            Self { data: [0; USER_SNAPSHOT_BYTES], length: 0, present: false }
        }
    }

    impl UserCatalogStore for CatalogStore {
        fn load(&mut self, output: &mut [u8]) -> Result<usize, UserError> {
            if !self.present {
                return Err(UserError::NotFound);
            }
            if self.length > output.len() {
                return Err(UserError::Persistence);
            }
            output[..self.length].copy_from_slice(&self.data[..self.length]);
            Ok(self.length)
        }

        fn save(&mut self, snapshot: &[u8]) -> Result<(), UserError> {
            if snapshot.len() > self.data.len() {
                return Err(UserError::Persistence);
            }
            self.data[..snapshot.len()].copy_from_slice(snapshot);
            self.length = snapshot.len();
            self.present = true;
            Ok(())
        }
    }

    fn root(value: u64) -> NamespaceRoot {
        NamespaceRoot::new(value, 1).unwrap()
    }

    #[test]
    fn claim_is_one_shot_and_login_verifies_argon2id_passwords() {
        let mut catalog = UserCatalog::new();
        let mut entropy = Entropy(1);
        assert_eq!(catalog.login(b"admin", b"password"), Err(UserError::NotClaimed));
        catalog.claim(b"admin", b"correct horse", root(1), &mut entropy).unwrap();
        assert!(catalog.is_claimed());
        assert_eq!(
            catalog.claim(b"other", b"password", root(2), &mut entropy),
            Err(UserError::AlreadyClaimed)
        );
        let (_, session) = catalog.login(b"admin", b"correct horse").unwrap();
        assert_eq!(catalog.login(b"admin", b"wrong"), Err(UserError::BadCredentials));
        catalog.logout(session).unwrap();
        assert_eq!(
            catalog.capability(session, NamespaceCapabilityHandle::EMPTY),
            Err(UserError::Revoked)
        );
    }

    #[test]
    fn namespace_capabilities_only_attenuate_and_revoke_descendants() {
        let mut catalog = UserCatalog::new();
        let mut entropy = Entropy(7);
        catalog.claim(b"admin", b"password", root(3), &mut entropy).unwrap();
        let (_, session) = catalog.login(b"admin", b"password").unwrap();
        let parent = catalog
            .attach_capability(
                session,
                root(3),
                NamespaceRights::READ | NamespaceRights::WRITE | NamespaceRights::DERIVE,
            )
            .unwrap();
        let child = catalog.derive(session, parent, root(3), NamespaceRights::READ).unwrap();
        assert_eq!(
            catalog.derive(session, child, root(3), NamespaceRights::WRITE),
            Err(UserError::Unauthorized)
        );
        catalog.revoke_capability(session, parent).unwrap();
        assert_eq!(catalog.capability(session, child), Err(UserError::Revoked));
    }

    #[test]
    fn snapshot_round_trip_drops_volatile_sessions() {
        let mut catalog = UserCatalog::new();
        let mut entropy = Entropy(11);
        catalog.claim(b"admin", b"password", root(4), &mut entropy).unwrap();
        let (_, session) = catalog.login(b"admin", b"password").unwrap();
        let mut snapshot = [0; USER_SNAPSHOT_BYTES];
        let length = catalog.encode_snapshot(&mut snapshot).unwrap();
        let mut restored = UserCatalog::new();
        restored.restore_snapshot(&snapshot[..length]).unwrap();
        assert!(restored.is_claimed());
        assert!(restored.login(b"admin", b"password").is_ok());
        assert_eq!(
            restored.capability(session, NamespaceCapabilityHandle::EMPTY),
            Err(UserError::Stale)
        );
    }

    #[test]
    fn catalog_storage_boundary_round_trips_and_missing_is_first_boot() {
        let mut catalog = UserCatalog::new();
        let mut entropy = Entropy(13);
        catalog.claim(b"admin", b"password", root(6), &mut entropy).unwrap();
        let mut store = CatalogStore::new();
        let mut buffer = [0; USER_SNAPSHOT_BYTES];
        catalog.save_to(&mut store, &mut buffer).unwrap();

        let mut restored = UserCatalog::new();
        restored.load_from(&mut store, &mut buffer).unwrap();
        assert!(restored.is_claimed());
        assert!(restored.login(b"admin", b"password").is_ok());

        let mut missing = CatalogStore::new();
        restored.load_from(&mut missing, &mut buffer).unwrap();
        assert!(!restored.is_claimed());
    }

    #[test]
    fn names_are_canonical_and_bounded() {
        assert!(UserName::parse(b"alice-1").is_ok());
        assert_eq!(UserName::parse(b"Alice"), Err(UserError::InvalidName));
        assert_eq!(UserName::parse(b""), Err(UserError::InvalidName));
    }

    #[test]
    fn administrative_name_and_password_changes_take_effect_without_persisting_sessions() {
        let mut catalog = UserCatalog::new();
        let mut entropy = Entropy(19);
        catalog.claim(b"admin", b"old-password", root(5), &mut entropy).unwrap();
        let (user, session) = catalog.login(b"admin", b"old-password").unwrap();
        catalog.rename_user(user, b"administrator").unwrap();
        assert_eq!(catalog.login(b"admin", b"old-password"), Err(UserError::NotFound));
        assert!(catalog.login(b"administrator", b"old-password").is_ok());
        catalog.set_password(user, b"new-password", &mut entropy).unwrap();
        assert_eq!(
            catalog.login(b"administrator", b"old-password"),
            Err(UserError::BadCredentials)
        );
        assert!(catalog.login(b"administrator", b"new-password").is_ok());
        catalog.logout(session).unwrap();
    }

    #[test]
    fn claim_response_does_not_create_a_session() {
        let mut service = UserService::new(Entropy(23));
        let mut request = UserRequest::new(UserOperation::Claim, 1);
        assert!(request.set_name(b"admin"));
        assert!(request.set_password(b"password"));
        let response = service.handle(request);
        assert_eq!(response.status, UserStatus::Ok);
        assert!(response.user.is_valid());
        assert!(!response.session.is_valid());
        assert!(service.catalog().is_claimed());
    }
}
