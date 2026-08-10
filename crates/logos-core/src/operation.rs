#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationIdentity {
    owner: u64,
    generation: u32,
    request_id: u32,
}

impl OperationIdentity {
    pub const fn new(owner: u64, generation: u32, request_id: u32) -> Option<Self> {
        if owner == 0 || generation == 0 || request_id == 0 {
            None
        } else {
            Some(Self { owner, generation, request_id })
        }
    }

    pub const fn owner(self) -> u64 {
        self.owner
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }

    pub const fn request_id(self) -> u32 {
        self.request_id
    }

    pub const fn matches(self, owner: u64, generation: u32, request_id: u32) -> bool {
        self.owner == owner && self.generation == generation && self.request_id == request_id
    }

    pub const fn expired(self, deadline: u64, tick: u64) -> bool {
        deadline != 0 && tick >= deadline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_rejects_invalid_or_stale_operations() {
        assert!(OperationIdentity::new(0, 1, 1).is_none());
        assert!(OperationIdentity::new(7, 0, 1).is_none());
        assert!(OperationIdentity::new(7, 1, 0).is_none());
        let identity = OperationIdentity::new(7, 3, 9).unwrap();
        assert!(identity.matches(7, 3, 9));
        assert!(!identity.matches(7, 4, 9));
        assert!(!identity.expired(0, u64::MAX));
        assert!(identity.expired(10, 10));
    }
}
