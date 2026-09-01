use core::fmt;

use crate::{
    UiAnimationPreset, UiBinding, UiBindingProperty, UiBuild, UiConditionalStyle, UiEasing,
    UiEventKind, UiExpression, UiName, UiNodeKind, UiNodeTemplate, UiStateStyle, UiStyle,
    UiStyleState,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiCodegenError {
    InvalidBuild,
    Output,
}

pub fn write_rust<W: fmt::Write>(build: &UiBuild, output: &mut W) -> Result<(), UiCodegenError> {
    if !build.is_valid() {
        return Err(UiCodegenError::InvalidBuild);
    }

    write_line(output, "pub fn build() -> logos_ui_compiler::UiBuild {")?;
    write_line(output, "    let mut document = logos_ui::UiDocument::EMPTY;")?;
    for index in 0..build.document.node_count() {
        let Some(node) = build.document.node(index) else {
            return Err(UiCodegenError::InvalidBuild);
        };
        write_node(output, index, node)?;
    }
    write_line(output, "    logos_ui_compiler::UiBuild::from_document(document)")?;
    write_line(output, "}")?;
    Ok(())
}

fn write_node<W: fmt::Write>(
    output: &mut W,
    index: usize,
    node: &UiNodeTemplate,
) -> Result<(), UiCodegenError> {
    write_line(
        output,
        format_line(index, "bindings", "logos_ui::UiBindingList::EMPTY", node.bindings.len != 0),
    )?;
    for binding in node.bindings.entries.iter().take(usize::from(node.bindings.len)) {
        write_binding(output, index, binding)?;
    }

    write_line(
        output,
        format_line(index, "styles", "logos_ui::UiStyleList::EMPTY", node.styles.len != 0),
    )?;
    for style in node.styles.tokens.iter().take(usize::from(node.styles.len)) {
        write_style_push(output, index, "styles", *style)?;
    }

    write_line(
        output,
        format_line(
            index,
            "state_styles",
            "logos_ui::UiStateStyleList::EMPTY",
            node.state_styles.len != 0,
        ),
    )?;
    for state_style in node.state_styles.entries.iter().take(usize::from(node.state_styles.len)) {
        write_state_style(output, index, state_style)?;
    }

    write_line(
        output,
        format_line(
            index,
            "conditional_styles",
            "logos_ui::UiConditionalStyleList::EMPTY",
            node.conditional_styles.len != 0,
        ),
    )?;
    for conditional in
        node.conditional_styles.entries.iter().take(usize::from(node.conditional_styles.len))
    {
        write_conditional_style(output, index, conditional)?;
    }

    if node.event.is_present() {
        write!(output, "    let node_{index}_event = logos_ui::UiEvent {{ kind: ")
            .map_err(|_| UiCodegenError::Output)?;
        write_event_kind(output, node.event.kind)?;
        write!(output, ", handler: ").map_err(|_| UiCodegenError::Output)?;
        write_expression(output, node.event.handler)?;
        write_line(output, " };")?;
    } else {
        write_line(output, format_line(index, "event", "logos_ui::UiEvent::EMPTY", false))?;
    }

    write_animation(output, index, node)?;

    write!(output, "    let node_{index} = logos_ui::UiNodeTemplate {{ kind: ")
        .map_err(|_| UiCodegenError::Output)?;
    write_node_kind(output, node.kind)?;
    write!(output, ", parent: ").map_err(|_| UiCodegenError::Output)?;
    if node.parent == u16::MAX {
        write!(output, "u16::MAX").map_err(|_| UiCodegenError::Output)?;
    } else {
        write!(output, "{}", node.parent).map_err(|_| UiCodegenError::Output)?;
    }
    write!(output, ", key: ").map_err(|_| UiCodegenError::Output)?;
    write_name(output, node.key)?;
    write!(output, ", text: ").map_err(|_| UiCodegenError::Output)?;
    write_text(output, node.text.as_bytes())?;
    write!(output, ", text_binding: ").map_err(|_| UiCodegenError::Output)?;
    write_expression(output, node.text_binding)?;
    write!(
        output,
        ", bindings: node_{index}_bindings, event: node_{index}_event, styles: node_{index}_styles, state_styles: node_{index}_state_styles, conditional_styles: node_{index}_conditional_styles, tab_index: {}i16, animation: node_{index}_animation }};",
        node.tab_index
    )
    .map_err(|_| UiCodegenError::Output)?;
    write_line(output, "")?;
    writeln!(
        output,
        "    document.push_node(node_{index}).expect(\"generated UI node capacity\");"
    )
    .map_err(|_| UiCodegenError::Output)?;
    Ok(())
}

fn write_animation<W: fmt::Write>(
    output: &mut W,
    index: usize,
    node: &UiNodeTemplate,
) -> Result<(), UiCodegenError> {
    if node.animation.keyframe_count == 0 {
        return write_line(
            output,
            format_line(index, "animation", "logos_ui::UiAnimationSpec::EMPTY", false),
        );
    }
    write_line(output, format_line(index, "animation", "logos_ui::UiAnimationSpec::EMPTY", true))?;
    writeln!(output, "    node_{index}_animation.duration_ms = {};", node.animation.duration_ms)
        .map_err(|_| UiCodegenError::Output)?;
    writeln!(output, "    node_{index}_animation.delay_ms = {};", node.animation.delay_ms)
        .map_err(|_| UiCodegenError::Output)?;
    writeln!(output, "    node_{index}_animation.repeat = {};", node.animation.repeat)
        .map_err(|_| UiCodegenError::Output)?;
    writeln!(
        output,
        "    node_{index}_animation.direction = {};",
        direction_expr(node.animation.direction)
    )
    .map_err(|_| UiCodegenError::Output)?;
    writeln!(output, "    node_{index}_animation.fill = {};", fill_expr(node.animation.fill))
        .map_err(|_| UiCodegenError::Output)?;
    write!(output, "    node_{index}_animation.easing = ").map_err(|_| UiCodegenError::Output)?;
    write_easing(output, node.animation.easing)?;
    write_line(output, ";")?;
    for keyframe in node.animation.keyframes.iter().take(usize::from(node.animation.keyframe_count))
    {
        write!(
            output,
            "    assert!(node_{index}_animation.push(logos_ui::UiKeyframe {{ offset_q16: {}, properties: {}, style: logos_ui::UiComputedStyle {{ fill_color: {}, text_color: {}, opacity_q16: {}, transform: logos_ui::UiTransform {{ translate_x: {}, translate_y: {}, scale_q8_8: {}, rotation_degrees: {} }} }}, easing: ",
            keyframe.offset_q16,
            keyframe.properties,
            keyframe.style.fill_color,
            keyframe.style.text_color,
            keyframe.style.opacity_q16,
            keyframe.style.transform.translate_x,
            keyframe.style.transform.translate_y,
            keyframe.style.transform.scale_q8_8,
            keyframe.style.transform.rotation_degrees,
        )
        .map_err(|_| UiCodegenError::Output)?;
        write_easing(output, keyframe.easing)?;
        write_line(output, " }));")?;
    }
    Ok(())
}

fn direction_expr(direction: logos_ui::UiAnimationDirection) -> &'static str {
    match direction {
        logos_ui::UiAnimationDirection::Normal => "logos_ui::UiAnimationDirection::Normal",
        logos_ui::UiAnimationDirection::Reverse => "logos_ui::UiAnimationDirection::Reverse",
        logos_ui::UiAnimationDirection::Alternate => "logos_ui::UiAnimationDirection::Alternate",
        logos_ui::UiAnimationDirection::AlternateReverse => {
            "logos_ui::UiAnimationDirection::AlternateReverse"
        }
    }
}

