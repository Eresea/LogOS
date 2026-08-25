use logos_abi::GuiRect;

pub const MAX_UI_NODES: usize = 32;
const NO_PARENT: u16 = u16::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum UiNodeKind {
    Root = 1,
    Panel = 2,
    Label = 3,
    Button = 4,
    TextInput = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiNodeHandle {
    pub slot: u16,
    pub generation: u16,
}

impl UiNodeHandle {
    pub const EMPTY: Self = Self { slot: u16::MAX, generation: 0 };

    pub const fn is_valid(self) -> bool {
        self.slot != u16::MAX && self.generation != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiNodeSpec {
    pub kind: UiNodeKind,
    pub parent: u16,
    pub key: u16,
}

impl UiNodeSpec {
    pub const fn root(kind: UiNodeKind, key: u16) -> Self {
        Self { kind, parent: NO_PARENT, key }
    }

    pub const fn child(kind: UiNodeKind, parent: u16, key: u16) -> Self {
        Self { kind, parent, key }
    }
}

pub struct UiBlueprint {
    specs: [UiNodeSpec; MAX_UI_NODES],
    count: usize,
}

impl UiBlueprint {
    pub const fn new() -> Self {
        Self { specs: [UiNodeSpec::root(UiNodeKind::Panel, 0); MAX_UI_NODES], count: 0 }
    }

    pub fn push_root(&mut self, kind: UiNodeKind, key: u16) -> Result<u16, UiError> {
        if self.count != 0 {
            return Err(UiError::RootExists);
        }
        self.push(UiNodeSpec::root(kind, key))
    }

    pub fn push_child(&mut self, kind: UiNodeKind, parent: u16, key: u16) -> Result<u16, UiError> {
        if usize::from(parent) >= self.count {
            return Err(UiError::InvalidParent);
        }
        self.push(UiNodeSpec::child(kind, parent, key))
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn push(&mut self, spec: UiNodeSpec) -> Result<u16, UiError> {
        if self.count == MAX_UI_NODES {
            return Err(UiError::Capacity);
        }
        let index = self.count as u16;
        self.specs[self.count] = spec;
        self.count += 1;
        Ok(index)
    }
}

impl Default for UiBlueprint {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiError {
    Capacity,
    InvalidParent,
    RootExists,
    Stale,
    NotFound,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiNode {
    pub handle: UiNodeHandle,
    pub parent: UiNodeHandle,
    pub kind: UiNodeKind,
    pub key: u16,
    pub bounds: GuiRect,
    pub dirty: bool,
}

impl UiNode {
    const EMPTY: Self = Self {
        handle: UiNodeHandle::EMPTY,
        parent: UiNodeHandle::EMPTY,
        kind: UiNodeKind::Panel,
        key: 0,
        bounds: GuiRect::EMPTY,
        dirty: false,
    };
}

pub struct UiTree {
    nodes: [UiNode; MAX_UI_NODES],
    generations: [u16; MAX_UI_NODES],
    count: usize,
}

impl UiTree {
    pub const fn new() -> Self {
        Self { nodes: [UiNode::EMPTY; MAX_UI_NODES], generations: [0; MAX_UI_NODES], count: 0 }
    }

    pub fn from_blueprint(blueprint: &UiBlueprint) -> Result<Self, UiError> {
        let mut tree = Self::new();
        for index in 0..blueprint.count {
            let spec = blueprint.specs[index];
            let parent = if spec.parent == NO_PARENT {
                UiNodeHandle::EMPTY
            } else {
                tree.nodes[usize::from(spec.parent)].handle
            };
            tree.insert(spec.kind, parent, spec.key)?;
        }
        Ok(tree)
    }

    pub fn insert(
        &mut self,
        kind: UiNodeKind,
        parent: UiNodeHandle,
        key: u16,
    ) -> Result<UiNodeHandle, UiError> {
        if parent.is_valid() && self.lookup(parent).is_err() {
            return Err(UiError::Stale);
        }
        if !parent.is_valid() && self.nodes.iter().any(|node| node.handle.is_valid()) {
            return Err(UiError::RootExists);
        }
        let Some((slot, node)) =
            self.nodes.iter_mut().enumerate().find(|(_, node)| !node.handle.is_valid())
        else {
            return Err(UiError::Capacity);
        };
        self.generations[slot] = self.generations[slot].wrapping_add(1).max(1);
        let handle = UiNodeHandle { slot: slot as u16, generation: self.generations[slot] };
        *node = UiNode { handle, parent, kind, key, bounds: GuiRect::EMPTY, dirty: true };
        self.count += 1;
        Ok(handle)
    }

    pub fn destroy(&mut self, handle: UiNodeHandle) -> Result<(), UiError> {
        let _ = self.lookup(handle)?;
        loop {
            let child = self
                .nodes
                .iter()
                .find(|node| node.handle.is_valid() && node.parent == handle)
                .map(|node| node.handle);
            let Some(child) = child else { break };
            self.destroy(child)?;
        }
        let node = self.lookup(handle)?;
        self.nodes[usize::from(node.handle.slot)] = UiNode::EMPTY;
        self.count -= 1;
        Ok(())
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn node(&self, handle: UiNodeHandle) -> Result<&UiNode, UiError> {
        self.lookup(handle)
    }

    pub fn node_mut(&mut self, handle: UiNodeHandle) -> Result<&mut UiNode, UiError> {
        let index = self.lookup(handle)?.handle.slot as usize;
        Ok(&mut self.nodes[index])
    }

    pub fn set_bounds(&mut self, handle: UiNodeHandle, bounds: GuiRect) -> Result<(), UiError> {
        let node = self.node_mut(handle)?;
        if node.bounds != bounds {
            node.bounds = bounds;
            node.dirty = true;
        }
        Ok(())
    }

    pub fn clear_dirty(&mut self, handle: UiNodeHandle) -> Result<(), UiError> {
        self.node_mut(handle)?.dirty = false;
        Ok(())
    }

    fn lookup(&self, handle: UiNodeHandle) -> Result<&UiNode, UiError> {
        let Some(node) = self.nodes.get(usize::from(handle.slot)) else {
            return Err(UiError::Stale);
        };
        if node.handle != handle {
            return Err(UiError::Stale);
        }
        Ok(node)
    }
}

impl Default for UiTree {
    fn default() -> Self {
        Self::new()
    }
}

const _: () = assert!(core::mem::size_of::<UiTree>() <= 4096);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blueprint_instantiates_a_generation_safe_tree() {
        let mut blueprint = UiBlueprint::new();
        let root = blueprint.push_root(UiNodeKind::Root, 1).unwrap();
        let button = blueprint.push_child(UiNodeKind::Button, root, 2).unwrap();
        let mut tree = UiTree::from_blueprint(&blueprint).unwrap();
        assert_eq!(tree.len(), 2);
        let root_handle = tree.node(tree.nodes[usize::from(root)].handle).unwrap().handle;
        let button_handle = tree.nodes[usize::from(button)].handle;
        assert_eq!(tree.node(button_handle).unwrap().parent, root_handle);
        tree.destroy(button_handle).unwrap();
        let replacement = tree.insert(UiNodeKind::Label, root_handle, 3).unwrap();
        assert_eq!(replacement.slot, button_handle.slot);
        assert_ne!(replacement.generation, button_handle.generation);
        assert_eq!(tree.node(button_handle), Err(UiError::Stale));
    }

    #[test]
    fn tree_destroy_removes_descendants_and_bounds_are_dirty() {
        let mut tree = UiTree::new();
        let root = tree.insert(UiNodeKind::Root, UiNodeHandle::EMPTY, 1).unwrap();
        let child = tree.insert(UiNodeKind::Panel, root, 2).unwrap();
        let leaf = tree.insert(UiNodeKind::Label, child, 3).unwrap();
        tree.set_bounds(leaf, GuiRect::new(4, 5, 20, 10)).unwrap();
        assert!(tree.node(leaf).unwrap().dirty);
        tree.clear_dirty(leaf).unwrap();
        assert!(!tree.node(leaf).unwrap().dirty);
        tree.destroy(root).unwrap();
        assert!(tree.is_empty());
    }

    #[test]
    fn blueprint_rejects_multiple_roots_and_invalid_parents() {
        let mut blueprint = UiBlueprint::new();
        let root = blueprint.push_root(UiNodeKind::Root, 1).unwrap();
        assert_eq!(blueprint.push_root(UiNodeKind::Root, 2), Err(UiError::RootExists));
        assert_eq!(
            blueprint.push_child(UiNodeKind::Button, root + 1, 3),
            Err(UiError::InvalidParent)
        );
    }
}
