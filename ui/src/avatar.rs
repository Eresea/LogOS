use crate::UiIcon;

pub const MAX_UI_AVATAR_TEXT_BYTES: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiAvatarImage {
    LogosMark,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiAvatarText {
    bytes: [u8; MAX_UI_AVATAR_TEXT_BYTES],
    len: u8,
}

impl UiAvatarText {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_UI_AVATAR_TEXT_BYTES {
            return None;
        }
        let mut text = Self { bytes: [0; MAX_UI_AVATAR_TEXT_BYTES], len: bytes.len() as u8 };
        text.bytes[..bytes.len()].copy_from_slice(bytes);
        Some(text)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiAvatarContent {
    None,
    Icon(UiIcon),
    Image(UiAvatarImage),
    Text(UiAvatarText),
}

impl UiAvatarContent {
    pub fn text(bytes: &[u8]) -> Option<Self> {
        UiAvatarText::from_bytes(bytes).map(Self::Text)
    }

    pub const fn icon(icon: UiIcon) -> Self {
        Self::Icon(icon)
    }

    pub const fn image(image: UiAvatarImage) -> Self {
        Self::Image(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_content_stays_within_the_two_byte_avatar_bound() {
        assert!(UiAvatarContent::text(b"A").is_some());
        assert!(UiAvatarContent::text(b"AB").is_some());
        assert!(UiAvatarContent::text(b"ABC").is_none());
    }
}
