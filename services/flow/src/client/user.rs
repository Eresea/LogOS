use core::mem;

use logos_abi::{
    GuiSessionContext, IpcBytes, IpcStatus, MessageKind, UserOperation, UserRequest, UserResponse,
    UserStatus,
};

use crate::{IdSpace, MAX_OUTPUT_BYTES, PendingOutput, Port, Transport};

pub struct UserClient {
    active: bool,
    done: bool,
    sent: bool,
    request: UserRequest,
    session: logos_abi::SessionHandle,
    user: logos_abi::UserId,
    capability: logos_abi::NamespaceCapabilityHandle,
    root: logos_abi::NamespaceRoot,
    rights: logos_abi::NamespaceRights,
    result: [u8; MAX_OUTPUT_BYTES],
    result_len: usize,
}

impl Default for UserClient {
    fn default() -> Self {
        Self::new()
    }
}

impl UserClient {
    pub const fn new() -> Self {
        Self {
            active: false,
            done: false,
            sent: false,
            request: UserRequest::new(UserOperation::Login, 1),
            session: logos_abi::SessionHandle::EMPTY,
            user: logos_abi::UserId::EMPTY,
            capability: logos_abi::NamespaceCapabilityHandle::EMPTY,
            root: logos_abi::NamespaceRoot::EMPTY,
            rights: logos_abi::NamespaceRights::NONE,
            result: [0; MAX_OUTPUT_BYTES],
            result_len: 0,
        }
    }

    pub fn start<T: Transport>(
        &mut self,
        transport: &mut T,
        command: crate::UserCommand<'_>,
    ) -> bool {
        use crate::UserCommand;
        if self.active || self.done {
            return false;
        }
        let operation = match command {
            UserCommand::Claim { .. } => UserOperation::Claim,
            UserCommand::Create { .. } => UserOperation::Create,
            UserCommand::Login { .. } => UserOperation::Login,
            UserCommand::Logout => UserOperation::Logout,
            UserCommand::Rename { .. } => UserOperation::Rename,
            UserCommand::SetPassword { .. } => UserOperation::SetPassword,
            UserCommand::Derive { .. } => UserOperation::Derive,
            UserCommand::RevokeCapability => UserOperation::RevokeCapability,
        };
        let mut request = UserRequest::new(operation, transport.next_request_id(IdSpace::User));
        request.session = self.session;
        request.user = self.user;
        request.capability = self.capability;
        request.root = self.root;
        request.rights = self.rights;
        match command {
            UserCommand::Claim { name, password }
            | UserCommand::Create { name, password }
            | UserCommand::Login { name, password } => {
                if !request.set_name(name) || !request.set_password(password) {
                    return false;
                }
            }
            UserCommand::Rename { name } => {
                if !request.set_name(name) {
                    return false;
                }
            }
            UserCommand::SetPassword { password } => {
                if !request.set_password(password) {
                    return false;
                }
            }
            UserCommand::Logout | UserCommand::RevokeCapability => {}
            UserCommand::Derive { rights } => {
                request.rights = rights;
            }
        }
        self.request = request;
        self.active = true;
        self.sent = false;
        self.result_len = 0;
        true
    }

    pub fn active(&self) -> bool {
        self.active
    }

    pub fn done(&self) -> bool {
        self.done
    }

    pub fn adopt_context(&mut self, context: GuiSessionContext) {
        if context.is_authenticated() {
            self.session = context.session;
            self.user = context.user;
            self.capability = context.capability;
            self.root = context.root;
            self.rights = context.rights;
        } else {
            self.clear_identity();
        }
    }

    fn clear_identity(&mut self) {
        self.session = logos_abi::SessionHandle::EMPTY;
        self.user = logos_abi::UserId::EMPTY;
        self.capability = logos_abi::NamespaceCapabilityHandle::EMPTY;
        self.root = logos_abi::NamespaceRoot::EMPTY;
        self.rights = logos_abi::NamespaceRights::NONE;
    }