fn fill_expr(fill: logos_ui::UiAnimationFill) -> &'static str {
    match fill {
        logos_ui::UiAnimationFill::None => "logos_ui::UiAnimationFill::None",
        logos_ui::UiAnimationFill::Forwards => "logos_ui::UiAnimationFill::Forwards",
        logos_ui::UiAnimationFill::Backwards => "logos_ui::UiAnimationFill::Backwards",
        logos_ui::UiAnimationFill::Both => "logos_ui::UiAnimationFill::Both",
    }
}

fn write_binding<W: fmt::Write>(
    output: &mut W,
    index: usize,
    binding: &UiBinding,
) -> Result<(), UiCodegenError> {
    write!(output, "    assert!(node_{index}_bindings.push(logos_ui::UiBinding {{ property: ")
        .map_err(|_| UiCodegenError::Output)?;
    write_binding_property(output, binding.property)?;
    write!(output, ", expression: ").map_err(|_| UiCodegenError::Output)?;
    write_expression(output, binding.expression)?;
    write_line(output, " }));")
}

fn write_style_push<W: fmt::Write>(
    output: &mut W,
    index: usize,
    field: &str,
    style: UiStyle,
) -> Result<(), UiCodegenError> {
    write!(output, "    assert!(node_{index}_{field}.push(").map_err(|_| UiCodegenError::Output)?;
    write_style(output, style)?;
    write_line(output, "));")
}

