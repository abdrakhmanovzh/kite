//! Cmd+K command palette: jump to chats and run quick actions.

use gpui_kit::{
    assets::IconName,
    component::{Icon, input::Input},
    prelude::*,
    *,
};

use super::{Messenger, widgets};
use crate::{
    model::{Chat, List, preview},
    telegram::Command,
    ui,
};

const MAX_CHATS: usize = 12;
/// Two-line rows need a little more than a control's height.
const ROW: f32 = 46.;
const CARD: ui::Color = ui::OVERLAY;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum PaletteItem {
    Chat(i64),
    Saved,
    Open(List),
}

/// Keys the palette handles before any focused input sees them.
pub(super) enum PaletteKey {
    Toggle,
    Up,
    Down,
    Confirm,
    Close,
}

impl PaletteKey {
    pub(super) fn from(keystroke: &Keystroke) -> Option<Self> {
        let m = &keystroke.modifiers;
        if keystroke.key == "k" && m.platform && !m.shift && !m.alt && !m.control {
            return Some(Self::Toggle);
        }
        if m.platform || m.alt || m.control {
            return None;
        }
        Some(match keystroke.key.as_str() {
            "up" => Self::Up,
            "down" => Self::Down,
            "enter" => Self::Confirm,
            "escape" => Self::Close,
            _ => return None,
        })
    }
}

