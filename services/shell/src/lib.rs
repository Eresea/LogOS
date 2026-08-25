#![no_std]

#[cfg(test)]
extern crate std;

pub use logos_ui::{
    MAX_UI_NODES, UiBlueprint, UiError, UiNode, UiNodeHandle, UiNodeKind, UiNodeSpec, UiTree,
};

pub fn compile_login_page() -> logos_ui_compiler::UiBuild {
    logos_ui_compiler::compile_login_page()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoginUiState {
    pub claim: bool,
    pub failure: bool,
}

impl LoginUiState {
    pub const fn new(claim: bool, failure: bool) -> Self {
        Self { claim, failure }
    }
}

pub fn login_page_text(
    build: &logos_ui_compiler::UiBuild,
    state: LoginUiState,
    output: &mut [u8; logos_abi::MAX_GUI_TEXT_BYTES],
) -> usize {
    if !build.is_valid() {
        return 0;
    }
    let mut length = 0;
    for index in 0..build.document.node_count() {
        let Some(node) = build.document.node(index) else { continue };
        match node.kind {
            UiNodeKind::Label => {
                let text = if node.key.as_bytes() == b"title" {
                    if state.failure {
                        b"Retry login".as_slice()
                    } else if state.claim {
                        b"Claim login".as_slice()
                    } else {
                        node.text.as_bytes()
                    }
                } else {
                    node.text.as_bytes()
                };
                append_text(output, &mut length, text);
            }
            UiNodeKind::TextInput => {
                append_text(output, &mut length, b" [");
                append_text(
                    output,
                    &mut length,
                    if node.key.as_bytes() == b"password" { b"pwd" } else { b"usr" },
                );
                append_text(output, &mut length, b"]");
            }
            UiNodeKind::Button => {
                append_text(output, &mut length, b" [");
                append_text(output, &mut length, node.text.as_bytes());
                append_text(output, &mut length, b"]");
            }
            UiNodeKind::Root | UiNodeKind::Panel | UiNodeKind::Form => {}
        }
    }
    length
}

fn append_text(output: &mut [u8; logos_abi::MAX_GUI_TEXT_BYTES], length: &mut usize, text: &[u8]) {
    for byte in text {
        if *length == output.len() {
            break;
        }
        output[*length] = *byte;
        *length += 1;
    }
}

pub const MAX_LOGIN_LAYOUT_NODES: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginHitTarget {
    Username,
    Password,
    Submit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoginLayoutNode {
    pub index: u16,
    pub kind: UiNodeKind,
    pub bounds: GuiRect,
    pub target: Option<LoginHitTarget>,
}

impl LoginLayoutNode {
    const EMPTY: Self =
        Self { index: u16::MAX, kind: UiNodeKind::Panel, bounds: GuiRect::EMPTY, target: None };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoginLayout {
    nodes: [LoginLayoutNode; MAX_LOGIN_LAYOUT_NODES],
    count: usize,
}

impl LoginLayout {
    pub fn from_build(build: &logos_ui_compiler::UiBuild, viewport: GuiRect) -> Option<Self> {
        if !build.is_valid() || viewport.is_empty() {
            return None;
        }
        let panel = inset(viewport, 24);
        let has_wide_field = (0..build.document.node_count())
            .filter_map(|index| build.document.node(index))
            .any(|node| {
                node.kind == UiNodeKind::TextInput
                    && has_style(node, logos_ui_compiler::UiStyle::Width96)
            });
        let field_width = (if has_wide_field { 384 } else { 240 }).min(panel.width);
        let center_x = panel.x.saturating_add((panel.width / 2) as i32);
        let field_x = center_x.saturating_sub((field_width / 2) as i32);
        let mut layout = Self { nodes: [LoginLayoutNode::EMPTY; MAX_LOGIN_LAYOUT_NODES], count: 0 };
        let mut bounds_by_index = [GuiRect::EMPTY; MAX_UI_NODES];
        let mut cursors = [0i32; MAX_UI_NODES];
        let mut cursor_initialized = [false; MAX_UI_NODES];
        for index in 0..build.document.node_count() {
            let node = build.document.node(index)?;
            let (bounds, target) = match node.kind {
                UiNodeKind::Root | UiNodeKind::Panel | UiNodeKind::Form => (panel, None),
                UiNodeKind::Label | UiNodeKind::TextInput | UiNodeKind::Button => {
                    let parent_index = usize::from(node.parent);
                    let parent = build.document.node(parent_index)?;
                    let parent_bounds = bounds_by_index[parent_index];
                    let horizontal = has_style(parent, logos_ui_compiler::UiStyle::FlexX)
                        && !has_style(parent, logos_ui_compiler::UiStyle::FlexY);
                    if !cursor_initialized[parent_index] {
                        let origin = if parent.kind == UiNodeKind::Form {
                            if horizontal {
                                parent_bounds.x
                            } else {
                                parent_bounds.y.saturating_add(48)
                            }
                        } else if horizontal {
                            parent_bounds.x
                        } else {
                            parent_bounds.y
                        };
                        cursors[parent_index] = origin;
                        cursor_initialized[parent_index] = true;
                    }
                    let (width, height) = match node.kind {
                        UiNodeKind::Label => (field_width, 32),
                        UiNodeKind::TextInput | UiNodeKind::Button => (field_width, 40),
                        _ => unreachable!(),
                    };
                    let position = cursors[parent_index];
                    let gap = gap_px(parent, horizontal);
                    cursors[parent_index] = position
                        .saturating_add(if horizontal { width as i32 } else { height as i32 })
                        .saturating_add(gap);
                    let bounds = if horizontal {
                        GuiRect::new(position, parent_bounds.y.saturating_add(48), width, height)
                    } else {
                        GuiRect::new(field_x, position, width, height)
                    };
                    let target = if node.key.as_bytes() == b"password" {
                        Some(LoginHitTarget::Password)
                    } else if node.kind == UiNodeKind::TextInput {
                        Some(LoginHitTarget::Username)
                    } else if node.kind == UiNodeKind::Button {
                        Some(LoginHitTarget::Submit)
                    } else {
                        None
                    };
                    (bounds, target)
                }
            };
            if layout.count == MAX_LOGIN_LAYOUT_NODES {
                return None;
            }
            bounds_by_index[index] = bounds;
            layout.nodes[layout.count] =
                LoginLayoutNode { index: index as u16, kind: node.kind, bounds, target };
            layout.count += 1;
        }
        Some(layout)
    }

    pub fn node(&self, index: u16) -> Option<LoginLayoutNode> {
        self.nodes[..self.count].iter().copied().find(|node| node.index == index)
    }

    pub fn bounds_for(&self, target: LoginHitTarget) -> Option<GuiRect> {
        self.nodes[..self.count]
            .iter()
            .find(|node| node.target == Some(target))
            .map(|node| node.bounds)
    }

    pub fn hit_test(&self, x: i32, y: i32) -> Option<LoginHitTarget> {
        self.nodes[..self.count]
            .iter()
            .rev()
            .find(|node| node.target.is_some() && node.bounds.contains(x, y))
            .and_then(|node| node.target)
    }
}

fn inset(rect: GuiRect, amount: i32) -> GuiRect {
    let width = rect.width.saturating_sub((amount as u32).saturating_mul(2));
    let height = rect.height.saturating_sub((amount as u32).saturating_mul(2));
    GuiRect::new(rect.x.saturating_add(amount), rect.y.saturating_add(amount), width, height)
}

fn has_style(node: &logos_ui_compiler::UiNodeTemplate, style: logos_ui_compiler::UiStyle) -> bool {
    node.styles.tokens[..node.styles.len as usize].contains(&style)
}

fn gap_px(node: &logos_ui_compiler::UiNodeTemplate, horizontal: bool) -> i32 {
    let mut general = None;
    let mut axis = None;
    for style in &node.styles.tokens[..node.styles.len as usize] {
        match (*style, horizontal) {
            (logos_ui_compiler::UiStyle::Gap(value), _) => general = Some(value),
            (logos_ui_compiler::UiStyle::GapX(value), true)
            | (logos_ui_compiler::UiStyle::GapY(value), false) => axis = Some(value),
            _ => {}
        }
    }
    i32::from(axis.or(general).unwrap_or(0)).saturating_mul(4)
}

pub fn login_page_node_text(
    build: &logos_ui_compiler::UiBuild,
    index: u16,
    state: LoginUiState,
    output: &mut [u8; logos_abi::MAX_GUI_TEXT_BYTES],
) -> usize {
    let Some(node) = build.document.node(usize::from(index)) else { return 0 };
    let text = match node.kind {
        UiNodeKind::Label if node.key.as_bytes() == b"title" && state.failure => b"Retry login",
        UiNodeKind::Label if node.key.as_bytes() == b"title" && state.claim => b"Claim login",
        UiNodeKind::Label | UiNodeKind::Button => node.text.as_bytes(),
        UiNodeKind::TextInput => node.key.as_bytes(),
        UiNodeKind::Root | UiNodeKind::Panel | UiNodeKind::Form => &[],
    };
    let mut length = 0;
    append_text(output, &mut length, text);
    length
}

use logos_abi::{
    GuiRect, GuiSessionContext, InputMessage, UserOperation, UserRequest, UserResponse, UserStatus,
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

    #[test]
    fn login_page_compiles_into_the_bounded_runtime_blueprint() {
        let build = compile_login_page();
        assert!(build.is_valid());
        let blueprint = build.document.to_blueprint().unwrap();
        let tree = UiTree::from_blueprint(&blueprint).unwrap();
        assert_eq!(tree.len(), 6);
    }

    #[test]
    fn login_page_text_is_compiled_and_bounded() {
        let build = compile_login_page();
        let mut output = [0; logos_abi::MAX_GUI_TEXT_BYTES];
        let length = login_page_text(&build, LoginUiState::new(false, false), &mut output);
        assert_eq!(&output[..length], b"LogOS [usr] [pwd] [Unlock]");
        let length = login_page_text(&build, LoginUiState::new(false, true), &mut output);
        assert_eq!(&output[..length], b"Retry login [usr] [pwd] [Unlock]");
    }

    #[test]
    fn login_layout_positions_fields_and_hit_tests_targets() {
        let build = compile_login_page();
        let layout = LoginLayout::from_build(&build, GuiRect::new(0, 0, 640, 400)).unwrap();
        let username = layout.bounds_for(LoginHitTarget::Username).unwrap();
        let password = layout.bounds_for(LoginHitTarget::Password).unwrap();
        let submit = layout.bounds_for(LoginHitTarget::Submit).unwrap();
        let title = layout.node(2).unwrap().bounds;
        assert_eq!(username.width, 384);
        assert_eq!(username.y, title.y + title.height as i32 + 16);
        assert_eq!(password.y, username.y + username.height as i32 + 16);
        assert_eq!(submit.y, password.y + password.height as i32 + 16);
        assert!(password.y > username.y);
        assert!(submit.y > password.y);
        assert_eq!(layout.hit_test(username.x + 1, username.y + 1), Some(LoginHitTarget::Username));
        assert_eq!(layout.hit_test(password.x + 1, password.y + 1), Some(LoginHitTarget::Password));
        assert_eq!(layout.hit_test(submit.x + 1, submit.y + 1), Some(LoginHitTarget::Submit));
        assert_eq!(layout.hit_test(0, 0), None);
    }

    #[test]
    fn login_page_node_text_comes_from_compiled_nodes() {
        let build = compile_login_page();
        let mut output = [0; logos_abi::MAX_GUI_TEXT_BYTES];
        let length = login_page_node_text(&build, 3, LoginUiState::new(false, false), &mut output);
        assert_eq!(&output[..length], b"username");
        let length = login_page_node_text(&build, 5, LoginUiState::new(false, false), &mut output);
        assert_eq!(&output[..length], b"Unlock");
    }
}