fn write_state_style<W: fmt::Write>(
    output: &mut W,
    index: usize,
    state_style: &UiStateStyle,
) -> Result<(), UiCodegenError> {
    write!(output, "    assert!(node_{index}_state_styles.push(logos_ui::UiStateStyle {{ state: ")
        .map_err(|_| UiCodegenError::Output)?;
    write_state(output, state_style.state)?;
    write!(output, ", style: ").map_err(|_| UiCodegenError::Output)?;
    write_style(output, state_style.style)?;
    write_line(output, " }));")
}

fn write_conditional_style<W: fmt::Write>(
    output: &mut W,
    index: usize,
    conditional: &UiConditionalStyle,
) -> Result<(), UiCodegenError> {
    write!(
        output,
        "    assert!(node_{index}_conditional_styles.push(logos_ui::UiConditionalStyle {{ style: "
    )
    .map_err(|_| UiCodegenError::Output)?;
    write_style(output, conditional.style)?;
    write!(output, ", expression: ").map_err(|_| UiCodegenError::Output)?;
    write_expression(output, conditional.expression)?;
    write_line(output, " }));")
}

fn format_line<'a>(index: usize, field: &'a str, value: &'a str, mutable: bool) -> StringLine<'a> {
    StringLine { index, field, value, mutable }
}

struct StringLine<'a> {
    index: usize,
    field: &'a str,
    value: &'a str,
    mutable: bool,
}

impl fmt::Display for StringLine<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "    let {}node_{}_{} = {};",
            if self.mutable { "mut " } else { "" },
            self.index,
            self.field,
            self.value
        )
    }
}

fn write_line<W: fmt::Write, D: fmt::Display>(
    output: &mut W,
    line: D,
) -> Result<(), UiCodegenError> {
    write!(output, "{line}").map_err(|_| UiCodegenError::Output)?;
    output.write_char('\n').map_err(|_| UiCodegenError::Output)
}

fn write_binding_property<W: fmt::Write>(
    output: &mut W,
    property: UiBindingProperty,
) -> Result<(), UiCodegenError> {
    let name = match property {
        UiBindingProperty::Text => "Text",
        UiBindingProperty::Styles => "Styles",
        UiBindingProperty::Value => "Value",
        UiBindingProperty::Disabled => "Disabled",
        UiBindingProperty::Form => "Form",
        UiBindingProperty::Control => "Control",
        UiBindingProperty::CanSubmit => "CanSubmit",
    };
    write!(output, "logos_ui::UiBindingProperty::{name}").map_err(|_| UiCodegenError::Output)
}

fn write_event_kind<W: fmt::Write>(
    output: &mut W,
    kind: UiEventKind,
) -> Result<(), UiCodegenError> {
    let name = match kind {
        UiEventKind::Click => "Click",
        UiEventKind::Submit => "Submit",
        UiEventKind::Changed => "Changed",
    };
    write!(output, "logos_ui::UiEventKind::{name}").map_err(|_| UiCodegenError::Output)
}

fn write_node_kind<W: fmt::Write>(output: &mut W, kind: UiNodeKind) -> Result<(), UiCodegenError> {
    let name = match kind {
        UiNodeKind::Root => "Root",
        UiNodeKind::Panel => "Panel",
        UiNodeKind::Label => "Label",
        UiNodeKind::Button => "Button",
        UiNodeKind::TextInput => "TextInput",
        UiNodeKind::Form => "Form",
    };
    write!(output, "logos_ui::UiNodeKind::{name}").map_err(|_| UiCodegenError::Output)
}

