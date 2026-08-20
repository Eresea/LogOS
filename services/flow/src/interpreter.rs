#![allow(clippy::module_name_repetitions)]

//! Fixed-size Flow front end.
//!
//! The parser and type checker deliberately own no heap storage.  They borrow
//! the submitted source and keep their AST in a bounded arena so the same
//! implementation can run in host tests and in the ring-3 Flow image.

pub const MAX_SOURCE_BYTES: usize = 256;
pub const MAX_EXPR_NODES: usize = 64;
pub const MAX_STATEMENTS: usize = 16;
pub const MAX_VARIABLES: usize = 8;
pub const MAX_ARGUMENTS: usize = 3;
pub const MAX_VARIABLE_NAME_BYTES: usize = 24;
pub const MAX_FILE_PATH_BYTES: usize = 256;
pub const MAX_PROMISES: usize = 4;
pub const MAX_CALLBACK_DEPTH: usize = 4;
pub const MAX_VALUE_BYTES: usize = 8192;
pub const MAX_ERROR_BYTES: usize = 96;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    pub start: u16,
    pub end: u16,
}

impl Span {
    const EMPTY: Self = Self { start: 0, end: 0 };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExprId(u8);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StmtId(u8);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlowType {
    Void,
    Bool,
    Number,
    String,
    Bytes,
    Namespace(NamespaceKind),
    Service,
    FileHandle,
    Response,
    Promise(PromiseType),
    Callback,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NamespaceKind {
    Filesystem,
    Network,
    System,
    Supervisor,
    Package,
    Program,
    Device,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromiseType {
    Void,
    Bool,
    Number,
    String,
    Bytes,
    Response,
    FileHandle,
}

impl PromiseType {
    pub const fn flow_type(self) -> FlowType {
        FlowType::Promise(self)
    }

    pub const fn value_type(self) -> FlowType {
        match self {
            Self::Void => FlowType::Void,
            Self::Bool => FlowType::Bool,
            Self::Number => FlowType::Number,
            Self::String => FlowType::String,
            Self::Bytes => FlowType::Bytes,
            Self::Response => FlowType::Response,
            Self::FileHandle => FlowType::FileHandle,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationSignature {
    pub namespace: NamespaceKind,
    pub name: &'static [u8],
    pub arguments: [FlowType; MAX_ARGUMENTS],
    pub minimum_argument_count: u8,
    pub argument_count: u8,
    pub result: FlowType,
}

pub struct OperationRegistry;

impl OperationRegistry {
    pub fn lookup(namespace: NamespaceKind, name: &[u8]) -> Option<OperationSignature> {
        let entries = [
            OperationSignature::new(
                NamespaceKind::Filesystem,
                b"open",
                [FlowType::String, FlowType::Void, FlowType::Void],
                1,
                FlowType::FileHandle,
            ),
            OperationSignature::new(
                NamespaceKind::Filesystem,
                b"touch",
                [FlowType::String, FlowType::Void, FlowType::Void],
                1,
                FlowType::FileHandle,
            ),
            OperationSignature::new(
                NamespaceKind::Filesystem,
                b"list",
                [FlowType::Void, FlowType::Void, FlowType::Void],
                0,
                PromiseType::String.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Filesystem,
                b"remove",
                [FlowType::String, FlowType::Void, FlowType::Void],
                1,
                PromiseType::Void.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Device,
                b"list",
                [FlowType::Void, FlowType::Void, FlowType::Void],
                0,
                PromiseType::String.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Filesystem,
                b"move",
                [FlowType::String, FlowType::String, FlowType::Void],
                2,
                PromiseType::Void.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Network,
                b"fetch",
                [FlowType::String, FlowType::String, FlowType::Void],
                2,
                PromiseType::Response.flow_type(),
            )
            .with_minimum_argument_count(1),
            OperationSignature::new(
                NamespaceKind::Network,
                b"status",
                [FlowType::Void, FlowType::Void, FlowType::Void],
                0,
                PromiseType::String.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Network,
                b"ping",
                [FlowType::String, FlowType::Void, FlowType::Void],
                1,
                PromiseType::String.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Network,
                b"tcp-probe",
                [FlowType::String, FlowType::Number, FlowType::Void],
                2,
                PromiseType::String.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::System,
                b"version",
                [FlowType::Void, FlowType::Void, FlowType::Void],
                0,
                PromiseType::String.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::System,
                b"uname",
                [FlowType::Void, FlowType::Void, FlowType::Void],
                0,
                PromiseType::String.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::System,
                b"shutdown",
                [FlowType::Void, FlowType::Void, FlowType::Void],
                0,
                PromiseType::Void.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::System,
                b"reboot",
                [FlowType::Void, FlowType::Void, FlowType::Void],
                0,
                PromiseType::Void.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Supervisor,
                b"status",
                [FlowType::Void, FlowType::Void, FlowType::Void],
                0,
                PromiseType::String.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Supervisor,
                b"name",
                [FlowType::Void, FlowType::Void, FlowType::Void],
                0,
                PromiseType::String.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Supervisor,
                b"version",
                [FlowType::Void, FlowType::Void, FlowType::Void],
                0,
                PromiseType::String.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Supervisor,
                b"start",
                [FlowType::Void, FlowType::Void, FlowType::Void],
                0,
                PromiseType::Void.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Supervisor,
                b"stop",
                [FlowType::Void, FlowType::Void, FlowType::Void],
                0,
                PromiseType::Void.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Supervisor,
                b"restart",
                [FlowType::Void, FlowType::Void, FlowType::Void],
                0,
                PromiseType::Void.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Package,
                b"list",
                [FlowType::Void, FlowType::Void, FlowType::Void],
                0,
                PromiseType::String.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Package,
                b"info",
                [FlowType::String, FlowType::Void, FlowType::Void],
                1,
                PromiseType::String.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Package,
                b"install",
                [FlowType::String, FlowType::Void, FlowType::Void],
                1,
                PromiseType::String.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Program,
                b"start",
                [FlowType::String, FlowType::Void, FlowType::Void],
                1,
                PromiseType::String.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Program,
                b"status",
                [FlowType::String, FlowType::Void, FlowType::Void],
                1,
                PromiseType::String.flow_type(),
            ),
            OperationSignature::new(
                NamespaceKind::Program,
                b"stop",
                [FlowType::String, FlowType::Void, FlowType::Void],
                1,
                PromiseType::String.flow_type(),
            ),
        ];
        let mut index = 0;
        while index < entries.len() {
            if entries[index].namespace == namespace && entries[index].name == name {
                return Some(entries[index]);
            }
            index += 1;
        }
        None
    }
}

impl OperationSignature {
    const fn new(
        namespace: NamespaceKind,
        name: &'static [u8],
        arguments: [FlowType; MAX_ARGUMENTS],
        argument_count: u8,
        result: FlowType,
    ) -> Self {
        Self {
            namespace,
            name,
            arguments,
            minimum_argument_count: argument_count,
            argument_count,
            result,
        }
    }

    const fn with_minimum_argument_count(mut self, count: u8) -> Self {
        self.minimum_argument_count = count;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlowParseError {
    TooLong,
    InvalidToken(Span),
    UnexpectedToken(Span),
    UnterminatedString(Span),
    TooManyExpressions,
    TooManyStatements,
    TooManyArguments(Span),
    CallbackLimit(Span),
    MissingExpression(Span),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlowTypeError {
    UnknownVariable(Span),
    UnknownMember(Span),
    InvalidCall(Span),
    WrongArity(Span),
    Expected { span: Span, expected: FlowType, actual: FlowType },
    ExpectedPromise(Span),
    InvalidReturn(Span),
    VariableLimit(Span),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromiseHandle(u8);

impl PromiseHandle {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromiseState {
    Vacant,
    Pending,
    Ready,
    Rejected,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlowValue {
    Void,
    Bool(bool),
    Number(u16),
    Promise { handle: PromiseHandle, kind: PromiseType },
    String { bytes: [u8; MAX_VALUE_BYTES], len: usize },
    Bytes { bytes: [u8; MAX_VALUE_BYTES], len: usize },
    Response { status: u16, body: [u8; MAX_VALUE_BYTES], body_len: usize },
    FileHandle { path: [u8; MAX_FILE_PATH_BYTES], path_len: usize, create: bool },
}

impl FlowValue {
    pub const fn kind(self) -> FlowType {
        match self {
            Self::Void => FlowType::Void,
            Self::Bool(_) => FlowType::Bool,
            Self::Number(_) => FlowType::Number,
            Self::Promise { kind, .. } => kind.flow_type(),
            Self::String { .. } => FlowType::String,
            Self::Bytes { .. } => FlowType::Bytes,
            Self::Response { .. } => FlowType::Response,
            Self::FileHandle { .. } => FlowType::FileHandle,
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_VALUE_BYTES {
            return None;
        }
        let mut value = [0; MAX_VALUE_BYTES];
        value[..bytes.len()].copy_from_slice(bytes);
        Some(Self::Bytes { bytes: value, len: bytes.len() })
    }

    pub fn string(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_VALUE_BYTES {
            return None;
        }
        let mut value = [0; MAX_VALUE_BYTES];
        value[..bytes.len()].copy_from_slice(bytes);
        Some(Self::String { bytes: value, len: bytes.len() })
    }

    pub fn response(status: u16, body: &[u8]) -> Option<Self> {
        if body.len() > MAX_VALUE_BYTES {
            return None;
        }
        let mut value = [0; MAX_VALUE_BYTES];
        value[..body.len()].copy_from_slice(body);
        Some(Self::Response { status, body: value, body_len: body.len() })
    }

    pub fn bytes(self, output: &mut [u8]) -> Option<usize> {
        let (bytes, len) = match self {
            Self::String { bytes, len } | Self::Bytes { bytes, len } => (bytes, len),
            Self::Response { body, body_len, .. } => (body, body_len),
            _ => return None,
        };
        if output.len() < len {
            return None;
        }
        output[..len].copy_from_slice(&bytes[..len]);
        Some(len)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromiseError {
    InvalidHandle,
    NotReady,
    AlreadyComplete,
    TypeMismatch,
    Capacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PromiseSlot {
    state: PromiseState,
    result: Option<FlowValue>,
    error: Option<u8>,
    kind: PromiseType,
}

impl PromiseSlot {
    const EMPTY: Self =
        Self { state: PromiseState::Vacant, result: None, error: None, kind: PromiseType::Void };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromiseTable {
    slots: [PromiseSlot; MAX_PROMISES],
}

impl PromiseTable {
    pub const fn new() -> Self {
        Self { slots: [PromiseSlot::EMPTY; MAX_PROMISES] }
    }

    pub fn allocate(&mut self, kind: PromiseType) -> Result<PromiseHandle, PromiseError> {
        let Some(index) = self.slots.iter().position(|slot| slot.state == PromiseState::Vacant)
        else {
            return Err(PromiseError::Capacity);
        };
        self.slots[index] = PromiseSlot {
            state: PromiseState::Pending,
            result: Some(FlowValue::Void),
            error: None,
            kind,
        };
        Ok(PromiseHandle(index as u8))
    }

    pub fn kind(&self, handle: PromiseHandle) -> Result<PromiseType, PromiseError> {
        self.slots.get(handle.index()).map(|slot| slot.kind).ok_or(PromiseError::InvalidHandle)
    }

    pub fn state(&self, handle: PromiseHandle) -> Result<PromiseState, PromiseError> {
        self.slots.get(handle.index()).map(|slot| slot.state).ok_or(PromiseError::InvalidHandle)
    }

    pub fn resolve(&mut self, handle: PromiseHandle, value: FlowValue) -> Result<(), PromiseError> {
        let slot = self.slots.get_mut(handle.index()).ok_or(PromiseError::InvalidHandle)?;
        if slot.state != PromiseState::Pending {
            return Err(PromiseError::AlreadyComplete);
        }
        if value.kind() != slot.kind.value_type() {
            return Err(PromiseError::TypeMismatch);
        }
        slot.state = PromiseState::Ready;
        slot.result = Some(value);
        slot.error = None;
        Ok(())
    }

    pub fn reject(&mut self, handle: PromiseHandle, error: u8) -> Result<(), PromiseError> {
        let slot = self.slots.get_mut(handle.index()).ok_or(PromiseError::InvalidHandle)?;
        if slot.state != PromiseState::Pending {
            return Err(PromiseError::AlreadyComplete);
        }
        slot.state = PromiseState::Rejected;
        slot.result = None;
        slot.error = Some(error);
        Ok(())
    }

    pub fn cancel(&mut self, handle: PromiseHandle) -> Result<(), PromiseError> {
        let slot = self.slots.get_mut(handle.index()).ok_or(PromiseError::InvalidHandle)?;
        if slot.state != PromiseState::Pending {
            return Err(PromiseError::AlreadyComplete);
        }
        slot.state = PromiseState::Cancelled;
        slot.result = None;
        slot.error = None;
        Ok(())
    }

    pub fn take(&mut self, handle: PromiseHandle) -> Result<FlowValue, PromiseError> {
        let slot = self.slots.get_mut(handle.index()).ok_or(PromiseError::InvalidHandle)?;
        match slot.state {
            PromiseState::Ready => {
                let value = slot.result.take().ok_or(PromiseError::NotReady)?;
                slot.state = PromiseState::Vacant;
                slot.kind = PromiseType::Void;
                Ok(value)
            }
            PromiseState::Pending => Err(PromiseError::NotReady),
            PromiseState::Rejected | PromiseState::Cancelled => Err(PromiseError::AlreadyComplete),
            PromiseState::Vacant => Err(PromiseError::InvalidHandle),
        }
    }

    pub fn peek(&self, handle: PromiseHandle) -> Result<FlowValue, PromiseError> {
        let slot = self.slots.get(handle.index()).ok_or(PromiseError::InvalidHandle)?;
        match slot.state {
            PromiseState::Ready => slot.result.ok_or(PromiseError::NotReady),
            PromiseState::Pending => Err(PromiseError::NotReady),
            PromiseState::Rejected | PromiseState::Cancelled => Err(PromiseError::AlreadyComplete),
            PromiseState::Vacant => Err(PromiseError::InvalidHandle),
        }
    }

    fn reclaim_unreferenced(&mut self, referenced: &[bool; MAX_PROMISES]) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if !referenced[index] {
                *slot = PromiseSlot::EMPTY;
            }
        }
    }
}

impl Default for PromiseTable {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlowRuntime {
    pub variables: Variables,
    pub promises: PromiseTable,
    values: [Option<FlowValue>; MAX_VARIABLES],
}

impl FlowRuntime {
    pub const fn new() -> Self {
        Self {
            variables: Variables::new(),
            promises: PromiseTable::new(),
            values: [None; MAX_VARIABLES],
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn variable(&self, name: &[u8]) -> Option<FlowValue> {
        self.variables.slot(name).and_then(|slot| self.values[slot])
    }

    pub fn set_variable(&mut self, name: &[u8], value: FlowValue) -> bool {
        let Some(slot) = self.variables.slot(name) else { return false };
        self.values[slot] = Some(value);
        true
    }

    pub fn resolve(&mut self, handle: PromiseHandle, value: FlowValue) -> Result<(), PromiseError> {
        self.promises.resolve(handle, value)
    }

    pub fn cancel(&mut self, handle: PromiseHandle) -> Result<(), PromiseError> {
        self.promises.cancel(handle)
    }

    pub fn promise(&self, name: &[u8]) -> Option<(PromiseHandle, PromiseType)> {
        let value = self.variable(name)?;
        let FlowValue::Promise { handle, kind } = value else { return None };
        Some((handle, kind))
    }

    pub fn promise_state(&self, name: &[u8]) -> Option<PromiseState> {
        self.promise(name).and_then(|(handle, _)| self.promises.state(handle).ok())
    }

    pub fn resolve_variable(&mut self, name: &[u8], value: FlowValue) -> Result<(), PromiseError> {
        let (handle, _) = self.promise(name).ok_or(PromiseError::InvalidHandle)?;
        self.promises.resolve(handle, value)
    }

    pub fn take_variable(&mut self, name: &[u8]) -> Result<FlowValue, PromiseError> {
        let (handle, _) = self.promise(name).ok_or(PromiseError::InvalidHandle)?;
        self.promises.take(handle)
    }

    pub fn cancel_variable(&mut self, name: &[u8]) -> Result<(), PromiseError> {
        let (handle, _) = self.promise(name).ok_or(PromiseError::InvalidHandle)?;
        self.promises.cancel(handle)
    }

    pub fn reclaim_temporary_promises(&mut self) {
        let mut referenced = [false; MAX_PROMISES];
        for value in self.values.iter().flatten() {
            if let FlowValue::Promise { handle, .. } = value {
                if handle.index() < MAX_PROMISES {
                    referenced[handle.index()] = true;
                }
            }
        }
        self.promises.reclaim_unreferenced(&referenced);
    }
}

impl Default for FlowRuntime {
    fn default() -> Self {
        Self::new()
    }
}

// Flow values are intentionally inline and bounded; boxing would add the
// allocator dependency this service is designed to avoid.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlowEvalResult {
    Ready(FlowValue),
    Pending(PromiseHandle),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlowEvalError {
    Type(FlowTypeError),
    UnknownValue(Span),
    Unsupported(Span),
    Promise(PromiseError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TokenKind<'a> {
    Identifier(&'a [u8]),
    String(&'a [u8]),
    Number(u16),
    Var,
    Await,
    Return,
    True,
    False,
    Dot,
    Comma,
    Semicolon,
    Equals,
    Arrow,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Eof,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Token<'a> {
    kind: TokenKind<'a>,
    span: Span,
}

struct Lexer<'a> {
    source: &'a [u8],
    cursor: usize,
}

impl<'a> Lexer<'a> {
    const fn new(source: &'a [u8]) -> Self {
        Self { source, cursor: 0 }
    }

    fn next(&mut self) -> Result<Token<'a>, FlowParseError> {
        while self.cursor < self.source.len() && self.source[self.cursor].is_ascii_whitespace() {
            self.cursor += 1;
        }
        let start = self.cursor;
        let Some(byte) = self.source.get(self.cursor).copied() else {
            return Ok(Token {
                kind: TokenKind::Eof,
                span: Span { start: start as u16, end: start as u16 },
            });
        };
        self.cursor += 1;
        let kind = match byte {
            b'.' => TokenKind::Dot,
            b',' => TokenKind::Comma,
            b';' => TokenKind::Semicolon,
            b'=' => {
                if self.source.get(self.cursor) == Some(&b'>') {
                    self.cursor += 1;
                    TokenKind::Arrow
                } else {
                    TokenKind::Equals
                }
            }
            b'(' => TokenKind::LeftParen,
            b')' => TokenKind::RightParen,
            b'[' => TokenKind::LeftBracket,
            b']' => TokenKind::RightBracket,
            b'{' => TokenKind::LeftBrace,
            b'}' => TokenKind::RightBrace,
            b'"' => {
                let value_start = self.cursor;
                while self.cursor < self.source.len() && self.source[self.cursor] != b'"' {
                    if self.source[self.cursor] < 0x20 {
                        return Err(FlowParseError::InvalidToken(self.span(start)));
                    }
                    self.cursor += 1;
                }
                if self.cursor == self.source.len() {
                    return Err(FlowParseError::UnterminatedString(self.span(start)));
                }
                let value = &self.source[value_start..self.cursor];
                self.cursor += 1;
                TokenKind::String(value)
            }
            b'0'..=b'9' => {
                let mut value = u16::from(byte - b'0');
                while self.source.get(self.cursor).is_some_and(u8::is_ascii_digit) {
                    value = value
                        .checked_mul(10)
                        .and_then(|value| {
                            value.checked_add(u16::from(self.source[self.cursor] - b'0'))
                        })
                        .ok_or(FlowParseError::InvalidToken(self.span(start)))?;
                    self.cursor += 1;
                }
                TokenKind::Number(value)
            }
            byte if byte.is_ascii_alphabetic() || byte == b'_' => {
                while self.source.get(self.cursor).is_some_and(|next| {
                    next.is_ascii_alphanumeric() || *next == b'_' || *next == b'-'
                }) {
                    self.cursor += 1;
                }
                let value = &self.source[start..self.cursor];
                match value {
                    b"var" => TokenKind::Var,
                    b"await" => TokenKind::Await,
                    b"return" => TokenKind::Return,
                    b"true" => TokenKind::True,
                    b"false" => TokenKind::False,
                    _ => TokenKind::Identifier(value),
                }
            }
            _ => return Err(FlowParseError::InvalidToken(self.span(start))),
        };
        Ok(Token { kind, span: self.span(start) })
    }

    fn span(&self, start: usize) -> Span {
        Span { start: start as u16, end: self.cursor as u16 }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExprKind<'a> {
    Empty,
    String(&'a [u8]),
    Number(u16),
    Bool(bool),
    Name(&'a [u8]),
    Member { base: ExprId, name: &'a [u8] },
    Index { base: ExprId, key: ExprId },
    Call { callee: ExprId, args: [ExprId; MAX_ARGUMENTS], arg_count: usize },
    Await(ExprId),
    Arrow { parameter: &'a [u8], expression: Option<ExprId>, body_start: usize, body_len: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExprNode<'a> {
    kind: ExprKind<'a>,
    span: Span,
}

impl<'a> ExprNode<'a> {
    const EMPTY: Self = Self { kind: ExprKind::Empty, span: Span::EMPTY };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StmtKind {
    Empty,
    Var { name: usize, init: ExprId },
    Assign { name: usize, init: ExprId },
    Expression(ExprId),
    Return(ExprId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Statement<'a> {
    kind: StmtKind,
    name: Option<&'a [u8]>,
    span: Span,
    callback: bool,
}

impl<'a> Statement<'a> {
    const EMPTY: Self =
        Self { kind: StmtKind::Empty, name: None, span: Span::EMPTY, callback: false };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Program<'a> {
    source: &'a [u8],
    expressions: [ExprNode<'a>; MAX_EXPR_NODES],
    expression_len: usize,
    statements: [Statement<'a>; MAX_STATEMENTS],
    statement_len: usize,
}

impl<'a> Program<'a> {
    pub fn parse(source: &'a [u8]) -> Result<Self, FlowParseError> {
        if source.len() > MAX_SOURCE_BYTES {
            return Err(FlowParseError::TooLong);
        }
        Parser::new(source).parse()
    }

    pub const fn source(&self) -> &'a [u8] {
        self.source
    }

    pub const fn statement_count(&self) -> usize {
        self.statement_len
    }

    pub fn type_check(&self, variables: &mut Variables) -> Result<FlowType, FlowTypeError> {
        let mut checker = TypeChecker { program: self, variables: *variables };
        let result = checker.check_statements(0, self.statement_len, false)?;
        *variables = checker.variables;
        Ok(result)
    }

    pub fn evaluate(&self, runtime: &mut FlowRuntime) -> Result<FlowEvalResult, FlowEvalError> {
        let mut variables = runtime.variables;
        self.type_check(&mut variables).map_err(FlowEvalError::Type)?;
        runtime.variables = variables;
        let mut evaluator = Evaluator { program: self, runtime, callback_value: None };
        let mut result = FlowEvalResult::Ready(FlowValue::Void);
        for statement in
            self.statements[..self.statement_len].iter().filter(|statement| !statement.callback)
        {
            result = match statement.kind {
                StmtKind::Var { init, .. } | StmtKind::Assign { init, .. } => {
                    let value = evaluator.expression(init)?;
                    let name = statement.name.unwrap_or_default();
                    let slot = evaluator
                        .runtime
                        .variables
                        .slot(name)
                        .ok_or(FlowEvalError::UnknownValue(statement.span))?;
                    match value {
                        FlowEvalResult::Ready(value) => {
                            evaluator.runtime.values[slot] = Some(value);
                        }
                        FlowEvalResult::Pending(handle) => {
                            evaluator.runtime.values[slot] = Some(FlowValue::Promise {
                                handle,
                                kind: evaluator
                                    .runtime
                                    .promises
                                    .kind(handle)
                                    .map_err(FlowEvalError::Promise)?,
                            });
                        }
                    }
                    FlowEvalResult::Ready(FlowValue::Void)
                }
                StmtKind::Expression(expression) => evaluator.expression(expression)?,
                StmtKind::Empty | StmtKind::Return(_) => FlowEvalResult::Ready(FlowValue::Void),
            };
        }
        Ok(result)
    }

    fn expression(&self, id: ExprId) -> &ExprNode<'a> {
        &self.expressions[usize::from(id.0)]
    }
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    lookahead: Option<Token<'a>>,
    source: &'a [u8],
    expressions: [ExprNode<'a>; MAX_EXPR_NODES],
    expression_len: usize,
    statements: [Statement<'a>; MAX_STATEMENTS],
    statement_len: usize,
    callback_depth: u8,
}

impl<'a> Parser<'a> {
    fn new(source: &'a [u8]) -> Self {
        Self {
            lexer: Lexer::new(source),
            lookahead: None,
            source,
            expressions: [ExprNode::EMPTY; MAX_EXPR_NODES],
            expression_len: 0,
            statements: [Statement::EMPTY; MAX_STATEMENTS],
            statement_len: 0,
            callback_depth: 0,
        }
    }

    fn parse(mut self) -> Result<Program<'a>, FlowParseError> {
        while !matches!(self.peek()?.kind, TokenKind::Eof) {
            self.parse_statement()?;
            if matches!(self.peek()?.kind, TokenKind::Semicolon) {
                self.take()?;
            } else if !matches!(self.peek()?.kind, TokenKind::Eof | TokenKind::RightBrace) {
                return Err(FlowParseError::UnexpectedToken(self.peek()?.span));
            }
        }
        Ok(Program {
            source: self.source,
            expressions: self.expressions,
            expression_len: self.expression_len,
            statements: self.statements,
            statement_len: self.statement_len,
        })
    }

    fn parse_statement(&mut self) -> Result<(), FlowParseError> {
        let token = self.peek()?;
        let (kind, name) = match token.kind {
            TokenKind::Var => {
                self.take()?;
                let name_token = self.take()?;
                let TokenKind::Identifier(name) = name_token.kind else {
                    return Err(FlowParseError::UnexpectedToken(name_token.span));
                };
                self.expect(TokenKind::Equals)?;
                (StmtKind::Var { name: 0, init: self.parse_expression()? }, Some(name))
            }
            TokenKind::Return => {
                self.take()?;
                (StmtKind::Return(self.parse_expression()?), None)
            }
            TokenKind::Identifier(name) => {
                self.take()?;
                if matches!(self.peek()?.kind, TokenKind::Equals) {
                    self.take()?;
                    (StmtKind::Assign { name: 0, init: self.parse_expression()? }, Some(name))
                } else {
                    let id = self.add(ExprKind::Name(name), token.span)?;
                    let id = self.parse_postfix(id)?;
                    (StmtKind::Expression(id), None)
                }
            }
            _ => (StmtKind::Expression(self.parse_expression()?), None),
        };
        if self.statement_len == MAX_STATEMENTS {
            return Err(FlowParseError::TooManyStatements);
        }
        let index = self.statement_len;
        let kind = match (kind, name) {
            (StmtKind::Var { init, .. }, Some(_name)) => StmtKind::Var { name: index, init },
            (StmtKind::Assign { init, .. }, Some(_name)) => StmtKind::Assign { name: index, init },
            (kind, _) => kind,
        };
        self.statements[index] =
            Statement { kind, name, span: token.span, callback: self.callback_depth != 0 };
        self.statement_len += 1;
        Ok(())
    }

    fn parse_expression(&mut self) -> Result<ExprId, FlowParseError> {
        if matches!(self.peek()?.kind, TokenKind::Await) {
            let token = self.take()?;
            let expression = self.parse_expression()?;
            return self.add(ExprKind::Await(expression), token.span);
        }
        let primary = self.parse_primary()?;
        self.parse_postfix(primary)
    }

    fn parse_primary(&mut self) -> Result<ExprId, FlowParseError> {
        let token = self.take()?;
        match token.kind {
            TokenKind::String(value) => self.add(ExprKind::String(value), token.span),
            TokenKind::Number(value) => self.add(ExprKind::Number(value), token.span),
            TokenKind::True => self.add(ExprKind::Bool(true), token.span),
            TokenKind::False => self.add(ExprKind::Bool(false), token.span),
            TokenKind::Identifier(name) => self.add(ExprKind::Name(name), token.span),
            TokenKind::LeftParen => self.parse_parenthesized(token.span),
            _ => Err(FlowParseError::MissingExpression(token.span)),
        }
    }

    fn parse_parenthesized(&mut self, span: Span) -> Result<ExprId, FlowParseError> {
        let save = self.clone_state();
        if let TokenKind::Identifier(parameter) = self.peek()?.kind {
            self.take()?;
            if matches!(self.peek()?.kind, TokenKind::RightParen) {
                self.take()?;
                if matches!(self.peek()?.kind, TokenKind::Arrow) {
                    self.take()?;
                    return self.parse_arrow(parameter, span);
                }
            }
        }
        self.restore_state(save);
        let expression = self.parse_expression()?;
        self.expect(TokenKind::RightParen)?;
        Ok(expression)
    }

    fn parse_arrow(&mut self, parameter: &'a [u8], span: Span) -> Result<ExprId, FlowParseError> {
        if usize::from(self.callback_depth) >= MAX_CALLBACK_DEPTH {
            return Err(FlowParseError::CallbackLimit(span));
        }
        self.callback_depth = self.callback_depth.saturating_add(1);
        if matches!(self.peek()?.kind, TokenKind::LeftBrace) {
            self.take()?;
            let body_start = self.statement_len;
            while !matches!(self.peek()?.kind, TokenKind::RightBrace | TokenKind::Eof) {
                self.parse_statement()?;
                if matches!(self.peek()?.kind, TokenKind::Semicolon) {
                    self.take()?;
                } else if !matches!(self.peek()?.kind, TokenKind::RightBrace) {
                    return Err(FlowParseError::UnexpectedToken(self.peek()?.span));
                }
            }
            self.callback_depth = self.callback_depth.saturating_sub(1);
            self.expect(TokenKind::RightBrace)?;
            return self.add(
                ExprKind::Arrow {
                    parameter,
                    expression: None,
                    body_start,
                    body_len: self.statement_len - body_start,
                },
                span,
            );
        }
        let expression = self.parse_expression()?;
        self.callback_depth = self.callback_depth.saturating_sub(1);
        self.add(
            ExprKind::Arrow { parameter, expression: Some(expression), body_start: 0, body_len: 0 },
            span,
        )
    }

    fn parse_postfix(&mut self, mut expression: ExprId) -> Result<ExprId, FlowParseError> {
        loop {
            match self.peek()?.kind {
                TokenKind::Dot => {
                    self.take()?;
                    let member = self.take()?;
                    let TokenKind::Identifier(name) = member.kind else {
                        return Err(FlowParseError::UnexpectedToken(member.span));
                    };
                    expression =
                        self.add(ExprKind::Member { base: expression, name }, member.span)?;
                }
                TokenKind::LeftBracket => {
                    self.take()?;
                    let key = self.parse_expression()?;
                    let close = self.expect(TokenKind::RightBracket)?;
                    expression = self.add(ExprKind::Index { base: expression, key }, close.span)?;
                }
                TokenKind::LeftParen => {
                    self.take()?;
                    let mut args = [ExprId(0); MAX_ARGUMENTS];
                    let mut arg_count = 0;
                    if !matches!(self.peek()?.kind, TokenKind::RightParen) {
                        loop {
                            if arg_count == MAX_ARGUMENTS {
                                return Err(FlowParseError::TooManyArguments(self.peek()?.span));
                            }
                            args[arg_count] = self.parse_expression()?;
                            arg_count += 1;
                            if !matches!(self.peek()?.kind, TokenKind::Comma) {
                                break;
                            }
                            self.take()?;
                        }
                    }
                    let close = self.expect(TokenKind::RightParen)?;
                    expression = self
                        .add(ExprKind::Call { callee: expression, args, arg_count }, close.span)?;
                }
                _ => break,
            }
        }
        Ok(expression)
    }

    fn add(&mut self, kind: ExprKind<'a>, span: Span) -> Result<ExprId, FlowParseError> {
        if self.expression_len == MAX_EXPR_NODES {
            return Err(FlowParseError::TooManyExpressions);
        }
        let id = ExprId(self.expression_len as u8);
        self.expressions[self.expression_len] = ExprNode { kind, span };
        self.expression_len += 1;
        Ok(id)
    }

    fn peek(&mut self) -> Result<Token<'a>, FlowParseError> {
        if self.lookahead.is_none() {
            self.lookahead = Some(self.lexer.next()?);
        }
        Ok(self.lookahead.unwrap_or(Token { kind: TokenKind::Eof, span: Span::EMPTY }))
    }

    fn take(&mut self) -> Result<Token<'a>, FlowParseError> {
        let token = self.peek()?;
        self.lookahead = None;
        Ok(token)
    }

    fn expect(&mut self, expected: TokenKind<'a>) -> Result<Token<'a>, FlowParseError> {
        let token = self.take()?;
        if core::mem::discriminant(&token.kind) != core::mem::discriminant(&expected) {
            return Err(FlowParseError::UnexpectedToken(token.span));
        }
        Ok(token)
    }

    fn clone_state(&self) -> (usize, Option<Token<'a>>, usize, usize) {
        (self.lexer.cursor, self.lookahead, self.expression_len, self.statement_len)
    }

    fn restore_state(&mut self, state: (usize, Option<Token<'a>>, usize, usize)) {
        self.lexer.cursor = state.0;
        self.lookahead = state.1;
        self.expression_len = state.2;
        self.statement_len = state.3;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Variables {
    entries: [Variable; MAX_VARIABLES],
    len: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Variable {
    name: [u8; MAX_VARIABLE_NAME_BYTES],
    name_len: usize,
    kind: FlowType,
}

impl Variable {
    const EMPTY: Self =
        Self { name: [0; MAX_VARIABLE_NAME_BYTES], name_len: 0, kind: FlowType::Void };
}

impl Variables {
    pub const fn new() -> Self {
        Self { entries: [Variable::EMPTY; MAX_VARIABLES], len: 0 }
    }

    fn find(&self, name: &[u8]) -> Option<FlowType> {
        self.entries[..self.len]
            .iter()
            .find(|entry| &entry.name[..entry.name_len] == name)
            .map(|entry| entry.kind)
    }

    fn slot(&self, name: &[u8]) -> Option<usize> {
        self.entries[..self.len].iter().position(|entry| &entry.name[..entry.name_len] == name)
    }

    fn declare(&mut self, name: &[u8], kind: FlowType, span: Span) -> Result<(), FlowTypeError> {
        if name.is_empty() || name.len() > MAX_VARIABLE_NAME_BYTES {
            return Err(FlowTypeError::VariableLimit(span));
        }
        if self.find(name).is_some() {
            return self.assign(name, kind, span);
        }
        if self.len == MAX_VARIABLES {
            return Err(FlowTypeError::VariableLimit(span));
        }
        let mut entry = Variable::EMPTY;
        entry.name[..name.len()].copy_from_slice(name);
        entry.name_len = name.len();
        entry.kind = kind;
        self.entries[self.len] = entry;
        self.len += 1;
        Ok(())
    }

    fn assign(&mut self, name: &[u8], kind: FlowType, span: Span) -> Result<(), FlowTypeError> {
        let Some(entry) =
            self.entries[..self.len].iter_mut().find(|entry| &entry.name[..entry.name_len] == name)
        else {
            return Err(FlowTypeError::UnknownVariable(span));
        };
        if entry.kind != kind {
            return Err(FlowTypeError::Expected { span, expected: entry.kind, actual: kind });
        }
        entry.kind = kind;
        Ok(())
    }
}

impl Default for Variables {
    fn default() -> Self {
        Self::new()
    }
}

struct TypeChecker<'a, 'p> {
    program: &'p Program<'a>,
    variables: Variables,
}

impl<'a, 'p> TypeChecker<'a, 'p> {
    fn check_statements(
        &mut self,
        start: usize,
        count: usize,
        callback: bool,
    ) -> Result<FlowType, FlowTypeError> {
        let mut result = FlowType::Void;
        for statement in self.program.statements[start..start + count].iter() {
            if statement.callback != callback {
                continue;
            }
            result = match statement.kind {
                StmtKind::Empty => FlowType::Void,
                StmtKind::Var { init, .. } => {
                    let kind = self.expr_type(init)?;
                    self.variables.declare(
                        statement.name.unwrap_or_default(),
                        kind,
                        statement.span,
                    )?;
                    FlowType::Void
                }
                StmtKind::Assign { init, .. } => {
                    let kind = self.expr_type(init)?;
                    self.variables.assign(
                        statement.name.unwrap_or_default(),
                        kind,
                        statement.span,
                    )?;
                    FlowType::Void
                }
                StmtKind::Expression(expression) => self.expr_type(expression)?,
                StmtKind::Return(expression) if callback => self.expr_type(expression)?,
                StmtKind::Return(_) => return Err(FlowTypeError::InvalidReturn(statement.span)),
            };
        }
        Ok(result)
    }

    fn expr_type(&mut self, id: ExprId) -> Result<FlowType, FlowTypeError> {
        let node = self.program.expression(id);
        match node.kind {
            ExprKind::Empty => Err(FlowTypeError::InvalidCall(node.span)),
            ExprKind::String(_) => Ok(FlowType::String),
            ExprKind::Number(_) => Ok(FlowType::Number),
            ExprKind::Bool(_) => Ok(FlowType::Bool),
            ExprKind::Name(name) => match self.variables.find(name) {
                Some(kind) => Ok(kind),
                None if name == b"fs" => Ok(FlowType::Namespace(NamespaceKind::Filesystem)),
                None if name == b"net" => Ok(FlowType::Namespace(NamespaceKind::Network)),
                None if name == b"sys" => Ok(FlowType::Namespace(NamespaceKind::System)),
                None if name == b"service" => Ok(FlowType::Namespace(NamespaceKind::Supervisor)),
                None if name == b"pkg" => Ok(FlowType::Namespace(NamespaceKind::Package)),
                None if name == b"program" => Ok(FlowType::Namespace(NamespaceKind::Program)),
                None if name == b"device" => Ok(FlowType::Namespace(NamespaceKind::Device)),
                None => Err(FlowTypeError::UnknownVariable(node.span)),
            },
            ExprKind::Index { base, key } => {
                let key_kind = self.expr_type(key)?;
                if let ExprKind::Member { base: namespace, name } =
                    self.program.expression(base).kind
                {
                    if name == b"interface"
                        && self.expr_type(namespace)? == FlowType::Namespace(NamespaceKind::Network)
                        && key_kind == FlowType::String
                    {
                        return Ok(FlowType::Service);
                    }
                }
                let base_kind = self.expr_type(base)?;
                if matches!(
                    base_kind,
                    FlowType::Namespace(NamespaceKind::Supervisor) | FlowType::Service
                ) && key_kind == FlowType::String
                {
                    Ok(FlowType::Service)
                } else {
                    Err(FlowTypeError::UnknownMember(node.span))
                }
            }
            ExprKind::Member { base, name } => {
                let base_kind = self.expr_type(base)?;
                match (base_kind, name) {
                    (FlowType::Response, b"body") => Ok(FlowType::Bytes),
                    (FlowType::Response, b"status") => Ok(FlowType::Number),
                    (FlowType::Namespace(NamespaceKind::Network), b"status") => {
                        Ok(FlowType::Callback)
                    }
                    (FlowType::Promise(_), b"then" | b"cancel") => Ok(FlowType::Callback),
                    (FlowType::FileHandle, b"create" | b"read" | b"write") => {
                        Ok(FlowType::Callback)
                    }
                    (FlowType::Service, name)
                        if OperationRegistry::lookup(NamespaceKind::Supervisor, name).is_some() =>
                    {
                        Ok(FlowType::Callback)
                    }
                    (FlowType::Namespace(_), _) => Err(FlowTypeError::UnknownMember(node.span)),
                    _ => Err(FlowTypeError::UnknownMember(node.span)),
                }
            }
            ExprKind::Call { callee, args, arg_count } => {
                self.call_type(callee, &args[..arg_count], node.span)
            }
            ExprKind::Await(expression) => match self.expr_type(expression)? {
                FlowType::Promise(kind) => Ok(kind.value_type()),
                _ => Err(FlowTypeError::ExpectedPromise(node.span)),
            },
            ExprKind::Arrow { .. } => Ok(FlowType::Callback),
        }
    }

    fn call_type(
        &mut self,
        callee: ExprId,
        args: &[ExprId],
        span: Span,
    ) -> Result<FlowType, FlowTypeError> {
        let callee_node = self.program.expression(callee);
        if let ExprKind::Member { base, name } = callee_node.kind {
            let base_kind = self.expr_type(base)?;
            if let FlowType::Namespace(namespace) = base_kind {
                if OperationRegistry::lookup(namespace, name).is_none() {
                    return Err(FlowTypeError::UnknownMember(span));
                }
            }
            if let FlowType::Service = base_kind {
                if OperationRegistry::lookup(NamespaceKind::Supervisor, name).is_none() {
                    return Err(FlowTypeError::UnknownMember(span));
                }
            }
            match (base_kind, name) {
                (FlowType::Namespace(NamespaceKind::Filesystem), b"open" | b"touch") => {
                    Self::require_arity(args, 1, span)?;
                    Self::require_type(self.expr_type(args[0])?, FlowType::String, span)?;
                    Ok(FlowType::FileHandle)
                }
                (FlowType::Namespace(NamespaceKind::Filesystem), b"list") => {
                    if args.len() > 1 {
                        return Err(FlowTypeError::WrongArity(span));
                    }
                    if let Some(path) = args.first() {
                        Self::require_type(self.expr_type(*path)?, FlowType::String, span)?;
                    }
                    Ok(PromiseType::String.flow_type())
                }
                (FlowType::Namespace(NamespaceKind::Filesystem), b"remove") => {
                    Self::require_arity(args, 1, span)?;
                    Self::require_type(self.expr_type(args[0])?, FlowType::String, span)?;
                    Ok(PromiseType::Void.flow_type())
                }
                (FlowType::Namespace(NamespaceKind::Filesystem), b"move") => {
                    Self::require_arity(args, 2, span)?;
                    Self::require_type(self.expr_type(args[0])?, FlowType::String, span)?;
                    Self::require_type(self.expr_type(args[1])?, FlowType::String, span)?;
                    Ok(PromiseType::Void.flow_type())
                }
                (FlowType::Namespace(NamespaceKind::Network), b"fetch") => {
                    if !(args.len() == 1 || args.len() == 2) {
                        return Err(FlowTypeError::WrongArity(span));
                    }
                    Self::require_type(self.expr_type(args[0])?, FlowType::String, span)?;
                    if args.len() == 2 {
                        Self::require_type(self.expr_type(args[1])?, FlowType::String, span)?;
                    }
                    Ok(PromiseType::Response.flow_type())
                }
                (FlowType::Namespace(NamespaceKind::System), b"version" | b"uname") => {
                    Self::require_arity(args, 0, span)?;
                    Ok(PromiseType::String.flow_type())
                }
                (FlowType::Namespace(NamespaceKind::Network), b"status") => {
                    Self::require_arity(args, 0, span)?;
                    Ok(PromiseType::String.flow_type())
                }
                (FlowType::Namespace(NamespaceKind::System), b"shutdown" | b"reboot") => {
                    Self::require_arity(args, 0, span)?;
                    Ok(PromiseType::Void.flow_type())
                }
                (FlowType::Namespace(NamespaceKind::Network), b"ping") => {
                    Self::require_arity(args, 1, span)?;
                    Self::require_type(self.expr_type(args[0])?, FlowType::String, span)?;
                    Ok(PromiseType::String.flow_type())
                }
                (FlowType::Namespace(NamespaceKind::Network), b"tcp-probe") => {
                    Self::require_arity(args, 2, span)?;
                    Self::require_type(self.expr_type(args[0])?, FlowType::String, span)?;
                    Self::require_type(self.expr_type(args[1])?, FlowType::Number, span)?;
                    Ok(PromiseType::String.flow_type())
                }
                (FlowType::Namespace(NamespaceKind::Package), b"list") => {
                    Self::require_arity(args, 0, span)?;
                    Ok(PromiseType::String.flow_type())
                }
                (FlowType::Namespace(NamespaceKind::Package), b"info") => {
                    Self::require_arity(args, 1, span)?;
                    Self::require_type(self.expr_type(args[0])?, FlowType::String, span)?;
                    Ok(PromiseType::String.flow_type())
                }
                (FlowType::Namespace(NamespaceKind::Package), b"install") => {
                    Self::require_arity(args, 1, span)?;
                    Self::require_type(self.expr_type(args[0])?, FlowType::String, span)?;
                    Ok(PromiseType::String.flow_type())
                }
                (FlowType::Namespace(NamespaceKind::Program), b"start" | b"status" | b"stop") => {
                    Self::require_arity(args, 1, span)?;
                    Self::require_type(self.expr_type(args[0])?, FlowType::String, span)?;
                    Ok(PromiseType::String.flow_type())
                }
                (FlowType::Namespace(NamespaceKind::Device), b"list") => {
                    Self::require_arity(args, 0, span)?;
                    Ok(PromiseType::String.flow_type())
                }
                (FlowType::FileHandle, b"create") => {
                    Self::require_arity(args, 0, span)?;
                    Ok(PromiseType::Void.flow_type())
                }
                (FlowType::FileHandle, b"read") => {
                    Self::require_arity(args, 0, span)?;
                    Ok(PromiseType::Bytes.flow_type())
                }
                (FlowType::FileHandle, b"write") => {
                    Self::require_arity(args, 1, span)?;
                    let actual = self.expr_type(args[0])?;
                    if !matches!(actual, FlowType::String | FlowType::Bytes) {
                        return Err(FlowTypeError::Expected {
                            span,
                            expected: FlowType::Bytes,
                            actual,
                        });
                    }
                    Ok(PromiseType::Void.flow_type())
                }
                (FlowType::Promise(result), b"cancel") => {
                    Self::require_arity(args, 0, span)?;
                    let _ = result;
                    Ok(PromiseType::Void.flow_type())
                }
                (FlowType::Promise(result), b"then") => {
                    Self::require_arity(args, 1, span)?;
                    let callback = self.program.expression(args[0]);
                    let ExprKind::Arrow { parameter, expression, body_start, body_len } =
                        callback.kind
                    else {
                        return Err(FlowTypeError::Expected {
                            span,
                            expected: FlowType::Callback,
                            actual: self.expr_type(args[0])?,
                        });
                    };
                    let mut callback_checker =
                        TypeChecker { program: self.program, variables: self.variables };
                    callback_checker.variables.declare(
                        parameter,
                        result.value_type(),
                        callback.span,
                    )?;
                    let callback_type = if let Some(expression) = expression {
                        callback_checker.expr_type(expression)?
                    } else {
                        callback_checker.check_statements(body_start, body_len, true)?
                    };
                    match callback_type {
                        FlowType::Promise(kind) => Ok(kind.flow_type()),
                        FlowType::Void => Ok(PromiseType::Void.flow_type()),
                        FlowType::Bool => Ok(PromiseType::Bool.flow_type()),
                        FlowType::Number => Ok(PromiseType::Number.flow_type()),
                        FlowType::String => Ok(PromiseType::String.flow_type()),
                        FlowType::Bytes => Ok(PromiseType::Bytes.flow_type()),
                        FlowType::Response => Ok(PromiseType::Response.flow_type()),
                        FlowType::FileHandle => Ok(PromiseType::FileHandle.flow_type()),
                        _ => Err(FlowTypeError::InvalidCall(span)),
                    }
                }
                (FlowType::Service, b"status" | b"name" | b"version") => {
                    Self::require_arity(args, 0, span)?;
                    Ok(PromiseType::String.flow_type())
                }
                (FlowType::Service, b"start" | b"stop" | b"restart") => {
                    Self::require_arity(args, 0, span)?;
                    Ok(PromiseType::Void.flow_type())
                }
                _ => Err(FlowTypeError::InvalidCall(span)),
            }
        } else if let ExprKind::Name(name) = callee_node.kind {
            if name == b"clear" {
                Self::require_arity(args, 0, span)?;
                return Ok(FlowType::Void);
            }
            if name == b"echo" {
                Self::require_arity(args, 1, span)?;
                Self::require_type(self.expr_type(args[0])?, FlowType::String, span)?;
                return Ok(FlowType::String);
            }
            if name != b"help" {
                return Err(FlowTypeError::InvalidCall(span));
            }
            if args.len() > 1 {
                return Err(FlowTypeError::WrongArity(span));
            }
            if let Some(topic) = args.first() {
                Self::require_type(self.expr_type(*topic)?, FlowType::String, span)?;
            }
            Ok(FlowType::String)
        } else {
            Err(FlowTypeError::InvalidCall(span))
        }
    }

    fn require_arity(args: &[ExprId], expected: usize, span: Span) -> Result<(), FlowTypeError> {
        if args.len() == expected { Ok(()) } else { Err(FlowTypeError::WrongArity(span)) }
    }

    fn require_type(actual: FlowType, expected: FlowType, span: Span) -> Result<(), FlowTypeError> {
        if actual == expected {
            Ok(())
        } else {
            Err(FlowTypeError::Expected { span, expected, actual })
        }
    }
}

struct Evaluator<'a, 'p> {
    program: &'p Program<'a>,
    runtime: &'p mut FlowRuntime,
    callback_value: Option<(&'a [u8], FlowValue)>,
}

impl<'a, 'p> Evaluator<'a, 'p> {
    fn type_checker(&self) -> TypeChecker<'a, 'p> {
        let mut variables = self.runtime.variables;
        if let Some((parameter, value)) = self.callback_value {
            let _ = variables.declare(parameter, value.kind(), Span::EMPTY);
        }
        TypeChecker { program: self.program, variables }
    }

    fn expression(&mut self, id: ExprId) -> Result<FlowEvalResult, FlowEvalError> {
        let node = self.program.expression(id);
        match node.kind {
            ExprKind::String(bytes) => Ok(FlowEvalResult::Ready(
                FlowValue::string(bytes).ok_or(FlowEvalError::Unsupported(node.span))?,
            )),
            ExprKind::Number(value) => Ok(FlowEvalResult::Ready(FlowValue::Number(value))),
            ExprKind::Bool(value) => Ok(FlowEvalResult::Ready(FlowValue::Bool(value))),
            ExprKind::Name(name) => self
                .callback_value
                .filter(|(parameter, _)| *parameter == name)
                .map(|(_, value)| FlowEvalResult::Ready(value))
                .or_else(|| self.runtime.variable(name).map(FlowEvalResult::Ready))
                .ok_or(FlowEvalError::UnknownValue(node.span)),
            ExprKind::Await(expression) => {
                let value = self.expression(expression)?;
                let FlowEvalResult::Ready(FlowValue::Promise { handle, .. }) = value else {
                    return Err(FlowEvalError::Type(FlowTypeError::ExpectedPromise(node.span)));
                };
                match self.runtime.promises.peek(handle) {
                    Ok(value) => Ok(FlowEvalResult::Ready(value)),
                    Err(PromiseError::NotReady) => Ok(FlowEvalResult::Pending(handle)),
                    Err(error) => Err(FlowEvalError::Promise(error)),
                }
            }
            ExprKind::Call { callee, args, arg_count } => {
                self.call(id, callee, &args[..arg_count], node.span)
            }
            ExprKind::Member { base, name } => {
                let FlowEvalResult::Ready(value) = self.expression(base)? else {
                    return Err(FlowEvalError::Unsupported(node.span));
                };
                match (value, name) {
                    (FlowValue::Response { status, .. }, b"status") => {
                        Ok(FlowEvalResult::Ready(FlowValue::Number(status)))
                    }
                    (FlowValue::Response { body, body_len, .. }, b"body") => {
                        Ok(FlowEvalResult::Ready(FlowValue::Bytes { bytes: body, len: body_len }))
                    }
                    _ => Err(FlowEvalError::Unsupported(node.span)),
                }
            }
            ExprKind::Index { .. } | ExprKind::Arrow { .. } | ExprKind::Empty => {
                Err(FlowEvalError::Unsupported(node.span))
            }
        }
    }

    fn call(
        &mut self,
        call_id: ExprId,
        callee: ExprId,
        args: &[ExprId],
        span: Span,
    ) -> Result<FlowEvalResult, FlowEvalError> {
        let callee_node = self.program.expression(callee);
        if let ExprKind::Name(name) = callee_node.kind {
            if name == b"clear" {
                let mut checker = self.type_checker();
                checker.expr_type(call_id).map_err(FlowEvalError::Type)?;
                return Ok(FlowEvalResult::Ready(FlowValue::Void));
            }
            if name == b"echo" {
                let mut checker = self.type_checker();
                checker.expr_type(call_id).map_err(FlowEvalError::Type)?;
                let FlowEvalResult::Ready(FlowValue::String { bytes, len }) =
                    self.expression(args[0])?
                else {
                    return Err(FlowEvalError::Unsupported(span));
                };
                return Ok(FlowEvalResult::Ready(
                    FlowValue::string(&bytes[..len]).ok_or(FlowEvalError::Unsupported(span))?,
                ));
            }
            if name == b"help" {
                let mut checker = self.type_checker();
                checker.expr_type(call_id).map_err(FlowEvalError::Type)?;
                let mut topic = [0; MAX_VALUE_BYTES];
                let topic_len = if let Some(argument) = args.first() {
                    let FlowEvalResult::Ready(FlowValue::String { bytes, len }) =
                        self.expression(*argument)?
                    else {
                        return Err(FlowEvalError::Unsupported(span));
                    };
                    topic[..len].copy_from_slice(&bytes[..len]);
                    len
                } else {
                    0
                };
                let mut output = [0; super::MAX_OUTPUT_BYTES];
                let rendered = if args.is_empty() {
                    super::format_help(None, &mut output)
                } else {
                    super::format_help(Some(&topic[..topic_len]), &mut output)
                };
                return Ok(FlowEvalResult::Ready(
                    FlowValue::string(&output[..rendered])
                        .ok_or(FlowEvalError::Unsupported(span))?,
                ));
            }
        }
        if let ExprKind::Member { base, name } = callee_node.kind {
            if matches!(name, b"open" | b"touch") {
                let mut checker = self.type_checker();
                checker.expr_type(call_id).map_err(FlowEvalError::Type)?;
                let FlowEvalResult::Ready(FlowValue::String { bytes, len }) =
                    self.expression(args[0])?
                else {
                    return Err(FlowEvalError::Unsupported(span));
                };
                if len > MAX_FILE_PATH_BYTES {
                    return Err(FlowEvalError::Unsupported(span));
                }
                let mut path = [0; MAX_FILE_PATH_BYTES];
                path[..len].copy_from_slice(&bytes[..len]);
                return Ok(FlowEvalResult::Ready(FlowValue::FileHandle {
                    path,
                    path_len: len,
                    create: name == b"touch",
                }));
            }
            if name == b"cancel" {
                if let FlowEvalResult::Ready(FlowValue::Promise { handle, .. }) =
                    self.expression(base)?
                {
                    self.runtime.cancel(handle).map_err(FlowEvalError::Promise)?;
                    let output = self
                        .runtime
                        .promises
                        .allocate(PromiseType::Void)
                        .map_err(FlowEvalError::Promise)?;
                    self.runtime
                        .promises
                        .resolve(output, FlowValue::Void)
                        .map_err(FlowEvalError::Promise)?;
                    return Ok(FlowEvalResult::Ready(FlowValue::Promise {
                        handle: output,
                        kind: PromiseType::Void,
                    }));
                }
            }
            if name == b"then" && args.len() == 1 {
                if let FlowEvalResult::Ready(FlowValue::Promise { handle, .. }) =
                    self.expression(base)?
                {
                    let callback = self.program.expression(args[0]);
                    let ExprKind::Arrow { parameter, expression, body_start, body_len } =
                        callback.kind
                    else {
                        return Err(FlowEvalError::Unsupported(callback.span));
                    };
                    if self.runtime.promises.state(handle).map_err(FlowEvalError::Promise)?
                        == PromiseState::Ready
                    {
                        let value =
                            self.runtime.promises.peek(handle).map_err(FlowEvalError::Promise)?;
                        self.callback_value = Some((parameter, value));
                        let callback_result = if let Some(expression) = expression {
                            self.expression(expression)?
                        } else {
                            let mut result = FlowEvalResult::Ready(FlowValue::Void);
                            for statement in
                                self.program.statements[body_start..body_start + body_len].iter()
                            {
                                if let StmtKind::Return(expression) = statement.kind {
                                    result = self.expression(expression)?;
                                    break;
                                }
                            }
                            result
                        };
                        self.callback_value = None;
                        return match callback_result {
                            FlowEvalResult::Pending(handle) => {
                                let kind = self
                                    .runtime
                                    .promises
                                    .kind(handle)
                                    .map_err(FlowEvalError::Promise)?;
                                Ok(FlowEvalResult::Ready(FlowValue::Promise { handle, kind }))
                            }
                            FlowEvalResult::Ready(value) => {
                                if let FlowValue::Promise { handle, kind } = value {
                                    return Ok(FlowEvalResult::Ready(FlowValue::Promise {
                                        handle,
                                        kind,
                                    }));
                                }
                                let promise_kind = promise_type(value.kind())
                                    .ok_or(FlowEvalError::Unsupported(span))?;
                                let output = self
                                    .runtime
                                    .promises
                                    .allocate(promise_kind)
                                    .map_err(FlowEvalError::Promise)?;
                                self.runtime
                                    .promises
                                    .resolve(output, value)
                                    .map_err(FlowEvalError::Promise)?;
                                Ok(FlowEvalResult::Ready(FlowValue::Promise {
                                    handle: output,
                                    kind: promise_kind,
                                }))
                            }
                        };
                    }
                }
            }
        }
        let mut checker = self.type_checker();
        let kind = checker.expr_type(call_id).map_err(FlowEvalError::Type)?;
        let FlowType::Promise(promise) = kind else {
            return Err(FlowEvalError::Unsupported(span));
        };
        let handle = self.runtime.promises.allocate(promise).map_err(FlowEvalError::Promise)?;
        Ok(FlowEvalResult::Ready(FlowValue::Promise { handle, kind: promise }))
    }
}

fn promise_type(kind: FlowType) -> Option<PromiseType> {
    match kind {
        FlowType::Void => Some(PromiseType::Void),
        FlowType::Bool => Some(PromiseType::Bool),
        FlowType::Number => Some(PromiseType::Number),
        FlowType::String => Some(PromiseType::String),
        FlowType::Bytes => Some(PromiseType::Bytes),
        FlowType::Response => Some(PromiseType::Response),
        FlowType::FileHandle => Some(PromiseType::FileHandle),
        _ => None,
    }
}

pub fn format_diagnostic(error: FlowTypeError, output: &mut [u8]) -> usize {
    let message = match error {
        FlowTypeError::UnknownVariable(_) => b"flow: unknown variable" as &[u8],
        FlowTypeError::UnknownMember(_) => b"flow: unknown member",
        FlowTypeError::InvalidCall(_) => b"flow: invalid call",
        FlowTypeError::WrongArity(_) => b"flow: wrong argument count",
        FlowTypeError::Expected { .. } => b"flow: type mismatch",
        FlowTypeError::ExpectedPromise(_) => b"flow: await expects a promise",
        FlowTypeError::InvalidReturn(_) => b"flow: return is only valid inside a callback",
        FlowTypeError::VariableLimit(_) => b"flow: variable limit reached",
    };
    let count = message.len().min(output.len());
    output[..count].copy_from_slice(&message[..count]);
    let span = match error {
        FlowTypeError::UnknownVariable(span)
        | FlowTypeError::UnknownMember(span)
        | FlowTypeError::InvalidCall(span)
        | FlowTypeError::WrongArity(span)
        | FlowTypeError::ExpectedPromise(span)
        | FlowTypeError::InvalidReturn(span)
        | FlowTypeError::VariableLimit(span) => span,
        FlowTypeError::Expected { span, .. } => span,
    };
    let mut length = count;
    for byte in b" @ " {
        if length == output.len() {
            return length;
        }
        output[length] = *byte;
        length += 1;
    }
    length = append_number(span.start, output, length);
    if length < output.len() {
        output[length] = b'-';
        length += 1;
    }
    length = append_number(span.end, output, length);
    length
}

fn append_number(value: u16, output: &mut [u8], mut length: usize) -> usize {
    let mut digits = [0; 5];
    let mut count = 0;
    let mut value = value;
    loop {
        digits[count] = b'0' + (value % 10) as u8;
        count += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    while count > 0 && length < output.len() {
        count -= 1;
        output[length] = digits[count];
        length += 1;
    }
    length
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_type_checks_typed_file_chain() {
        let source = br#"await net.fetch("http://10.0.2.2/readme").then((response) => {
            return fs.touch("/download").write(response.body);
        })"#;
        let program = Program::parse(source).unwrap();
        let mut variables = Variables::new();
        assert_eq!(program.type_check(&mut variables), Ok(FlowType::Void));
    }

    #[test]
    fn persistent_variables_keep_inferred_promise_type() {
        let program =
            Program::parse(br#"var response = net.fetch("http://10.0.2.2/readme")"#).unwrap();
        let mut variables = Variables::new();
        assert_eq!(program.type_check(&mut variables), Ok(FlowType::Void));
        let second = Program::parse(b"await response").unwrap();
        assert_eq!(second.type_check(&mut variables), Ok(FlowType::Response));
    }

    #[test]
    fn rejects_awaiting_non_promise() {
        let program = Program::parse(b"await fs.open(\"/file\")").unwrap();
        let mut variables = Variables::new();
        assert!(matches!(
            program.type_check(&mut variables),
            Err(FlowTypeError::ExpectedPromise(_))
        ));
    }

    #[test]
    fn rejects_more_than_fixed_arguments() {
        assert!(matches!(
            Program::parse(b"net.fetch(\"a\", \"b\", \"c\", \"d\")"),
            Err(FlowParseError::TooManyArguments(_))
        ));
    }

    #[test]
    fn rejects_overlong_source() {
        assert_eq!(Program::parse(&[b'x'; MAX_SOURCE_BYTES + 1]), Err(FlowParseError::TooLong));
    }

    #[test]
    fn evaluator_returns_bounded_terminal_help() {
        let program = Program::parse(br#"help("fs")"#).unwrap();
        let mut runtime = FlowRuntime::new();
        let FlowEvalResult::Ready(FlowValue::String { bytes, len }) =
            program.evaluate(&mut runtime).unwrap()
        else {
            panic!("expected immediate help value");
        };
        assert!(bytes[..len].starts_with(b"fs\r\nUsage: fs."));
    }

    #[test]
    fn evaluator_returns_immediate_clear_value() {
        let program = Program::parse(b"clear()").unwrap();
        let mut runtime = FlowRuntime::new();
        assert_eq!(program.evaluate(&mut runtime), Ok(FlowEvalResult::Ready(FlowValue::Void)));
    }

    #[test]
    fn evaluator_keeps_fetch_promise_until_resolution() {
        let mut runtime = FlowRuntime::new();
        let declaration =
            Program::parse(b"var response = net.fetch(\"http://10.0.2.2/readme\")").unwrap();
        assert_eq!(declaration.evaluate(&mut runtime), Ok(FlowEvalResult::Ready(FlowValue::Void)));
        let FlowValue::Promise { handle, kind } = runtime.variable(b"response").unwrap() else {
            panic!("expected promise value");
        };
        assert_eq!(kind, PromiseType::Response);
        let awaiting = Program::parse(b"await response").unwrap();
        assert_eq!(awaiting.evaluate(&mut runtime), Ok(FlowEvalResult::Pending(handle)));
        runtime
            .resolve(
                handle,
                FlowValue::Response { status: 200, body: [0; MAX_VALUE_BYTES], body_len: 0 },
            )
            .unwrap();
        assert_eq!(
            awaiting.evaluate(&mut runtime),
            Ok(FlowEvalResult::Ready(FlowValue::Response {
                status: 200,
                body: [0; MAX_VALUE_BYTES],
                body_len: 0,
            }))
        );
    }

    #[test]
    fn evaluator_returns_immediate_echo_value() {
        let program = Program::parse(br#"echo("hello")"#).unwrap();
        let mut runtime = FlowRuntime::new();
        assert_eq!(
            program.evaluate(&mut runtime),
            Ok(FlowEvalResult::Ready(FlowValue::string(b"hello").unwrap()))
        );
    }

    #[test]
    fn promise_completion_is_typed_and_cancellable() {
        let mut table = PromiseTable::new();
        let handle = table.allocate(PromiseType::Bytes).unwrap();
        let _ = table.allocate(PromiseType::Response).unwrap();
        let _ = table.allocate(PromiseType::String).unwrap();
        let _ = table.allocate(PromiseType::Void).unwrap();
        assert_eq!(table.allocate(PromiseType::Bool), Err(PromiseError::Capacity));
        assert_eq!(table.resolve(handle, FlowValue::Void), Err(PromiseError::TypeMismatch));
        assert_eq!(table.cancel(handle), Ok(()));
        assert_eq!(table.state(handle), Ok(PromiseState::Cancelled));
    }

    #[test]
    fn ready_promise_then_executes_and_flattens_callback() {
        let mut runtime = FlowRuntime::new();
        let declaration =
            Program::parse(b"var response = net.fetch(\"http://10.0.2.2/readme\")").unwrap();
        declaration.evaluate(&mut runtime).unwrap();
        let FlowValue::Promise { handle, .. } = runtime.variable(b"response").unwrap() else {
            panic!("expected promise value");
        };
        runtime
            .resolve(
                handle,
                FlowValue::Response { status: 200, body: [0; MAX_VALUE_BYTES], body_len: 0 },
            )
            .unwrap();
        let callback =
            Program::parse(b"await response.then((item) => { return item.status; })").unwrap();
        assert_eq!(
            callback.evaluate(&mut runtime),
            Ok(FlowEvalResult::Ready(FlowValue::Number(200)))
        );
    }
}
