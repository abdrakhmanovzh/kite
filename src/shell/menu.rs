//! Our own popup menu: dropdowns and right-click menus share one look with
//! the palette instead of the kit's fixed-size menu.

use std::rc::Rc;

use gpui_kit::{assets::IconName, component::Icon, prelude::*, *};

use super::{Messenger, widgets};
use crate::{
    telegram::{Command, Reaction},
    ui,
};

pub(super) type MenuAction = Rc<dyn Fn(&mut Messenger, &mut Window, &mut Context<Messenger>)>;

pub(super) enum MenuEntry {
    Item {
        icon: IconName,
        label: SharedString,
        danger: bool,
        shortcut: Option<&'static str>,
        action: MenuAction,
    },
    Separator,
    /// Quick reactions for a message, shown as a row of emoji.
    Reactions {
        chat_id: i64,
        message_id: i64,
        chosen: Vec<String>,
    },
}

impl MenuEntry {
    pub(super) fn item(
        icon: IconName,
        label: impl Into<SharedString>,
        action: impl Fn(&mut Messenger, &mut Window, &mut Context<Messenger>) + 'static,
    ) -> Self {
        Self::Item {
            icon,
            label: label.into(),
            danger: false,
            shortcut: None,
            action: Rc::new(action),
        }
    }

    pub(super) fn shortcut(mut self, keys: &'static str) -> Self {
        if let Self::Item { shortcut, .. } = &mut self {
            *shortcut = Some(keys);
        }
        self
    }

    pub(super) fn danger(mut self) -> Self {
        if let Self::Item { danger, .. } = &mut self {
            *danger = true;
        }
        self
    }
}

pub(super) struct OpenMenu {
    position: Point<Pixels>,
    anchor: Anchor,
    entries: Vec<MenuEntry>,
}

const REACTIONS: [&str; 7] = ["👍", "❤️", "🔥", "😁", "😮", "😢", "🙏"];
/// Reactions in the quick row (a "more" button makes it one cell wider).
const ROW: usize = REACTIONS.len();
const CELL: f32 = 34.;
const CELL_GAP: f32 = 2.;

/// One reaction button; chosen ones are tinted and remove on click.
fn reaction_cell(
    index: usize,
    emoji: String,
    chosen: &[String],
    chat_id: i64,
    message_id: i64,
    cx: &mut Context<Messenger>,
) -> Stateful<Div> {
    let remove = chosen.contains(&emoji);
    div()
        .id(("menu-reaction", index))
        .size(px(CELL))
        .rounded(px(ui::RADIUS_MD))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(19.))
        .cursor_pointer()
        .when(remove, |el| el.bg(ui::hsla_alpha(ui::ACCENT, 0x33)))
        .hover(|s| s.bg(ui::hsla(ui::HOVER_RAISED)))
        .on_click(cx.listener({
            let emoji = emoji.clone();
            move |this, _, _, cx| {
                this.menu = None;
                this.send(Command::React {
                    chat_id,
                    message_id,
                    reaction: Reaction::Emoji(emoji.clone()),
                    remove,
                });
                cx.notify();
            }
        }))
        .child(emoji)
}

impl Messenger {
    pub(super) fn open_menu(
        &mut self,
        position: Point<Pixels>,
        anchor: Anchor,
        entries: Vec<MenuEntry>,
        cx: &mut Context<Self>,
    ) {
        self.menu = Some(OpenMenu {
            position,
            anchor,
            entries,
        });
        self.reactions_expanded = false;
        cx.notify();
    }

    pub(super) fn close_menu(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.menu.take().is_some();
        if open {
            cx.notify();
        }
        open
    }

