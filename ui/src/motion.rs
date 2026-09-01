#![allow(clippy::cast_possible_truncation)]

use crate::UiRect;
use crate::runtime::MAX_UI_NODES;

pub const MAX_UI_KEYFRAMES: usize = 8;
pub const MAX_UI_MOTION_DURATION_MS: u16 = 2_000;
pub const UI_MOTION_FRAME_MS: u64 = 16;
/// Sentinel used only by framework-approved looping animation presets.
pub const UI_ANIMATION_INFINITE_REPEAT: u8 = u8::MAX;

const MOTION_COLORS: u8 = 1 << 0;
const MOTION_OPACITY: u8 = 1 << 1;
const MOTION_TRANSFORM: u8 = 1 << 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum UiTransitionProperty {
    Colors = MOTION_COLORS,
    Opacity = MOTION_OPACITY,
    Transform = MOTION_TRANSFORM,
}

impl UiTransitionProperty {
    pub const fn mask(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UiEasing {
    Linear,
    EaseIn,
    #[default]
    EaseOut,
    EaseInOut,
    EaseInQuad,
    EaseOutQuad,
    EaseInOutQuad,
    EaseInCubic,
    EaseOutCubic,
    EaseInOutCubic,
    EaseOutBack,
    /// Control points are percentages in the range 0..=100.
    CubicBezier {
        x1: i16,
        y1: i16,
        x2: i16,
        y2: i16,
    },
}

impl UiEasing {
    pub const fn is_valid(self) -> bool {
        match self {
            Self::CubicBezier { x1, y1, x2, y2 } => {
                x1 >= 0
                    && x1 <= 100
                    && x2 >= 0
                    && x2 <= 100
                    && y1 >= -100
                    && y1 <= 200
                    && y2 >= -100
                    && y2 <= 200
            }
            _ => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiTransform {
    pub translate_x: i16,
    pub translate_y: i16,
    pub scale_q8_8: u16,
    pub rotation_degrees: i16,
}

impl UiTransform {
    pub const IDENTITY: Self =
        Self { translate_x: 0, translate_y: 0, scale_q8_8: 256, rotation_degrees: 0 };

    pub const fn is_identity(self) -> bool {
        self.translate_x == 0
            && self.translate_y == 0
            && self.scale_q8_8 == 256
            && self.rotation_degrees == 0
    }

    pub fn contains(self, bounds: UiRect, x: i32, y: i32) -> bool {
        if bounds.is_empty() || self.scale_q8_8 == 0 {
            return false;
        }
        if self.is_identity() {
            return bounds.contains(x, y);
        }
        let center_x = i64::from(bounds.x) + i64::from(bounds.width) / 2;
        let center_y = i64::from(bounds.y) + i64::from(bounds.height) / 2;
        let dx = i64::from(x) - center_x - i64::from(self.translate_x);
        let dy = i64::from(y) - center_y - i64::from(self.translate_y);
        let (sin, cos) = sin_cos(self.rotation_degrees);
        let source_x = center_x
            + (dx * i64::from(cos) + dy * i64::from(sin)) * 256
                / (i64::from(self.scale_q8_8) * 32_767);
        let source_y = center_y
            + (dy * i64::from(cos) - dx * i64::from(sin)) * 256
                / (i64::from(self.scale_q8_8) * 32_767);
        bounds.contains(source_x as i32, source_y as i32)
    }
}

impl Default for UiTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiComputedStyle {
    pub fill_color: u32,
    pub text_color: u32,
    pub opacity_q16: u16,
    pub transform: UiTransform,
}

impl UiComputedStyle {
    pub const DEFAULT: Self = Self {
        fill_color: 0,
        text_color: 0,
        opacity_q16: u16::MAX,
        transform: UiTransform::IDENTITY,
    };
}

impl Default for UiComputedStyle {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UiAnimationDirection {
    #[default]
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UiAnimationFill {
    #[default]
    None,
    Forwards,
    Backwards,
    Both,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiAnimationPreset {
    In,
    Pulse,
    Spin,
}

impl UiAnimationPreset {
    pub fn spec(self) -> UiAnimationSpec {
        let mut spec = UiAnimationSpec::EMPTY;
        spec.duration_ms = match self {
            Self::In => 180,
            Self::Pulse => 700,
            Self::Spin => 1_000,
        };
        spec.fill =
            if matches!(self, Self::In) { UiAnimationFill::Both } else { UiAnimationFill::None };
        spec.repeat = if matches!(self, Self::In) { 1 } else { UI_ANIMATION_INFINITE_REPEAT };
        spec.direction = if matches!(self, Self::Spin) {
            UiAnimationDirection::Normal
        } else {
            UiAnimationDirection::Alternate
        };
        let from = match self {
            Self::In => UiComputedStyle {
                opacity_q16: 0,
                transform: UiTransform {
                    translate_y: -8,
                    scale_q8_8: 251,
                    ..UiTransform::IDENTITY
                },
                ..UiComputedStyle::DEFAULT
            },
            Self::Pulse => UiComputedStyle { opacity_q16: 49_152, ..UiComputedStyle::DEFAULT },
            Self::Spin => UiComputedStyle {
                transform: UiTransform { rotation_degrees: 0, ..UiTransform::IDENTITY },
                ..UiComputedStyle::DEFAULT
            },
        };
        let to = match self {
            Self::In | Self::Pulse => UiComputedStyle::DEFAULT,
            Self::Spin => UiComputedStyle {
                transform: UiTransform { rotation_degrees: 360, ..UiTransform::IDENTITY },
                ..UiComputedStyle::DEFAULT
            },
        };
        let _ = spec.push(UiKeyframe::new(0, from));
        let _ = spec.push(UiKeyframe::new(u16::MAX, to));
        spec
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiTransitionSpec {
    pub properties: u8,
    pub duration_ms: u16,
    pub delay_ms: u16,
    pub easing: UiEasing,
}

impl UiTransitionSpec {
    pub const DEFAULT: Self =
        Self { properties: 0, duration_ms: 200, delay_ms: 0, easing: UiEasing::EaseOut };

    pub const fn includes(self, property: UiTransitionProperty) -> bool {
        self.properties & property.mask() != 0
    }

    pub const fn with_property(mut self, property: UiTransitionProperty) -> Self {
        self.properties |= property.mask();
        self
    }

    pub const fn is_valid(self) -> bool {
        self.properties & !(MOTION_COLORS | MOTION_OPACITY | MOTION_TRANSFORM) == 0
            && self.duration_ms <= MAX_UI_MOTION_DURATION_MS
            && self.delay_ms <= MAX_UI_MOTION_DURATION_MS
            && self.easing.is_valid()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiKeyframe {
    pub offset_q16: u16,
    pub properties: u8,
    pub style: UiComputedStyle,
    pub easing: UiEasing,
}

impl UiKeyframe {
    pub const EMPTY: Self = Self {
        offset_q16: 0,
        properties: 0,
        style: UiComputedStyle::DEFAULT,
        easing: UiEasing::EaseOut,
    };

    pub const fn new(offset_q16: u16, style: UiComputedStyle) -> Self {
        Self {
            offset_q16,
            properties: MOTION_COLORS | MOTION_OPACITY | MOTION_TRANSFORM,
            style,
            easing: UiEasing::EaseOut,
        }
    }

    pub const fn with_properties(mut self, property: UiTransitionProperty) -> Self {
        self.properties = property.mask();
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiAnimationSpec {
    pub keyframes: [UiKeyframe; MAX_UI_KEYFRAMES],
    pub keyframe_count: u8,
    pub duration_ms: u16,
    pub delay_ms: u16,
    pub repeat: u8,
    pub direction: UiAnimationDirection,
    pub fill: UiAnimationFill,
    pub easing: UiEasing,
}

impl UiAnimationSpec {
    pub const EMPTY: Self = Self {
        keyframes: [UiKeyframe::EMPTY; MAX_UI_KEYFRAMES],
        keyframe_count: 0,
        duration_ms: 200,
        delay_ms: 0,
        repeat: 1,
        direction: UiAnimationDirection::Normal,
        fill: UiAnimationFill::None,
        easing: UiEasing::EaseOut,
    };

    pub const fn new(duration_ms: u16) -> Self {
        Self { duration_ms, ..Self::EMPTY }
    }

    pub fn push(&mut self, keyframe: UiKeyframe) -> bool {
        if usize::from(self.keyframe_count) == MAX_UI_KEYFRAMES
            || (self.keyframe_count != 0
                && keyframe.offset_q16
                    <= self.keyframes[usize::from(self.keyframe_count - 1)].offset_q16)
        {
            return false;
        }
        self.keyframes[usize::from(self.keyframe_count)] = keyframe;
        self.keyframe_count += 1;
        true
    }

    pub const fn is_valid(self) -> bool {
        self.keyframe_count >= 2
            && self.keyframe_count as usize <= MAX_UI_KEYFRAMES
            && self.duration_ms <= MAX_UI_MOTION_DURATION_MS
            && self.delay_ms <= MAX_UI_MOTION_DURATION_MS
            && (self.repeat <= 8 || self.repeat == UI_ANIMATION_INFINITE_REPEAT)
            && self.keyframes[0].offset_q16 == 0
            && self.keyframes[self.keyframe_count as usize - 1].offset_q16 == u16::MAX
            && self.easing.is_valid()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiMotionStatus {
    pub active: bool,
    pub changed: bool,
}

impl UiMotionStatus {
    pub const IDLE: Self = Self { active: false, changed: false };
}

#[derive(Clone, Copy)]
struct UiMotion {
    valid: bool,
    active: bool,
    animation: bool,
    started_ms: u64,
    transition: UiTransitionSpec,
    animation_spec: UiAnimationSpec,
    from: UiComputedStyle,
    to: UiComputedStyle,
    current: UiComputedStyle,
}

impl UiMotion {
    const EMPTY: Self = Self {
        valid: false,
        active: false,
        animation: false,
        started_ms: 0,
        transition: UiTransitionSpec::DEFAULT,
        animation_spec: UiAnimationSpec::EMPTY,
        from: UiComputedStyle::DEFAULT,
        to: UiComputedStyle::DEFAULT,
        current: UiComputedStyle::DEFAULT,
    };
}

pub struct UiAnimator {
    motions: [UiMotion; MAX_UI_NODES],
}

impl UiAnimator {
    pub const fn new() -> Self {
        Self { motions: [UiMotion::EMPTY; MAX_UI_NODES] }
    }

    pub fn start_transition(
        &mut self,
        index: usize,
        from: UiComputedStyle,
        to: UiComputedStyle,
        transition: UiTransitionSpec,
        now_ms: u64,
    ) -> bool {
        if index >= MAX_UI_NODES || !transition.is_valid() {
            return false;
        }
        let motion = &mut self.motions[index];
        motion.valid = true;
        motion.active = transition.properties != 0 && from != to;
        motion.animation = false;
        motion.started_ms = now_ms;
        motion.transition = transition;
        motion.from = from;
        motion.to = to;
        motion.current = from;
        if !motion.active {
            motion.current = to;
        }
        true
    }

    pub fn start_animation(
        &mut self,
        index: usize,
        base: UiComputedStyle,
        spec: UiAnimationSpec,
        now_ms: u64,
    ) -> bool {
        if index >= MAX_UI_NODES || !spec.is_valid() {
            return false;
        }
        let motion = &mut self.motions[index];
        motion.valid = true;
        motion.active = true;
        motion.animation = true;
        motion.started_ms = now_ms;
        motion.animation_spec = spec;
        motion.from = base;
        motion.to = base;
        motion.current = base;
        true
    }

    pub fn value(&self, index: usize) -> Option<UiComputedStyle> {
        self.motions.get(index).filter(|motion| motion.valid).map(|motion| motion.current)
    }

    pub fn clear(&mut self, index: usize) {
        if let Some(motion) = self.motions.get_mut(index) {
            *motion = UiMotion::EMPTY;
        }
    }

    pub fn advance(&mut self, now_ms: u64) -> UiMotionStatus {
        let mut active = false;
        let mut changed = false;
        for motion in &mut self.motions {
            if !motion.active {
                continue;
            }
            let before = motion.current;
            if motion.animation {
                let (style, done) = sample_animation(*motion, now_ms);
                motion.current = style;
                motion.active = !done;
            } else {
                let elapsed = now_ms.saturating_sub(motion.started_ms);
                if elapsed <= u64::from(motion.transition.delay_ms) {
                    motion.current = motion.from;
                } else {
                    let duration = u64::from(motion.transition.duration_ms.max(1));
                    let progress = ((elapsed - u64::from(motion.transition.delay_ms)) * 65_535
                        / duration)
                        .min(65_535) as u16;
                    let eased = ease(motion.transition.easing, progress);
                    motion.current =
                        interpolate(motion.from, motion.to, eased, motion.transition.properties);
                    if progress == u16::MAX {
                        motion.active = false;
                    }
                }
            }
            changed |= before != motion.current;
            active |= motion.active;
        }
        UiMotionStatus { active, changed }
    }

    pub fn next_deadline(&self, now_ms: u64) -> Option<u64> {
        self.motions
            .iter()
            .filter(|motion| motion.active)
            .map(|motion| {
                let frame = now_ms.saturating_add(UI_MOTION_FRAME_MS);
                motion
                    .started_ms
                    .saturating_add(u64::from(if motion.animation {
                        motion.animation_spec.delay_ms
                    } else {
                        motion.transition.delay_ms
                    }))
                    .min(frame)
            })
            .min()
    }
}

impl Default for UiAnimator {
    fn default() -> Self {
        Self::new()
    }
}

fn sample_animation(motion: UiMotion, now_ms: u64) -> (UiComputedStyle, bool) {
    let spec = motion.animation_spec;
    let elapsed = now_ms.saturating_sub(motion.started_ms);
    if elapsed < u64::from(spec.delay_ms) {
        if matches!(spec.fill, UiAnimationFill::Backwards | UiAnimationFill::Both) {
            let reverse = matches!(
                spec.direction,
                UiAnimationDirection::Reverse | UiAnimationDirection::AlternateReverse
            );
            return (sample_keyframes(spec, if reverse { u16::MAX } else { 0 }), false);
        }
        return (motion.from, false);
    }
    let duration = u64::from(spec.duration_ms.max(1));
    let total = if spec.repeat == UI_ANIMATION_INFINITE_REPEAT {
        u64::MAX
    } else {
        duration.saturating_mul(u64::from(spec.repeat))
    };
    let after_delay = elapsed - u64::from(spec.delay_ms);
    let done = spec.repeat == 0 || after_delay >= total;
    let sample_time = after_delay.min(total.saturating_sub(1));
    let cycle = sample_time / duration;
    let local = (sample_time % duration * 65_535 / duration) as u16;
    let reverse = match spec.direction {
        UiAnimationDirection::Normal => false,
        UiAnimationDirection::Reverse => true,
        UiAnimationDirection::Alternate => cycle % 2 == 1,
        UiAnimationDirection::AlternateReverse => cycle % 2 == 0,
    };
    let progress = if reverse { u16::MAX - local } else { local };
    let style = sample_keyframes(spec, progress);
    if done {
        match spec.fill {
            UiAnimationFill::Forwards | UiAnimationFill::Both => {
                (sample_keyframes(spec, if reverse { 0 } else { u16::MAX }), true)
            }
            _ => (motion.from, true),
        }
    } else {
        (style, false)
    }
}

fn sample_keyframes(spec: UiAnimationSpec, progress: u16) -> UiComputedStyle {
    let count = usize::from(spec.keyframe_count);
    let mut right = 1;
    while right < count && progress > spec.keyframes[right].offset_q16 {
        right += 1;
    }
    let left = right.saturating_sub(1);
    let first = spec.keyframes[left];
    let second = spec.keyframes[right.min(count - 1)];
    let span = u32::from(second.offset_q16.saturating_sub(first.offset_q16)).max(1);
    let local =
        (u32::from(progress.saturating_sub(first.offset_q16)) * 65_535 / span).min(65_535) as u16;
    let eased =
        ease(if first.easing == UiEasing::EaseOut { spec.easing } else { first.easing }, local);
    interpolate(first.style, second.style, eased, first.properties | second.properties)
}

fn interpolate(
    from: UiComputedStyle,
    to: UiComputedStyle,
    progress: u16,
    properties: u8,
) -> UiComputedStyle {
    let mut value = from;
    if properties & MOTION_COLORS != 0 {
        value.fill_color = lerp_color(from.fill_color, to.fill_color, progress);
        value.text_color = lerp_color(from.text_color, to.text_color, progress);
    }
    if properties & MOTION_OPACITY != 0 {
        value.opacity_q16 = lerp_u16(from.opacity_q16, to.opacity_q16, progress);
    }
    if properties & MOTION_TRANSFORM != 0 {
        value.transform.translate_x =
            lerp_i16(from.transform.translate_x, to.transform.translate_x, progress);
        value.transform.translate_y =
            lerp_i16(from.transform.translate_y, to.transform.translate_y, progress);
        value.transform.scale_q8_8 =
            lerp_u16(from.transform.scale_q8_8, to.transform.scale_q8_8, progress);
        value.transform.rotation_degrees =
            lerp_i16(from.transform.rotation_degrees, to.transform.rotation_degrees, progress);
    }
    value
}

fn lerp_u16(from: u16, to: u16, progress: u16) -> u16 {
    let from = i64::from(from);
    let delta = i64::from(to) - from;
    (from + delta * i64::from(progress) / 65_535).clamp(0, i64::from(u16::MAX)) as u16
}

fn lerp_i16(from: i16, to: i16, progress: u16) -> i16 {
    let from = i64::from(from);
    let delta = i64::from(to) - from;
    (from * 65_535 + delta * i64::from(progress))
        .div_euclid(65_535)
        .clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
}

fn lerp_color(from: u32, to: u32, progress: u16) -> u32 {
    let mut result = 0;
    for shift in [0, 8, 16, 24] {
        let a = ((from >> shift) & 0xff) as u16;
        let b = ((to >> shift) & 0xff) as u16;
        result |= u32::from(lerp_u16(a << 8, b << 8, progress) >> 8) << shift;
    }
    result
}

fn ease(easing: UiEasing, t: u16) -> u16 {
    let t = i64::from(t);
    let max = 65_535i64;
    let square = t * t / max;
    let cube = square * t / max;
    let value = match easing {
        UiEasing::Linear => t,
        UiEasing::EaseIn | UiEasing::EaseInQuad => square,
        UiEasing::EaseOut | UiEasing::EaseOutQuad => max - (max - t) * (max - t) / max,
        UiEasing::EaseInOut => {
            if t < max / 2 {
                2 * t * t / max
            } else {
                max - ((-2 * t + 2) * (-2 * t + 2) / (2 * max))
            }
        }
        UiEasing::EaseInOutQuad => {
            if t < max / 2 {
                2 * t * t / max
            } else {
                max - ((-2 * t + 2) * (-2 * t + 2) / (2 * max))
            }
        }
        UiEasing::EaseInCubic => cube,
        UiEasing::EaseOutCubic => max - (max - t) * (max - t) * (max - t) / (max * max),
        UiEasing::EaseInOutCubic => {
            if t < max / 2 {
                4 * t * t * t / (max * max)
            } else {
                max - ((-2 * t + 2) * (-2 * t + 2) * (-2 * t + 2) / (2 * max * max))
            }
        }
        UiEasing::EaseOutBack => {
            let c1 = 1_701i64;
            let c3 = c1 + max;
            let p = t - max;
            (max + c3 * p * p * p / (max * max) + c1 * p * p / max).clamp(0, max)
        }
        UiEasing::CubicBezier { x1, y1, x2, y2 } => cubic_bezier(x1, y1, x2, y2, t),
    };
    value.clamp(0, max) as u16
}

fn cubic_bezier(x1: i16, y1: i16, x2: i16, y2: i16, target_x: i64) -> i64 {
    let max = 65_535i64;
    let x1 = i64::from(x1.clamp(0, 100)) * max / 100;
    let x2 = i64::from(x2.clamp(0, 100)) * max / 100;
    let y1 = i64::from(y1.clamp(-100, 200)) * max / 100;
    let y2 = i64::from(y2.clamp(-100, 200)) * max / 100;
    let mut low = 0i64;
    let mut high = max;
    for _ in 0..12 {
        let parameter = (low + high) / 2;
        if bezier(parameter, x1, x2) < target_x {
            low = parameter + 1;
        } else {
            high = parameter;
        }
    }
    bezier_with_endpoints(high.min(max), y1, y2).clamp(0, max)
}

const SIN_Q15: [i32; 7] = [0, 8_481, 16_384, 23_170, 28_378, 31_651, 32_767];

fn sin_cos(degrees: i16) -> (i32, i32) {
    let degrees = i32::from(degrees).rem_euclid(360);
    let quarter = if degrees <= 90 {
        degrees
    } else if degrees <= 180 {
        180 - degrees
    } else if degrees <= 270 {
        degrees - 180
    } else {
        360 - degrees
    };
    let sine = quarter_sine(quarter) * if degrees > 180 { -1 } else { 1 };
    let cosine = quarter_sine(90 - quarter) * if degrees > 90 && degrees <= 270 { -1 } else { 1 };
    (sine, cosine)
}

fn quarter_sine(degrees: i32) -> i32 {
    let degrees = degrees.clamp(0, 90);
    let index = (degrees / 15) as usize;
    if index == 6 {
        return SIN_Q15[6];
    }
    SIN_Q15[index] + (SIN_Q15[index + 1] - SIN_Q15[index]) * (degrees % 15) / 15
}

fn bezier(parameter: i64, first: i64, second: i64) -> i64 {
    bezier_with_endpoints(parameter, first, second)
}

fn bezier_with_endpoints(parameter: i64, first: i64, second: i64) -> i64 {
    let max = 65_535i64;
    let inverse = max - parameter;
    (3 * inverse * inverse * parameter * first
        + 3 * inverse * parameter * parameter * second
        + parameter * parameter * parameter)
        / (max * max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(value: u16) -> UiComputedStyle {
        UiComputedStyle {
            fill_color: u32::from(value),
            text_color: u32::from(value),
            opacity_q16: value,
            transform: UiTransform { translate_x: value as i16, ..UiTransform::IDENTITY },
        }
    }

    #[test]
    fn transition_reaches_target_and_exposes_deadline() {
        let mut animator = UiAnimator::new();
        let spec = UiTransitionSpec {
            properties: MOTION_OPACITY,
            duration_ms: 100,
            ..UiTransitionSpec::DEFAULT
        };
        assert!(animator.start_transition(0, style(0), style(u16::MAX), spec, 10));
        assert_eq!(animator.next_deadline(10), Some(10));
        assert!(animator.advance(60).active);
        assert_eq!(animator.value(0).unwrap().opacity_q16, 49_151);
        assert!(!animator.advance(110).active);
        assert_eq!(animator.value(0).unwrap().opacity_q16, u16::MAX);
    }

    #[test]
    fn animation_respects_repeat_direction_and_fill() {
        let mut spec = UiAnimationSpec::EMPTY;
        spec.duration_ms = 100;
        spec.repeat = 2;
        spec.direction = UiAnimationDirection::Alternate;
        spec.fill = UiAnimationFill::Forwards;
        assert!(spec.push(UiKeyframe::new(0, style(0))));
        assert!(spec.push(UiKeyframe::new(u16::MAX, style(u16::MAX))));
        let mut animator = UiAnimator::new();
        assert!(animator.start_animation(0, style(0), spec, 0));
        assert!(animator.advance(50).active);
        assert!(animator.advance(150).active);
        assert!(!animator.advance(200).active);
        assert_eq!(animator.value(0).unwrap().fill_color, 0);
    }

    #[test]
    fn approved_presets_loop_without_expiring() {
        let spec = UiAnimationPreset::Pulse.spec();
        assert_eq!(spec.repeat, UI_ANIMATION_INFINITE_REPEAT);
        let mut animator = UiAnimator::new();
        assert!(animator.start_animation(0, UiComputedStyle::DEFAULT, spec, 0));
        assert!(animator.advance(u64::MAX / 2).active);
        assert!(animator.next_deadline(u64::MAX / 2).is_some());
    }

    #[test]
    fn invalid_keyframes_are_rejected_and_ordered() {
        let mut spec = UiAnimationSpec::EMPTY;
        assert!(spec.push(UiKeyframe::new(100, style(0))));
        assert!(!spec.push(UiKeyframe::new(100, style(1))));
        assert!(!spec.is_valid());
    }
}
