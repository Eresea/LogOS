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
