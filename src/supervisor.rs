const MAX_MANIFESTS: usize = 4;

pub const SUPERVISOR: &[u8] = b"supervisor";
pub const VIRTIO_BALLOON: &[u8] = b"virtio-balloon";

pub struct Manifest {
    pub name: &'static [u8],
    pub dependencies: &'static [&'static [u8]],
}

const SUPERVISOR_MANIFEST: Manifest = Manifest { name: SUPERVISOR, dependencies: &[] };
const VIRTIO_MANIFEST: Manifest = Manifest { name: VIRTIO_BALLOON, dependencies: &[SUPERVISOR] };
const BOOT_MANIFESTS: &[Manifest] = &[SUPERVISOR_MANIFEST, VIRTIO_MANIFEST];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Duplicate,
    MissingDependency,
    Cycle,
}

pub struct Plan {
    order: [Option<&'static Manifest>; MAX_MANIFESTS],
    len: usize,
}

#[derive(Clone, Copy)]
struct Heartbeat {
    name: &'static [u8],
    timeout: u64,
    last: u64,
}

pub struct Health {
    heartbeats: [Option<Heartbeat>; MAX_MANIFESTS],
}

impl Health {
    pub const fn new() -> Self {
        Self { heartbeats: [None; MAX_MANIFESTS] }
    }

    pub fn watch(&mut self, plan: &Plan, name: &'static [u8], timeout: u64, tick: u64) -> bool {
        if timeout == 0 || !plan.starts(name) || self.find(name).is_some() {
            return false;
        }
        for slot in &mut self.heartbeats {
            if slot.is_none() {
                *slot = Some(Heartbeat { name, timeout, last: tick });
                return true;
            }
        }
        false
    }

    pub fn beat(&mut self, name: &[u8], tick: u64) -> bool {
        let Some(index) = self.find(name) else {
            return false;
        };
        self.heartbeats[index].as_mut().unwrap().last = tick;
        true
    }

    pub fn healthy(&self, name: &[u8], tick: u64) -> bool {
        self.find(name).is_some_and(|index| {
            let heartbeat = self.heartbeats[index].unwrap();
            tick.wrapping_sub(heartbeat.last) <= heartbeat.timeout
        })
    }

    fn find(&self, name: &[u8]) -> Option<usize> {
        self.heartbeats
            .iter()
            .position(|heartbeat| heartbeat.is_some_and(|heartbeat| heartbeat.name == name))
    }
}

impl Plan {
    pub fn build(manifests: &'static [Manifest]) -> Result<Self, Error> {
        if manifests.len() > MAX_MANIFESTS {
            return Err(Error::Cycle);
        }
        for (index, manifest) in manifests.iter().enumerate() {
            if manifest.name.is_empty()
                || manifests[..index].iter().any(|other| other.name == manifest.name)
            {
                return Err(Error::Duplicate);
            }
            if manifest
                .dependencies
                .iter()
                .any(|dependency| !manifests.iter().any(|other| other.name == *dependency))
            {
                return Err(Error::MissingDependency);
            }
        }
        let mut plan = Self { order: [None; MAX_MANIFESTS], len: 0 };
        while plan.len < manifests.len() {
            let mut progressed = false;
            for manifest in manifests {
                if !plan.contains(manifest.name)
                    && manifest.dependencies.iter().all(|dependency| plan.contains(dependency))
                {
                    plan.order[plan.len] = Some(manifest);
                    plan.len += 1;
                    progressed = true;
                }
            }
            if !progressed {
                return Err(Error::Cycle);
            }
        }
        Ok(plan)
    }

    pub fn starts(&self, name: &[u8]) -> bool {
        self.contains(name)
    }

    fn contains(&self, name: &[u8]) -> bool {
        self.order[..self.len]
            .iter()
            .any(|manifest| manifest.is_some_and(|manifest| manifest.name == name))
    }
}

pub fn boot_plan() -> Result<Plan, Error> {
    Plan::build(BOOT_MANIFESTS)
}

pub fn self_check() -> bool {
    const A: &[u8] = b"a";
    const B: &[u8] = b"b";
    const OK: &[Manifest] =
        &[Manifest { name: B, dependencies: &[A] }, Manifest { name: A, dependencies: &[] }];
    const MISSING: &[Manifest] = &[Manifest { name: A, dependencies: &[B] }];
    const CYCLE: &[Manifest] =
        &[Manifest { name: A, dependencies: &[B] }, Manifest { name: B, dependencies: &[A] }];
    Plan::build(OK).is_ok_and(|plan| plan.starts(A) && plan.starts(B))
        && matches!(Plan::build(MISSING), Err(Error::MissingDependency))
        && matches!(Plan::build(CYCLE), Err(Error::Cycle))
}

pub fn health_self_check() -> bool {
    let Ok(plan) = boot_plan() else {
        return false;
    };
    let mut health = Health::new();
    health.watch(&plan, VIRTIO_BALLOON, 2, 10)
        && health.healthy(VIRTIO_BALLOON, 12)
        && !health.healthy(VIRTIO_BALLOON, 13)
        && health.beat(VIRTIO_BALLOON, 13)
        && health.healthy(VIRTIO_BALLOON, 15)
        && !health.watch(&plan, VIRTIO_BALLOON, 2, 15)
}