impl Messenger {
    pub(super) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = true;
        self.palette_index = 0;
        // Start every session scrolled to the top.
        self.palette_scroll = ScrollHandle::new();
        self.search_results = None;
        self.search.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = false;
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.search_results = None;
        if let Some(input) = self.selected.and_then(|id| self.composers.get(&id)) {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    /// Returns whether the key was consumed.
    pub(super) fn palette_key(
        &mut self,
        key: PaletteKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.login != super::Login::Ready {
            return false;
        }
        // Esc closes an open menu before anything else.
        if matches!(key, PaletteKey::Close) && self.close_menu(cx) {
            return true;
        }
        match key {
            PaletteKey::Toggle if self.palette_open => self.close_palette(window, cx),
            PaletteKey::Toggle => self.open_palette(window, cx),
            PaletteKey::Close if !self.palette_open => return self.escape(window, cx),
            _ if !self.palette_open => return false,
            PaletteKey::Close => self.close_palette(window, cx),
            PaletteKey::Up | PaletteKey::Down => {
                let count = self.palette_items(cx).len();
                if count > 0 {
                    self.palette_index = if matches!(key, PaletteKey::Up) {
                        (self.palette_index + count - 1) % count
                    } else {
                        (self.palette_index + 1) % count
                    };
                    self.reveal_palette_item(cx);
                }
            }
            PaletteKey::Confirm => {
                if let Some(item) = self.palette_items(cx).get(self.palette_index).copied() {
                    self.run_palette_item(item, window, cx);
                }
            }
        }
        true
    }

    fn reveal_palette_item(&self, cx: &mut Context<Self>) {
        // Children are: [chats header] chats… [actions header] actions…
        let items = self.palette_items(cx);
        let has_chats = items.iter().any(|i| matches!(i, PaletteItem::Chat(_)));
        let first_action = items
            .iter()
            .position(|i| !matches!(i, PaletteItem::Chat(_)));
        let child = self.palette_index
            + usize::from(has_chats)
            + usize::from(first_action.is_some_and(|first| self.palette_index >= first));
        // The first row of a section brings its header into view too.
        let starts_section = self.palette_index == 0 || Some(self.palette_index) == first_action;
        self.palette_scroll
            .scroll_to_item(if starts_section { child - 1 } else { child });
        cx.notify();
    }

    fn run_palette_item(&mut self, item: PaletteItem, window: &mut Window, cx: &mut Context<Self>) {
        self.close_palette(window, cx);
        match item {
            PaletteItem::Chat(id) => self.select_chat(id, window, cx),
            PaletteItem::Saved => {
                self.send(Command::SavedMessages);
            }
            PaletteItem::Open(list) => self.switch_list(list, window, cx),
        }
        cx.notify();
    }

    pub(super) fn palette_items(&self, cx: &App) -> Vec<PaletteItem> {
        let query = self.search.read(cx).value().trim().to_lowercase();
        let mut chats: Vec<&Chat> = if query.is_empty() {
            self.chats
                .values()
                .filter(|c| c.order(List::Main) > 0)
                .collect()
        } else {
            self.chats
                .values()
                .filter(|c| {
                    self.chat_title(c).to_lowercase().contains(&query)
                        || self
                            .search_results
                            .as_ref()
                            .is_some_and(|(_, ids)| ids.contains(&c.info.id))
                })
                .collect()
        };
        // Cached: the key lowercases the title, which shouldn't run per comparison.
        chats.sort_by_cached_key(|c| {
            (
                // Title matches first, then by recency in the main list.
                !self.chat_title(c).to_lowercase().starts_with(&query),
                std::cmp::Reverse(c.order(List::Main).max(c.order(List::Archive))),
            )
        });
        let mut items: Vec<PaletteItem> = chats
            .into_iter()
            .take(if query.is_empty() { 6 } else { MAX_CHATS })
            .map(|c| PaletteItem::Chat(c.info.id))
            .collect();
        let mut actions = vec![
            ("saved messages", PaletteItem::Saved),
            ("archived chats", PaletteItem::Open(List::Archive)),
            ("all chats", PaletteItem::Open(List::Main)),
        ];
        actions.extend(self.folders.iter().map(|f| {
            (
                f.name.text.text.as_str(),
                PaletteItem::Open(List::Folder(f.id)),
            )
        }));
        items.extend(
            actions
                .into_iter()
                .filter(|(label, _)| query.is_empty() || label.to_lowercase().contains(&query))
                .map(|(_, item)| item),
        );
        items
    }

    pub(super) fn palette(&self, cx: &mut Context<Self>) -> Div {
        let items = self.palette_items(cx);
        let selected = self.palette_index.min(items.len().saturating_sub(1));
        let first_action = items
            .iter()
            .position(|i| !matches!(i, PaletteItem::Chat(_)));
        let has_chats = items.iter().any(|i| matches!(i, PaletteItem::Chat(_)));
        let query = self.search.read(cx).value().trim().to_owned();
        let headers = usize::from(has_chats) + usize::from(first_action.is_some());
        // Rows are ROW tall, headers ~31px, plus list padding; cap and scroll beyond.
        let list_height = if items.is_empty() {
            80.
        } else {
            (items.len() as f32 * ROW + headers as f32 * 31. + 12.).min(420.)
        };

        let mut list = div()
            .id("palette-results")
            .size_full()
            .track_scroll(&self.palette_scroll)
            .overflow_y_scroll()
            .px(px(6.))
            .pt(px(4.))
            .pb(px(8.));
        if has_chats {
            list = list.child(section(if query.is_empty() { "Recent" } else { "Chats" }));
        }
        for (index, item) in items.iter().copied().enumerate() {
            if Some(index) == first_action {
                list = list.child(section("Actions"));
            }
            list = list.child(self.palette_row(index, item, index == selected, cx));
        }
        if items.is_empty() {
            list = list.child(
                div()
                    .py(px(28.))
                    .flex()
                    .justify_center()
                    .text_color(ui::hsla(ui::MUTED))
                    .child(format!("Nothing found for “{query}”")),
            );
        }

        div()
            .absolute()
            .inset_0()
            .bg(ui::scrim())
            .flex()
            .items_start()
            .justify_center()
            .pt(px(90.))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close_palette(window, cx)),
            )
            .child(
                widgets::card()
                    .id("palette")
                    .w(px(600.))
                    .max_w(relative(0.9))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .h(px(ui::CONTROL_LG + 4.))
                            .flex_shrink_0()
                            .px(px(ui::GUTTER))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .border_b_1()
                            .border_color(ui::hsla(ui::STROKE_STRONG))
                            .child(
                                Icon::new(IconName::Search)
                                    .size(px(ui::ICON_MD))
                                    .text_color(ui::hsla(ui::MUTED)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(ui::TEXT_MD))
                                    .child(Input::new(&self.search).appearance(false).h(px(28.))),
                            )
                            .child(key_hint("esc")),
                    )
                    // The list scrolls inside the card; the footer stays put.
                    .child(
                        div()
                            .relative()
                            .flex_shrink_0()
                            .h(px(list_height))
                            .child(list)
                            .child(edge_fades(self.palette_scroll.clone())),
                    )
                    .child(
                        div()
                            .h(px(40.))
                            .flex_shrink_0()
                            .px(px(ui::GUTTER))
                            .flex()
                            .items_center()
                            .gap(px(18.))
                            .bg(ui::hsla(ui::BASE))
                            // GPUI clips rectangles, so match the card's corners.
                            .rounded_b(px(ui::RADIUS_LG - 1.))
                            .border_t_1()
                            .border_color(ui::hsla(ui::STROKE_STRONG))
                            .text_size(px(ui::TEXT_SM))
                            .text_color(ui::hsla(ui::MUTED))
                            .child(hint_pair("↑↓", "navigate"))
                            .child(hint_pair("↵", "open"))
                            .child(hint_pair("⌘K", "toggle")),
                    ),
            )
    }

    fn palette_row(
        &self,
        index: usize,
        item: PaletteItem,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let row = div()
            .id(("palette-item", index))
            .h(px(ROW))
            .px(px(ui::GUTTER - 6.))
            // Concentric with the card: its radius minus the list's 6px inset.
            .rounded(px(ui::RADIUS_MD))
            .flex()
            .items_center()
            .gap(px(12.))
            .cursor_pointer()
            .when(selected, |el| el.bg(ui::hsla(ui::HOVER_RAISED)))
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if this.palette_index != index {
                    this.palette_index = index;
                    cx.notify();
                }
            }))
            .on_click(
                cx.listener(move |this, _, window, cx| this.run_palette_item(item, window, cx)),
            );
        let action = |icon: IconName, label: String| {
            (
                widgets::icon_tile(icon, ui::AVATAR_SM).into_any_element(),
                label,
                None,
            )
        };
        let (leading, title, subtitle): (AnyElement, String, Option<String>) = match item {
            PaletteItem::Chat(id) => {
                let Some(chat) = self.chats.get(&id) else {
                    return row;
                };
                let (status, _) = self.chat_status(chat);
                let subtitle = if status.is_empty() {
                    chat.info
                        .last_message
                        .as_ref()
                        .map(|m| preview(&m.content).0)
                } else {
                    Some(status)
                };
                (
                    self.chat_avatar(chat, ui::AVATAR_SM),
                    self.chat_title(chat),
                    subtitle,
                )
            }
            PaletteItem::Saved => action(IconName::Bookmark, "Saved Messages".into()),
            PaletteItem::Open(List::Archive) => action(IconName::Archive, "Archived Chats".into()),
            PaletteItem::Open(List::Main) => action(IconName::MessageCircle, "All Chats".into()),
            PaletteItem::Open(List::Folder(id)) => action(
                IconName::Folder,
                self.folders
                    .iter()
                    .find(|f| f.id == id)
                    .map_or_else(|| "Folder".into(), |f| f.name.text.text.clone()),
            ),
        };
        row.child(leading)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(ui::TEXT_MD))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .when_some(subtitle, |el, subtitle| {
                        el.child(
                            div()
                                .truncate()
                                .text_size(px(ui::TEXT_SM))
                                .text_color(ui::hsla(ui::MUTED))
                                .child(subtitle),
                        )
                    }),
            )
            .when(selected, |el| el.child(widgets::kbd("↵")))
    }
}

