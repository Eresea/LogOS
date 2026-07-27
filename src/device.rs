#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Input,
    Display,
    Block,
    Network,
    Entropy,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Interface {
    class: Class,
    version: u16,
}

impl Interface {
    pub const fn new(class: Class) -> Self {
        Self { class, version: 1 }
    }

    pub const fn class(self) -> Class {
        self.class
    }

    pub fn compatible(self, other: Self) -> bool {
        self.class == other.class && self.version == other.version
    }
}

pub fn self_check() -> bool {
    let input = Interface::new(Class::Input);
    let display = Interface::new(Class::Display);
    let block = Interface::new(Class::Block);
    let network = Interface::new(Class::Network);
    let entropy = Interface::new(Class::Entropy);
    input.compatible(input)
        && !input.compatible(display)
        && block.class() == Class::Block
        && network.class() == Class::Network
        && entropy.class() == Class::Entropy
}