    pub(super) fn render_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.as_ref()?;
        let mut card = widgets::card()
            .id("menu")
            .min_w(px(220.))
            .p(px(6.))
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.close_menu(cx);
            }));
        // Expanded reactions replace the menu with the full, scrollable list.
        if self.reactions_expanded
            && let Some(MenuEntry::Reactions {
                chat_id,
                message_id,
                chosen,
            }) = menu.entries.first()
        {
            card = card.child(self.reaction_grid(*chat_id, *message_id, chosen, cx));
            return Some(self.place_menu(menu, card));
        }
        // A reactions row with nothing allowed is dropped with its separator.
        let mut skip_separator = false;
        for (index, entry) in menu.entries.iter().enumerate() {
            if std::mem::take(&mut skip_separator) && matches!(entry, MenuEntry::Separator) {
                continue;
            }
            card =
                match entry {
                    MenuEntry::Separator => card.child(
                        div()
                            .mx(px(-6.))
                            .my(px(5.))
                            .h(px(1.))
                            .bg(ui::hsla(ui::STROKE_STRONG)),
                    ),
                    MenuEntry::Reactions {
                        chat_id,
                        message_id,
                        chosen,
                    } => match self.reaction_row(*chat_id, *message_id, chosen, cx) {
                        Some(row) => card.child(row),
                        None => {
                            skip_separator = true;
                            card
                        }
                    },
                    MenuEntry::Item {
                        icon,
                        label,
                        danger,
                        shortcut,
                        action,
                    } => {
                        let action = action.clone();
                        let color = if *danger { ui::DANGER } else { ui::TEXT };
                        card.child(
                            div()
                                .id(("menu-item", index))
                                .h(px(32.))
                                .px(px(10.))
                                .rounded(px(ui::RADIUS_MD))
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .cursor_pointer()
                                .text_size(px(ui::TEXT_MD))
                                .text_color(ui::hsla(color))
                                .hover(|s| {
                                    s.bg(ui::hsla(if *danger {
                                        ui::DANGER_HOVER
                                    } else {
                                        ui::HOVER_RAISED
                                    }))
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.menu = None;
                                    action(this, window, cx);
                                    cx.notify();
                                }))
                                .child(Icon::new(*icon).size(px(ui::ICON_MD)).text_color(ui::hsla(
                                    if *danger { ui::DANGER } else { ui::MUTED },
                                )))
                                .child(div().flex_1().child(label.clone()))
                                .when_some(*shortcut, |el, keys| {
                                    el.child(
                                        div()
                                            .text_size(px(ui::TEXT_SM))
                                            .text_color(ui::hsla(ui::MUTED))
                                            .child(keys),
                                    )
                                }),
                        )
                    }
                };
        }
        Some(self.place_menu(menu, card))
    }

    /// Floats the card at the menu's position, kept inside the window.
    fn place_menu(&self, menu: &OpenMenu, card: Stateful<Div>) -> AnyElement {
        deferred(
            anchored()
                .position(menu.position)
                .anchor(menu.anchor)
                .snap_to_window_with_margin(px(8.))
                .child(card),
        )
        .with_priority(1)
        .into_any_element()
    }

    /// Emoji the user may react with: Telegram's list for this message once
    /// it arrives, until then the chat's allowed reactions (all of ours, a
    /// chat-chosen subset, or none).
    fn reaction_choices(&self, chat_id: i64, message_id: i64) -> Vec<String> {
        if let Some(((chat, message), emoji)) = &self.reaction_options
            && (*chat, *message) == (chat_id, message_id)
        {
            return emoji.clone();
        }
        match self
            .chats
            .get(&chat_id)
            .map(|c| &c.info.available_reactions)
        {
            Some(tdlib_rs::enums::ChatAvailableReactions::Some(some)) => some
                .reactions
                .iter()
                .filter_map(|r| match r {
                    tdlib_rs::enums::ReactionType::Emoji(e) => Some(e.emoji.clone()),
                    _ => None,
                })
                .collect(),
            _ => REACTIONS.iter().map(|e| (*e).to_owned()).collect(),
        }
    }

    /// The quick row: the first few allowed reactions, and a button opening
    /// the full list when there are more.
    fn reaction_row(
        &self,
        chat_id: i64,
        message_id: i64,
        chosen: &[String],
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let choices = self.reaction_choices(chat_id, message_id);
        if choices.is_empty() {
            return None;
        }
        let more = choices.len() > ROW;
        Some(
            div()
                .flex()
                .gap(px(CELL_GAP))
                .pb(px(2.))
                .children(
                    choices
                        .into_iter()
                        .take(ROW)
                        .enumerate()
                        .map(|(index, emoji)| {
                            reaction_cell(index, emoji, chosen, chat_id, message_id, cx)
                        }),
                )
                .when(more, |el| {
                    el.child(
                        div()
                            .id("menu-reactions-more")
                            .size(px(CELL))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .bg(ui::hsla(ui::RAISED))
                            .hover(|s| s.bg(ui::hsla(ui::HOVER_RAISED)))
                            .child(
                                Icon::new(IconName::ChevronDown)
                                    .size(px(ui::ICON_MD))
                                    .text_color(ui::hsla(ui::SECONDARY)),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.reactions_expanded = true;
                                cx.notify();
                            })),
                    )
                }),
        )
    }

    /// Every allowed reaction, in rows as wide as the quick row; scrolls.
    fn reaction_grid(
        &self,
        chat_id: i64,
        message_id: i64,
        chosen: &[String],
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let choices = self.reaction_choices(chat_id, message_id);
        let width = CELL * (ROW + 1) as f32 + CELL_GAP * ROW as f32;
        div()
            .id("menu-reaction-grid")
            .w(px(width))
            .max_h(px(CELL * 6. + CELL_GAP * 5.))
            .overflow_y_scroll()
            .child(div().flex().flex_wrap().gap(px(CELL_GAP)).children(
                choices.into_iter().enumerate().map(|(index, emoji)| {
                    reaction_cell(index, emoji, chosen, chat_id, message_id, cx)
                }),
            ))
    }
}
