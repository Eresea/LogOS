const FAULTS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    FailOnce,
    FailAlways,
    Delay(u64),
    Drop,
    CrashOwner,
}

#[derive(Clone, Copy)]
struct Entry {
    name: &'static str,
    action: Action,
}

pub struct Faults {
    entries: [Option<Entry>; FAULTS],
}

impl Faults {
    pub const fn new() -> Self {
        Self { entries: [None; FAULTS] }
    }
    pub fn set(&mut self, name: &'static str, action: Action) -> bool {
        if name.is_empty() || name.len() > 48 {
            return false;
        }
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.is_none()) {
            *entry = Some(Entry { name, action });
            true
        } else {
            false
        }
    }
    pub fn take(&mut self, name: &str) -> Option<Action> {
        let slot =
            self.entries.iter_mut().find(|slot| slot.is_some_and(|entry| entry.name == name))?;
        let action = slot.unwrap().action;
        if action == Action::FailOnce {
            *slot = None;
        }
        Some(action)
    }
}

impl Default for Faults {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn once_is_consumed_and_always_remains() {
        let mut faults = Faults::new();
        assert!(faults.set("ipc.after_enqueue", Action::FailOnce));
        assert_eq!(faults.take("ipc.after_enqueue"), Some(Action::FailOnce));
        assert_eq!(faults.take("ipc.after_enqueue"), None);
        assert!(faults.set("driver.before_completion", Action::FailAlways));
        assert_eq!(faults.take("driver.before_completion"), Some(Action::FailAlways));
        assert_eq!(faults.take("driver.before_completion"), Some(Action::FailAlways));
    }
}