fn write_state<W: fmt::Write>(output: &mut W, state: UiStyleState) -> Result<(), UiCodegenError> {
    match state {
        UiStyleState::Hover => write!(output, "logos_ui::UiStyleState::Hover"),
        UiStyleState::Focus => write!(output, "logos_ui::UiStyleState::Focus"),
        UiStyleState::Pressed => write!(output, "logos_ui::UiStyleState::Pressed"),
        UiStyleState::Disabled => write!(output, "logos_ui::UiStyleState::Disabled"),
    }
    .map_err(|_| UiCodegenError::Output)
}

fn write_style<W: fmt::Write>(output: &mut W, style: UiStyle) -> Result<(), UiCodegenError> {
    match style {
        UiStyle::FullHeight => write!(output, "logos_ui::UiStyle::FullHeight"),
        UiStyle::FullWidth => write!(output, "logos_ui::UiStyle::FullWidth"),
        UiStyle::FlexX => write!(output, "logos_ui::UiStyle::FlexX"),
        UiStyle::FlexY => write!(output, "logos_ui::UiStyle::FlexY"),
        UiStyle::ItemsCenter => write!(output, "logos_ui::UiStyle::ItemsCenter"),
        UiStyle::JustifyCenter => write!(output, "logos_ui::UiStyle::JustifyCenter"),
        UiStyle::Width96 => write!(output, "logos_ui::UiStyle::Width96"),
        UiStyle::Gap(value) => write!(output, "logos_ui::UiStyle::Gap({value})"),
        UiStyle::GapX(value) => write!(output, "logos_ui::UiStyle::GapX({value})"),
        UiStyle::GapY(value) => write!(output, "logos_ui::UiStyle::GapY({value})"),
        UiStyle::MarginTop4 => write!(output, "logos_ui::UiStyle::MarginTop4"),
        UiStyle::MarginBottom2 => write!(output, "logos_ui::UiStyle::MarginBottom2"),
        UiStyle::PaddingX6 => write!(output, "logos_ui::UiStyle::PaddingX6"),
        UiStyle::PaddingY3 => write!(output, "logos_ui::UiStyle::PaddingY3"),
        UiStyle::RoundedLarge => write!(output, "logos_ui::UiStyle::RoundedLarge"),
        UiStyle::RoundedFull => write!(output, "logos_ui::UiStyle::RoundedFull"),
        UiStyle::BackgroundAccent => write!(output, "logos_ui::UiStyle::BackgroundAccent"),
        UiStyle::TextMuted => write!(output, "logos_ui::UiStyle::TextMuted"),
        UiStyle::Text4xl => write!(output, "logos_ui::UiStyle::Text4xl"),
        UiStyle::FontLight => write!(output, "logos_ui::UiStyle::FontLight"),
        UiStyle::Opacity50 => write!(output, "logos_ui::UiStyle::Opacity50"),
        UiStyle::TransitionColors => write!(output, "logos_ui::UiStyle::TransitionColors"),
        UiStyle::TransitionOpacity => write!(output, "logos_ui::UiStyle::TransitionOpacity"),
        UiStyle::TransitionTransform => {
            write!(output, "logos_ui::UiStyle::TransitionTransform")
        }
        UiStyle::TransitionAll => write!(output, "logos_ui::UiStyle::TransitionAll"),
        UiStyle::Duration(value) => write!(output, "logos_ui::UiStyle::Duration({value})"),
        UiStyle::Delay(value) => write!(output, "logos_ui::UiStyle::Delay({value})"),
        UiStyle::Ease(value) => {
            write!(output, "logos_ui::UiStyle::Ease(").map_err(|_| UiCodegenError::Output)?;
            write_easing(output, value)?;
            write!(output, ")")
        }
        UiStyle::Animation(preset) => match preset {
            UiAnimationPreset::In => {
                write!(output, "logos_ui::UiStyle::Animation(logos_ui::UiAnimationPreset::In)")
            }
            UiAnimationPreset::Pulse => {
                write!(output, "logos_ui::UiStyle::Animation(logos_ui::UiAnimationPreset::Pulse)")
            }
            UiAnimationPreset::Spin => {
                write!(output, "logos_ui::UiStyle::Animation(logos_ui::UiAnimationPreset::Spin)")
            }
        },
    }
    .map_err(|_| UiCodegenError::Output)
}