/// Soft edges over the list hinting at more rows above or below.
///
/// Drawn at paint time: GPUI applies scrolling after render, so only paint
/// sees the position that is actually on screen.
fn edge_fades(scroll: ScrollHandle) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let scrolled = -scroll.offset().y;
            let remaining = scroll.max_offset().y - scrolled;
            let height = px(36.);
            // Revealing a row leaves the list's padding unscrolled; ignore that slack.
            let slack = px(12.);
            let solid = ui::hsla(CARD);
            let clear = ui::hsla_alpha(CARD, 0);
            if scrolled > slack {
                window.paint_quad(fill(
                    Bounds::new(bounds.origin, size(bounds.size.width, height)),
                    linear_gradient(
                        180.,
                        linear_color_stop(solid, 0.),
                        linear_color_stop(clear, 1.),
                    ),
                ));
            }
            if remaining > slack {
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.origin.x, bounds.bottom() - height),
                        size(bounds.size.width, height),
                    ),
                    linear_gradient(
                        180.,
                        linear_color_stop(clear, 0.),
                        linear_color_stop(solid, 1.),
                    ),
                ));
            }
        },
    )
    .absolute()
    .inset_0()
}

fn section(label: &'static str) -> Div {
    widgets::section_label(label).px(px(ui::GUTTER - 6.))
}

fn key_hint(label: &'static str) -> Div {
    widgets::kbd(label)
}

fn hint_pair(key: &'static str, label: &'static str) -> Div {
    widgets::hint(key, label)
}
