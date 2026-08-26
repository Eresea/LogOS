use crate::runtime::UiNodeHandle;

pub const MAX_UI_OUTPUT_EVENTS: usize = 32;
pub const MAX_UI_EVENT_ROUTES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiHandlerId(u16);

impl UiHandlerId {
    pub const fn new(index: u16) -> Self {
        Self(index)
    }

    pub const fn index(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum UiEventType {
    PointerDown = 1,
    PointerUp = 2,
    PointerMove = 3,
    KeyDown = 4,
    TextInput = 5,
    Focus = 6,
    Blur = 7,
    Click = 8,
    Submit = 9,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiInputEvent {
    PointerDown { x: i32, y: i32 },
    PointerUp { x: i32, y: i32 },
    PointerMove { x: i32, y: i32 },
    KeyDown { code: u16, modifiers: u8 },
    TextInput { scalar: u32 },
    Focus,
    Blur,
    Click,
    Submit,
}

impl UiInputEvent {
    pub const fn event_type(self) -> UiEventType {
        match self {
            Self::PointerDown { .. } => UiEventType::PointerDown,
            Self::PointerUp { .. } => UiEventType::PointerUp,
            Self::PointerMove { .. } => UiEventType::PointerMove,
            Self::KeyDown { .. } => UiEventType::KeyDown,
            Self::TextInput { .. } => UiEventType::TextInput,
            Self::Focus => UiEventType::Focus,
            Self::Blur => UiEventType::Blur,
            Self::Click => UiEventType::Click,
            Self::Submit => UiEventType::Submit,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiRoutedEvent {
    pub target: UiNodeHandle,
    pub handler: UiHandlerId,
    pub event: UiInputEvent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiOutputError {
    Full,
}

pub struct UiOutput<T: Copy> {
    entries: [Option<T>; MAX_UI_OUTPUT_EVENTS],
    head: usize,
    tail: usize,
    len: usize,
}

impl<T: Copy> UiOutput<T> {
    pub const fn new() -> Self {
        Self { entries: [None; MAX_UI_OUTPUT_EVENTS], head: 0, tail: 0, len: 0 }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn emit(&mut self, value: T) -> Result<(), UiOutputError> {
        if self.len == MAX_UI_OUTPUT_EVENTS {
            return Err(UiOutputError::Full);
        }
        self.entries[self.tail] = Some(value);
        self.tail = (self.tail + 1) % MAX_UI_OUTPUT_EVENTS;
        self.len += 1;
        Ok(())
    }

    pub fn pop(&mut self) -> Option<T> {
        let value = self.entries[self.head].take()?;
        self.head = (self.head + 1) % MAX_UI_OUTPUT_EVENTS;
        self.len -= 1;
        Some(value)
    }

    pub fn clear(&mut self) {
        while self.pop().is_some() {}
    }
}

impl<T: Copy> Default for UiOutput<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiEventError {
    Capacity,
    OutputFull,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct UiEventRoute {
    target: UiNodeHandle,
    event_type: UiEventType,
    handler: UiHandlerId,
}

pub struct UiEventRouter {
    routes: [UiEventRoute; MAX_UI_EVENT_ROUTES],
    len: usize,
}

impl UiEventRouter {
    pub const fn new() -> Self {
        Self {
            routes: [UiEventRoute {
                target: UiNodeHandle::EMPTY,
                event_type: UiEventType::Click,
                handler: UiHandlerId::new(0),
            }; MAX_UI_EVENT_ROUTES],
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn subscribe(
        &mut self,
        target: UiNodeHandle,
        event_type: UiEventType,
        handler: UiHandlerId,
    ) -> Result<(), UiEventError> {
        if let Some(route) = self
            .routes
            .iter_mut()
            .take(self.len)
            .find(|route| route.target == target && route.event_type == event_type)
        {
            route.handler = handler;
            return Ok(());
        }
        if self.len == MAX_UI_EVENT_ROUTES {
            return Err(UiEventError::Capacity);
        }
        self.routes[self.len] = UiEventRoute { target, event_type, handler };
        self.len += 1;
        Ok(())
    }

    pub fn unsubscribe(&mut self, target: UiNodeHandle, event_type: UiEventType) -> bool {
        let Some(index) = self.routes[..self.len]
            .iter()
            .position(|route| route.target == target && route.event_type == event_type)
        else {
            return false;
        };
        self.routes.copy_within(index + 1..self.len, index);
        self.len -= 1;
        true
    }

    pub fn unsubscribe_target(&mut self, target: UiNodeHandle) -> usize {
        let mut removed = 0;
        let mut index = 0;
        while index < self.len {
            if self.routes[index].target == target {
                self.routes.copy_within(index + 1..self.len, index);
                self.len -= 1;
                removed += 1;
            } else {
                index += 1;
            }
        }
        removed
    }

    pub fn is_subscribed(&self, target: UiNodeHandle, event_type: UiEventType) -> bool {
        self.routes
            .iter()
            .take(self.len)
            .any(|route| route.target == target && route.event_type == event_type)
    }

    pub fn dispatch(
        &self,
        target: UiNodeHandle,
        event: UiInputEvent,
        output: &mut UiOutput<UiRoutedEvent>,
    ) -> Result<bool, UiEventError> {
        let Some(route) = self
            .routes
            .iter()
            .take(self.len)
            .find(|route| route.target == target && route.event_type == event.event_type())
        else {
            return Ok(false);
        };
        output
            .emit(UiRoutedEvent { target, handler: route.handler, event })
            .map_err(|_| UiEventError::OutputFull)?;
        Ok(true)
    }
}

impl Default for UiEventRouter {
    fn default() -> Self {
        Self::new()
    }
}

const _: () = assert!(core::mem::size_of::<UiOutput<UiRoutedEvent>>() <= 1024);
const _: () = assert!(core::mem::size_of::<UiEventRouter>() <= 1024);

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: UiNodeHandle = UiNodeHandle { slot: 1, generation: 1 };
    const SECOND: UiNodeHandle = UiNodeHandle { slot: 2, generation: 1 };

    #[test]
    fn typed_output_is_fifo_and_bounded() {
        let mut output = UiOutput::<u16>::new();
        for value in 0..MAX_UI_OUTPUT_EVENTS as u16 {
            assert_eq!(output.emit(value), Ok(()));
        }
        assert_eq!(output.emit(99), Err(UiOutputError::Full));
        for value in 0..MAX_UI_OUTPUT_EVENTS as u16 {
            assert_eq!(output.pop(), Some(value));
        }
        assert!(output.is_empty());
        output.emit(7).unwrap();
        output.clear();
        assert_eq!(output.pop(), None);
    }

    #[test]
    fn router_dispatches_only_matching_typed_hooks() {
        let mut router = UiEventRouter::new();
        router.subscribe(FIRST, UiEventType::Click, UiHandlerId::new(4)).unwrap();
        let mut output = UiOutput::new();

        assert_eq!(router.dispatch(FIRST, UiInputEvent::Submit, &mut output), Ok(false));
        assert_eq!(router.dispatch(SECOND, UiInputEvent::Click, &mut output), Ok(false));
        assert_eq!(router.dispatch(FIRST, UiInputEvent::Click, &mut output), Ok(true));
        let event = output.pop().unwrap();
        assert_eq!(event.target, FIRST);
        assert_eq!(event.handler, UiHandlerId::new(4));
        assert_eq!(event.event, UiInputEvent::Click);
    }

    #[test]
    fn subscription_replacement_and_removal_are_generation_bound() {
        let mut router = UiEventRouter::new();
        router.subscribe(FIRST, UiEventType::KeyDown, UiHandlerId::new(1)).unwrap();
        router.subscribe(FIRST, UiEventType::KeyDown, UiHandlerId::new(2)).unwrap();
        assert_eq!(router.len(), 1);
        let mut output = UiOutput::new();
        router
            .dispatch(FIRST, UiInputEvent::KeyDown { code: 13, modifiers: 0 }, &mut output)
            .unwrap();
        assert_eq!(output.pop().unwrap().handler, UiHandlerId::new(2));
        assert!(router.unsubscribe(FIRST, UiEventType::KeyDown));
        assert!(!router.unsubscribe(FIRST, UiEventType::KeyDown));
    }

    #[test]
    fn target_cleanup_removes_only_the_exact_generation() {
        let mut router = UiEventRouter::new();
        router.subscribe(FIRST, UiEventType::Click, UiHandlerId::new(1)).unwrap();
        router.subscribe(FIRST, UiEventType::Submit, UiHandlerId::new(2)).unwrap();
        router.subscribe(SECOND, UiEventType::Click, UiHandlerId::new(3)).unwrap();

        assert_eq!(router.unsubscribe_target(FIRST), 2);
        assert_eq!(router.len(), 1);
        assert!(!router.is_subscribed(FIRST, UiEventType::Click));
        assert!(router.is_subscribed(SECOND, UiEventType::Click));
    }

    #[test]
    fn output_backpressure_is_reported_without_dropping_input() {
        let mut router = UiEventRouter::new();
        router.subscribe(FIRST, UiEventType::Click, UiHandlerId::new(1)).unwrap();
        let mut output = UiOutput::new();
        for _ in 0..MAX_UI_OUTPUT_EVENTS {
            router.dispatch(FIRST, UiInputEvent::Click, &mut output).unwrap();
        }
        assert_eq!(
            router.dispatch(FIRST, UiInputEvent::Click, &mut output),
            Err(UiEventError::OutputFull)
        );
        assert_eq!(output.len(), MAX_UI_OUTPUT_EVENTS);
    }
}