    pub fn drive<T: Transport>(&mut self, transport: &mut T) -> bool {
        if !self.active {
            return false;
        }
        if !self.sent {
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    (&self.request as *const UserRequest).cast::<u8>(),
                    mem::size_of::<UserRequest>(),
                )
            };
            let Some(message) = IpcBytes::from_bytes(MessageKind::UserRequest, bytes) else {
                self.fail(b"user request too large\r\n");
                return true;
            };
            match transport.send(Port::User, &message) {
                IpcStatus::Ok => self.sent = true,
                IpcStatus::Full => return false,
                _ => self.fail(b"user service unavailable\r\n"),
            }
            return true;
        }
        let mut message = IpcBytes::empty(MessageKind::UserResponse);
        match transport.receive(Port::User, &mut message) {
            IpcStatus::Ok => {}
            IpcStatus::Empty => return false,
            _ => {
                self.fail(b"user service unavailable\r\n");
                return true;
            }
        }
        let Some(bytes) = message.as_bytes() else {
            self.fail(b"user response malformed\r\n");
            return true;
        };
        if bytes.len() != mem::size_of::<UserResponse>() || !UserResponse::wire_enums_valid(bytes) {
            self.fail(b"user response malformed\r\n");
            return true;
        }
        let response: UserResponse = unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast()) };
        if !response.is_valid_for(self.request) {
            self.fail(b"user response stale\r\n");
            return true;
        }
        if response.status == UserStatus::Ok {
            if matches!(self.request.operation, UserOperation::Claim | UserOperation::Login) {
                self.session = response.session;
                self.user = response.user;
                self.capability = response.capability;
                self.root = response.root;
                self.rights = response.rights;
            } else if self.request.operation == UserOperation::Logout {
                self.clear_identity();
            } else if self.request.operation == UserOperation::RevokeCapability {
                self.capability = logos_abi::NamespaceCapabilityHandle::EMPTY;
            } else if self.request.operation == UserOperation::Derive {
                self.capability = response.capability;
                self.root = response.root;
                self.rights = response.rights;
            }
            self.finish(b"user: ok\r\n");
        } else {
            self.finish(user_status_text(response.status));
        }
        true
    }

    fn finish(&mut self, message: &[u8]) {
        self.result_len = message.len().min(self.result.len());
        self.result[..self.result_len].copy_from_slice(&message[..self.result_len]);
        self.active = false;
        self.done = true;
    }

    fn fail(&mut self, message: &[u8]) {
        self.finish(message);
    }

    pub fn take_result(&mut self, pending: &mut PendingOutput) {
        if self.done {
            pending.stage(&self.result[..self.result_len]);
            self.done = false;
        }
    }
}

