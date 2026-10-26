//! Shared primitives built from the design tokens in `ui`. Views compose
//! these instead of styling controls one by one.

use gpui_kit::{
    assets::IconName,
    component::{
        Icon,
        button::{Button, ButtonVariants},
    },
    prelude::*,
    *,
};

use crate::ui::{self, hsla, hsla_alpha};

/// The Kite logo (blue circle with the kite), decoded once.
pub fn logo(size: f32) -> Img {
    static LOGO: std::sync::LazyLock<std::sync::Arc<Image>> = std::sync::LazyLock::new(|| {
        std::sync::Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../../assets/icon/logo.png").to_vec(),
        ))
    });
    img(LOGO.clone()).size(px(size)).flex_shrink_0()
}

/// Square ghost button with a single icon: toolbars, headers, panels.
/// The kit derives the glyph size from the button size, so use its named
/// sizes (default: 32px box, 16px glyph) rather than pixel sizes.
pub fn icon_button(id: impl Into<ElementId>, icon: IconName) -> Button {
    Button::new(id)
        .ghost()
        .rounded(px(ui::RADIUS_MD))
        .icon(Icon::new(icon).text_color(hsla(ui::SECONDARY)))
}

/// Text field container: place an `Input` with `.appearance(false)` inside.
pub fn field() -> Div {
    div()
        .h(px(ui::CONTROL_MD))
        .px(px(10.))
        .rounded(px(ui::RADIUS_MD))
        .bg(hsla(ui::RAISED))
        .border_1()
        .border_color(hsla(ui::STROKE_STRONG))
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(ui::TEXT_MD))
}

/// Leading search glyph for fields.
pub fn field_icon(icon: IconName) -> Icon {
    Icon::new(icon)
        .size(px(ui::ICON_MD))
        .text_color(hsla(ui::PLACEHOLDER))
}

pub enum Tone {
    Accent,
    Muted,
    /// On a selected (accent-filled) row.
    Inverted,
}

/// Unread counters and similar small counts.
pub fn badge(text: impl Into<SharedString>, tone: Tone) -> Div {
    let (bg, fg) = match tone {
        Tone::Accent => (ui::BADGE, ui::WHITE),
        Tone::Muted => (ui::BADGE_MUTED, ui::WHITE),
        Tone::Inverted => (ui::SELECTED_TEXT, ui::SELECTED),
    };
    div()
        .flex_shrink_0()
        .h(px(20.))
        .min_w(px(20.))
        .px(px(6.))
        .rounded_full()
        .bg(hsla(bg))
        .text_color(hsla(fg))
        .text_size(px(ui::TEXT_XS))
        .font_weight(FontWeight::SEMIBOLD)
        .flex()
        .items_center()
        .justify_center()
        .child(text.into())
}

/// Centered informational pill: day separators, service events, empty states.
pub fn pill(text: impl Into<SharedString>) -> Div {
    div()
        .max_w(px(420.))
        .px(px(12.))
        .py(px(4.))
        .rounded_full()
        .bg(hsla_alpha(ui::SERVICE, 0xe6))
        .text_size(px(ui::TEXT_SM))
        .font_weight(FontWeight::MEDIUM)
        .text_color(hsla(ui::SERVICE_TEXT))
        .child(text.into())
}

/// Selectable rounded chip: topics, sticker packs.
pub fn chip(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_shrink_0()
        .h(px(ui::CONTROL_SM))
        .px(px(12.))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(ui::TEXT_SM))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .when(active, |el| {
            el.bg(hsla(ui::SELECTED))
                .text_color(hsla(ui::SELECTED_TEXT))
        })
        .when(!active, |el| {
            el.bg(hsla(ui::RAISED))
                .text_color(hsla(ui::SECONDARY))
                .hover(|s| s.bg(hsla(ui::HOVER_RAISED)))
        })
        .child(label.into())
}