fn write_easing<W: fmt::Write>(output: &mut W, easing: UiEasing) -> Result<(), UiCodegenError> {
    let name = match easing {
        UiEasing::Linear => "Linear",
        UiEasing::EaseIn => "EaseIn",
        UiEasing::EaseOut => "EaseOut",
        UiEasing::EaseInOut => "EaseInOut",
        UiEasing::EaseInQuad => "EaseInQuad",
        UiEasing::EaseOutQuad => "EaseOutQuad",
        UiEasing::EaseInOutQuad => "EaseInOutQuad",
        UiEasing::EaseInCubic => "EaseInCubic",
        UiEasing::EaseOutCubic => "EaseOutCubic",
        UiEasing::EaseInOutCubic => "EaseInOutCubic",
        UiEasing::EaseOutBack => "EaseOutBack",
        UiEasing::CubicBezier { x1, y1, x2, y2 } => {
            return write!(
                output,
                "logos_ui::UiEasing::CubicBezier {{ x1: {x1}, y1: {y1}, x2: {x2}, y2: {y2} }}"
            )
            .map_err(|_| UiCodegenError::Output);
        }
    };
    write!(output, "logos_ui::UiEasing::{name}").map_err(|_| UiCodegenError::Output)
}

fn write_name<W: fmt::Write>(output: &mut W, name: UiName) -> Result<(), UiCodegenError> {
    if name.is_empty() {
        output.write_str("logos_ui::UiName::EMPTY").map_err(|_| UiCodegenError::Output)
    } else {
        output.write_str("logos_ui::UiName::from_bytes(").map_err(|_| UiCodegenError::Output)?;
        write_bytes(output, name.as_bytes())?;
        write!(output, ").expect(\"generated UI name\")").map_err(|_| UiCodegenError::Output)
    }
}

fn write_text<W: fmt::Write>(output: &mut W, text: &[u8]) -> Result<(), UiCodegenError> {
    output.write_str("logos_ui::UiText::from_bytes(").map_err(|_| UiCodegenError::Output)?;
    write_bytes(output, text)?;
    write!(output, ").expect(\"generated UI text\")").map_err(|_| UiCodegenError::Output)
}

fn write_expression<W: fmt::Write>(
    output: &mut W,
    expression: UiExpression,
) -> Result<(), UiCodegenError> {
    if expression.as_bytes().is_empty() {
        output.write_str("logos_ui::UiExpression::EMPTY").map_err(|_| UiCodegenError::Output)
    } else {
        output
            .write_str("logos_ui::UiExpression::from_bytes(")
            .map_err(|_| UiCodegenError::Output)?;
        write_bytes(output, expression.as_bytes())?;
        write!(output, ").expect(\"generated UI expression\")").map_err(|_| UiCodegenError::Output)
    }
}

fn write_bytes<W: fmt::Write>(output: &mut W, bytes: &[u8]) -> Result<(), UiCodegenError> {
    output.write_str("b\"").map_err(|_| UiCodegenError::Output)?;
    for byte in bytes {
        match *byte {
            b'\\' => output.write_str("\\\\"),
            b'"' => output.write_str("\\\""),
            b'\n' => output.write_str("\\n"),
            b'\r' => output.write_str("\\r"),
            b'\t' => output.write_str("\\t"),
            0x20..=0x7e => output.write_char(*byte as char),
            value => write!(output, "\\x{value:02x}"),
        }
        .map_err(|_| UiCodegenError::Output)?;
    }
    output.write_char('"').map_err(|_| UiCodegenError::Output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_login_builder_contains_no_template_parser() {
        let build = crate::compile_login_page();
        let mut generated = std::string::String::new();
        write_rust(&build, &mut generated).unwrap();

        assert!(generated.starts_with("pub fn build()"));
        assert!(generated.contains("logos_ui::UiDocument::EMPTY"));
        assert!(generated.contains("logos_ui::UiStyle::FlexY"));
        assert!(!generated.contains("compile_login_page"));
        assert!(!generated.contains("include_str"));
    }
}