pub fn user_status_text(status: UserStatus) -> &'static [u8] {
    match status {
        UserStatus::Unclaimed => b"user: system is unclaimed\r\n",
        UserStatus::AlreadyClaimed => b"user: already claimed\r\n",
        UserStatus::NotFound => b"user: user not found\r\n",
        UserStatus::Unauthorized => b"user: unauthorized\r\n",
        UserStatus::BadCredentials => b"user: bad credentials\r\n",
        UserStatus::Stale => b"user: stale handle\r\n",
        UserStatus::Revoked => b"user: revoked\r\n",
        UserStatus::Capacity => b"user: capacity exhausted\r\n",
        UserStatus::Corrupt | UserStatus::Invalid => b"user: invalid\r\n",
        UserStatus::Ok => b"user: ok\r\n",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UserCommand, transport::fake::FakeTransport};
    use logos_abi::{SessionHandle, UserId};

    fn message<T: Copy>(value: &T) -> IpcBytes {
        let bytes = unsafe {
            core::slice::from_raw_parts((value as *const T).cast::<u8>(), mem::size_of::<T>())
        };
        IpcBytes::from_bytes(MessageKind::UserResponse, bytes).unwrap()
    }

    fn sent_request(transport: &FakeTransport) -> UserRequest {
        let message: IpcBytes = transport.last_sent(Port::User);
        assert_eq!(message.kind, MessageKind::UserRequest);
        unsafe { core::ptr::read_unaligned(message.as_bytes().unwrap().as_ptr().cast()) }
    }

    fn result(client: &mut UserClient) -> std::vec::Vec<u8> {
        let mut pending = PendingOutput::new();
        client.take_result(&mut pending);
        pending.staged().to_vec()
    }

    fn login(transport: &mut FakeTransport, client: &mut UserClient) -> UserRequest {
        assert!(client.start(transport, UserCommand::Login { name: b"ada", password: b"pw" }));
        assert!(client.drive(transport));
        sent_request(transport)
    }

    #[test]
    fn login_failure_maps_status_to_text_and_keeps_the_identity_empty() {
        let mut transport = FakeTransport::new();
        let mut client = UserClient::new();
        let request = login(&mut transport, &mut client);
        assert_eq!(request.operation, UserOperation::Login);
        transport
            .reply(Port::User, &message(&UserResponse::new(request, UserStatus::BadCredentials)));
        assert!(client.drive(&mut transport));
        assert!(!client.active());
        assert_eq!(result(&mut client), b"user: bad credentials\r\n");
        // The next request still carries no session.
        assert!(client.start(&mut transport, UserCommand::Logout));
        client.drive(&mut transport);
        assert_eq!(sent_request(&transport).session, SessionHandle::EMPTY);
    }

    #[test]
    fn login_success_adopts_the_session_for_later_requests() {
        let mut transport = FakeTransport::new();
        let mut client = UserClient::new();
        let request = login(&mut transport, &mut client);
        let mut response = UserResponse::new(request, UserStatus::Ok);
        response.session = SessionHandle::new(3, 7).unwrap();
        response.user = UserId::new(5, 1).unwrap();
        transport.reply(Port::User, &message(&response));
        assert!(client.drive(&mut transport));
        assert_eq!(result(&mut client), b"user: ok\r\n");

        assert!(client.start(&mut transport, UserCommand::Logout));
        client.drive(&mut transport);
        let logout = sent_request(&transport);
        assert_eq!(logout.session, response.session);
        assert_eq!(logout.user, response.user);
        assert_ne!(logout.request_id, request.request_id);
    }

    #[test]
    fn mismatched_request_id_is_rejected_as_stale() {
        let mut transport = FakeTransport::new();
        let mut client = UserClient::new();
        let mut request = login(&mut transport, &mut client);
        request.request_id = request.request_id.wrapping_add(1);
        transport.reply(Port::User, &message(&UserResponse::new(request, UserStatus::Ok)));
        assert!(client.drive(&mut transport));
        assert_eq!(result(&mut client), b"user response stale\r\n");
    }

    #[test]
    fn transport_and_wire_failures_map_to_fixed_text() {
        let mut transport = FakeTransport::new();
        let mut client = UserClient::new();
        login(&mut transport, &mut client);
        transport.reply_status(Port::User, IpcStatus::Disconnected);
        assert!(client.drive(&mut transport));
        assert_eq!(result(&mut client), b"user service unavailable\r\n");

        let mut transport = FakeTransport::new();
        let mut client = UserClient::new();
        login(&mut transport, &mut client);
        transport
            .reply(Port::User, &IpcBytes::from_bytes(MessageKind::UserResponse, b"x").unwrap());
        assert!(client.drive(&mut transport));
        assert_eq!(result(&mut client), b"user response malformed\r\n");

        let mut transport = FakeTransport::new();
        transport.fail_sends(Port::User, IpcStatus::Disconnected);
        let mut client = UserClient::new();
        assert!(client.start(&mut transport, UserCommand::Logout));
        assert!(client.drive(&mut transport));
        assert_eq!(result(&mut client), b"user service unavailable\r\n");
    }

    #[test]
    fn oversize_credentials_and_busy_client_are_refused() {
        let mut transport = FakeTransport::new();
        let mut client = UserClient::new();
        let long = [b'a'; 255];
        assert!(!client.start(&mut transport, UserCommand::Login { name: &long, password: b"" }));
        assert!(client.start(&mut transport, UserCommand::Logout));
        assert!(!client.start(&mut transport, UserCommand::Logout));
    }

    #[test]
    fn status_text_is_stable() {
        assert_eq!(user_status_text(UserStatus::Unclaimed), b"user: system is unclaimed\r\n");
        assert_eq!(user_status_text(UserStatus::Corrupt), user_status_text(UserStatus::Invalid));
    }
}
