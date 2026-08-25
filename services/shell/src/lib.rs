#![no_std]

#[cfg(test)]
extern crate std;

pub use logos_ui::{
    MAX_UI_NODES, UiBlueprint, UiError, UiNode, UiNodeHandle, UiNodeKind, UiNodeSpec, UiTree,
};

use logos_abi::{
    GuiSessionContext, InputMessage, UserOperation, UserRequest, UserResponse, UserStatus,
};

pub const MAX_LOGIN_RETRIES: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellFocus {
    LockScreen,
    Terminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellPhase {
    Locked,
    ClaimPending,
    LoginPending,
    Authenticated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellInputRoute {
    LockScreen(InputMessage),
    Terminal(InputMessage),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellError {
    InvalidRequest,
    Busy,
    NotAuthenticated,
    RetryLimit,
    Stale,
}

pub struct Shell {
    phase: ShellPhase,
    focus: ShellFocus,
    context: GuiSessionContext,
    pending_request: u32,
    next_request: u32,
    retries: u8,
    restart_generation: u16,
}

impl Shell {
    pub const fn new() -> Self {
        Self {
            phase: ShellPhase::Locked,
            focus: ShellFocus::LockScreen,
            context: GuiSessionContext::EMPTY,
            pending_request: 0,
            next_request: 1,
            retries: 0,
            restart_generation: 1,
        }
    }

    pub const fn phase(&self) -> ShellPhase {
        self.phase
    }

    pub const fn focus(&self) -> ShellFocus {
        self.focus
    }

    pub const fn context(&self) -> GuiSessionContext {
        self.context
    }

    pub const fn retries(&self) -> u8 {
        self.retries
    }

    pub fn route_input(&self, input: InputMessage) -> ShellInputRoute {
        match self.focus {
            ShellFocus::LockScreen => ShellInputRoute::LockScreen(input),
            ShellFocus::Terminal => ShellInputRoute::Terminal(input),
        }
    }

    pub fn begin_user_request(
        &mut self,
        operation: UserOperation,
        name: &[u8],
        password: &[u8],
    ) -> Result<UserRequest, ShellError> {
        if self.pending_request != 0 {
            return Err(ShellError::Busy);
        }
        if !matches!(operation, UserOperation::Claim | UserOperation::Login) {
            return Err(ShellError::InvalidRequest);
        }
        if operation == UserOperation::Login && self.retries >= MAX_LOGIN_RETRIES {
            return Err(ShellError::RetryLimit);
        }
        let request_id = self.next_request;
        self.next_request = self.next_request.wrapping_add(1).max(1);
        let mut request = UserRequest::new(operation, request_id);
        if !request.set_name(name) || !request.set_password(password) {
            return Err(ShellError::InvalidRequest);
        }
        self.pending_request = request_id;
        self.phase = if operation == UserOperation::Claim {
            ShellPhase::ClaimPending
        } else {
            ShellPhase::LoginPending
        };
        Ok(request)
    }

    pub fn acknowledge_sent(request: &mut UserRequest) {
        request.password.fill(0);
        request.password_len = 0;
    }

    pub fn apply_user_response(&mut self, response: UserResponse) -> Result<(), ShellError> {
        if response.request_id != self.pending_request || self.pending_request == 0 {
            self.clear_context();
            return Err(ShellError::Stale);
        }
        self.pending_request = 0;
        match response.status {
            UserStatus::Ok
                if response.session.is_valid()
                    && response.user.is_valid()
                    && response.capability.is_valid()
                    && response.root.is_valid()
                    && response.rights.is_valid() =>
            {
                self.context = GuiSessionContext {
                    session: response.session,
                    user: response.user,
                    capability: response.capability,
                    root: response.root,
                    rights: response.rights,
                    reserved: [0; 3],
                };
                self.phase = ShellPhase::Authenticated;
                self.focus = ShellFocus::Terminal;
                self.retries = 0;
                Ok(())
            }
            UserStatus::Unclaimed => {
                self.clear_context();
                self.phase = ShellPhase::Locked;
                self.focus = ShellFocus::LockScreen;
                Ok(())
            }
            UserStatus::BadCredentials => {
                self.clear_context();
                self.phase = ShellPhase::Locked;
                self.focus = ShellFocus::LockScreen;
                self.retries = self.retries.saturating_add(1);
                Ok(())
            }
            _ => {
                self.clear_context();
                self.phase = ShellPhase::Locked;
                self.focus = ShellFocus::LockScreen;
                Ok(())
            }
        }
    }

    pub fn logout(&mut self) -> Result<UserRequest, ShellError> {
        let context = self.context;
        if !context.is_authenticated() {
            return Err(ShellError::NotAuthenticated);
        }
        let request_id = self.next_request;
        self.next_request = self.next_request.wrapping_add(1).max(1);
        let mut request = UserRequest::new(UserOperation::Logout, request_id);
        request.session = context.session;
        self.pending_request = request_id;
        self.clear_context();
        self.phase = ShellPhase::Locked;
        self.focus = ShellFocus::LockScreen;
        Ok(request)
    }

    pub fn restart(&mut self) {
        self.restart_generation = self.restart_generation.wrapping_add(1).max(1);
        self.pending_request = 0;
        self.clear_context();
        self.phase = ShellPhase::Locked;
        self.focus = ShellFocus::LockScreen;
        self.retries = 0;
    }

    fn clear_context(&mut self) {
        self.context = GuiSessionContext::EMPTY;
    }
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

const _: () = assert!(core::mem::size_of::<Shell>() <= 128);

#[cfg(test)]
mod tests {
    use super::*;
    use logos_abi::{
        NamespaceCapabilityHandle, NamespaceRights, NamespaceRoot, SessionHandle, UserId,
    };

    fn response(request: UserRequest, status: UserStatus) -> UserResponse {
        let mut response = UserResponse::new(request, status);
        if status == UserStatus::Ok {
            response.session = SessionHandle::new(1, 2).unwrap();
            response.user = UserId::new(3, 4).unwrap();
            response.capability = NamespaceCapabilityHandle::new(5, 6).unwrap();
            response.root = NamespaceRoot::new(7, 8).unwrap();
            response.rights = NamespaceRights::READ;
        }
        response
    }

    #[test]
    fn focus_routes_lock_screen_then_terminal() {
        let mut shell = Shell::new();
        let input = InputMessage::key(logos_abi::KeyCode::Enter, logos_abi::KeyState::Pressed, 0);
        assert_eq!(shell.route_input(input), ShellInputRoute::LockScreen(input));
        let mut request =
            shell.begin_user_request(UserOperation::Login, b"alice", b"secret").unwrap();
        Shell::acknowledge_sent(&mut request);
        assert!(request.password.iter().all(|byte| *byte == 0));
        shell.apply_user_response(response(request, UserStatus::Ok)).unwrap();
        assert_eq!(shell.route_input(input), ShellInputRoute::Terminal(input));
    }

    #[test]
    fn failure_logout_restart_and_stale_responses_clear_session() {
        let mut shell = Shell::new();
        let request = shell.begin_user_request(UserOperation::Login, b"alice", b"bad").unwrap();
        shell.apply_user_response(response(request, UserStatus::BadCredentials)).unwrap();
        assert_eq!(shell.retries(), 1);
        let request = shell.begin_user_request(UserOperation::Login, b"alice", b"secret").unwrap();
        shell.apply_user_response(response(request, UserStatus::Ok)).unwrap();
        let logout = shell.logout().unwrap();
        assert!(shell.context().is_clear());
        shell.apply_user_response(response(logout, UserStatus::Stale)).unwrap();
        shell.restart();
        assert_eq!(shell.phase(), ShellPhase::Locked);
        assert!(shell.context().is_clear());
    }
}
