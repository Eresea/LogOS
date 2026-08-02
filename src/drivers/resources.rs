use crate::platform::session::Principal;

const RESOURCES: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Input,
    Display,
    Entropy,
    Inference,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Ref(u8);

#[derive(Clone, Copy)]
struct Resource {
    kind: Kind,
    owner: Principal,
}

pub struct Registry {
    resources: [Option<Resource>; RESOURCES],
}

impl Registry {
    pub const fn new() -> Self {
        Self { resources: [None; RESOURCES] }
    }

    pub fn publish(&mut self, kind: Kind, owner: Principal) -> Option<Ref> {
        self.resources.iter_mut().enumerate().find(|(_, slot)| slot.is_none()).map(
            |(index, slot)| {
                *slot = Some(Resource { kind, owner });
                Ref(index as u8)
            },
        )
    }

    pub fn resolve(&self, reference: Ref, kind: Kind) -> Option<Principal> {
        self.resources
            .get(reference.0 as usize)
            .and_then(|resource| *resource)
            .filter(|resource| resource.kind == kind)
            .map(|resource| resource.owner)
    }
}

pub fn self_check() -> bool {
    let mut resources = Registry::new();
    resources.publish(Kind::Entropy, Principal::service(1)).is_some_and(|reference| {
        resources.resolve(reference, Kind::Entropy) == Some(Principal::service(1))
            && resources.resolve(reference, Kind::Input).is_none()
    }) && resources.publish(Kind::Display, Principal::service(2)).is_some()
        && resources.publish(Kind::Inference, Principal::service(3)).is_some()
}