/// Navigation tab: quiet text, raised pill when active.
pub fn tab(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_shrink_0()
        .h(px(ui::CONTROL_SM))
        .px(px(12.))
        .rounded(px(ui::RADIUS_MD))
        .flex()
        .items_center()
        .text_size(px(ui::TEXT_SM))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .when(active, |el| {
            el.bg(hsla(ui::RAISED)).text_color(hsla(ui::TEXT))
        })
        .when(!active, |el| {
            el.text_color(hsla(ui::MUTED))
                .hover(|s| s.bg(hsla(ui::HOVER)).text_color(hsla(ui::SECONDARY)))
        })
        .child(label.into())
}

/// Keyboard key cap.
pub fn kbd(label: impl Into<SharedString>) -> Div {
    div()
        .h(px(20.))
        .min_w(px(20.))
        .px(px(5.))
        .rounded(px(ui::RADIUS_SM))
        .bg(hsla(ui::RAISED))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(ui::TEXT_XS))
        .font_weight(FontWeight::MEDIUM)
        .text_color(hsla(ui::SECONDARY))
        .child(label.into())
}

/// Header of a side panel: title on the gutter, a trailing close button.
pub fn panel_header(title: impl Into<SharedString>, close: Button) -> Div {
    div()
        .h(px(ui::HEADER))
        .flex_shrink_0()
        .pl(px(ui::GUTTER))
        .pr(px(ui::GUTTER - 8.))
        .flex()
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(hsla(ui::STROKE))
        .child(
            div()
                .text_size(px(ui::TEXT_LG))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.into()),
        )
        .child(close)
}

/// Rounded search field: sticker search, in-chat search.
pub fn search_field() -> Div {
    div()
        .h(px(ui::CONTROL_MD))
        .pl(px(12.))
        .pr(px(4.))
        .rounded_full()
        .bg(hsla(ui::RAISED))
        .flex()
        .items_center()
        .gap(px(8.))
        .child(field_icon(IconName::Search))
}

/// A quoted message: accent bar and tint, the sender or action as title,
/// then a one-line preview. Reply quotes in bubbles and the composer's
/// reply/edit/forward preview share it.
pub fn quote(
    accent: ui::Color,
    title: impl Into<SharedString>,
    preview: impl Into<SharedString>,
    preview_color: ui::Color,
) -> Div {
    div()
        .min_w_0()
        .rounded(px(ui::RADIUS_SM))
        .overflow_hidden()
        .bg(hsla_alpha(accent, 0x1f))
        .border_l(px(3.))
        .border_color(hsla(accent))
        .pl(px(8.))
        .pr(px(8.))
        .py(px(3.))
        .child(
            div()
                .text_size(px(ui::TEXT_SM))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(hsla(accent))
                .truncate()
                .child(title.into()),
        )
        .child(
            div()
                .text_size(px(ui::TEXT_SM))
                .text_color(hsla(preview_color))
                .truncate()
                .child(preview.into()),
        )
}

/// Small uppercase heading above a group of rows; aligned with the
/// content below it, so callers add no horizontal padding.
pub fn section_label(label: impl Into<SharedString>) -> Div {
    div()
        .pt(px(10.))
        .pb(px(8.))
        .text_size(px(ui::TEXT_XS))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(hsla(ui::MUTED))
        .child(label.into().to_uppercase())
}

/// Floating layer: menus' look, applied to our own popovers and palette.
pub fn card() -> Div {
    div()
        .rounded(px(ui::RADIUS_LG))
        .bg(hsla(ui::OVERLAY))
        .border_1()
        .border_color(hsla(ui::STROKE_STRONG))
        .shadow_xl()
}

/// Rounded square holding an icon, used as a leading visual in rows.
pub fn icon_tile(icon: IconName, size: f32) -> Div {
    div()
        .size(px(size))
        .flex_shrink_0()
        .rounded(px(ui::RADIUS_MD))
        .bg(hsla(ui::RAISED))
        .flex()
        .items_center()
        .justify_center()
        .child(
            Icon::new(icon)
                .size(px(ui::ICON_MD))
                .text_color(hsla(ui::SECONDARY)),
        )
}

/// Labelled hint like "↵ open".
pub fn hint(key: &'static str, label: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(ui::TEXT_SM))
        .text_color(hsla(ui::MUTED))
        .child(kbd(key))
        .child(label)
}
