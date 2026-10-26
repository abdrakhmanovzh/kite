use std::ops::Range;

use gpui_kit::{
    assets::IconName,
    component::{
        Icon,
        button::{Button, ButtonVariants},
        scroll::ScrollableElement,
    },
    prelude::*,
    *,
};

use super::{Messenger, drag_strip, widgets};
use crate::{
    model::{Chat, List, preview},
    telegram::Command,
    ui,
};

const ROW_HEIGHT: f32 = 64.;
const AVATAR: f32 = ui::AVATAR_LG;
/// Sentinel id for the "Archived Chats" row at the top of the main list.
const ARCHIVE_ROW: i64 = i64::MIN;

impl Messenger {
    pub(super) fn sidebar(&mut self, width: Pixels, cx: &mut Context<Self>) -> Div {
        let mut chats: Vec<&Chat> = self
            .chats
            .values()
            .filter(|chat| chat.order(self.list) > 0)
            .collect();
        chats.sort_unstable_by_key(|chat| std::cmp::Reverse((chat.order(self.list), chat.info.id)));
        let archived = self.chats.values().any(|c| c.order(List::Archive) > 0);
        self.chat_rows.clear();
        if self.list == List::Main && archived {
            self.chat_rows.push(ARCHIVE_ROW);
        }
        self.chat_rows.extend(chats.iter().map(|chat| chat.info.id));
        let count = self.chat_rows.len();
        let empty = count == 0;

        let tabs = (!self.folders.is_empty() && self.list != List::Archive).then(|| {
            std::iter::once((List::Main, "All".to_owned()))
                .chain(
                    self.folders
                        .iter()
                        .map(|f| (List::Folder(f.id), f.name.text.text.clone())),
                )
                .enumerate()
                .map(|(index, (list, label))| {
                    widgets::tab(("folder", index), label, self.list == list).on_click(
                        cx.listener(move |this, _, window, cx| this.switch_list(list, window, cx)),
                    )
                })
                .collect::<Vec<_>>()
        });

        div()
            .relative()
            .w(width)
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            // Traffic lights on the left, actions on the right; the rest drags the window.
            // 48px centers the buttons on the traffic lights (their center is at 24pt).
            .child(
                div()
                    .relative()
                    .h(px(48.))
                    .flex_shrink_0()
                    .pl(px(80.))
                    .pr(px(ui::GUTTER - 8.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(drag_strip("sidebar-drag").h_full())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(ui::TEXT_SM))
                            .text_color(ui::hsla(ui::MUTED))
                            .truncate()
                            .when(self.connection != "Connected", |el| {
                                el.child(self.connection)
                            }),
                    )
                    .child(self.top_actions(cx)),
            )
            .when(self.forward_from.is_some(), |el| {
                el.child(
                    div()
                        .mx_3()
                        .mb_2()
                        .px_3()
                        .py_2()
                        .rounded(px(ui::RADIUS_MD))
                        .bg(ui::hsla_alpha(ui::ACCENT, 0x24))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(Icon::new(IconName::Forward).text_color(ui::hsla(ui::ACCENT)))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(ui::TEXT_SM))
                                .child("Choose a chat to forward to"),
                        )
                        .child(
                            widgets::icon_button("cancel-forward", IconName::X).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.forward_from = None;
                                    cx.notify();
                                }),
                            ),
                        ),
                )
            })
            .when(self.list == List::Archive, |el| {
                let count = self
                    .chats
                    .values()
                    .filter(|c| c.order(List::Archive) > 0)
                    .count();
                el.child(
                    div()
                        .flex_shrink_0()
                        // Same 8px edge as the tabs and row selection.
                        .pl(px(8.))
                        .pr(px(ui::GUTTER))
                        .mb(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            Button::new("archive-back")
                                .ghost()
                                .rounded(px(ui::RADIUS_MD))
                                // Chevron on the gutter, over the avatars.
                                .pl(px(ui::GUTTER - 8.))
                                .pr(px(12.))
                                .icon(
                                    Icon::new(IconName::ChevronLeft)
                                        .text_color(ui::hsla(ui::SECONDARY)),
                                )
                                .label("Archived")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.switch_list(List::Main, window, cx)
                                })),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_size(px(ui::TEXT_SM))
                                .text_color(ui::hsla(ui::MUTED))
                                .child(ui::count_label(count as i32, "chat", "chats")),
                        ),
                )
            })
            .when_some(tabs, |el, tabs| {
                el.child(
                    div()
                        .id("folders")
                        .flex_shrink_0()
                        // Pills line up with the rows' selection below.
                        .px(px(8.))
                        .pb(px(8.))
                        .flex()
                        .gap(px(4.))
                        .overflow_x_scroll()
                        .children(tabs),
                )
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .when(empty, |el| {
                        el.child(
                            div()
                                .pt_10()
                                .flex()
                                .justify_center()
                                .text_color(ui::hsla(ui::MUTED))
                                .child(if self.chats_loading {
                                    "Loading chats…"
                                } else {
                                    "No chats here yet"
                                }),
                        )
                    })
                    .child(
                        uniform_list(
                            "chats",
                            count,
                            cx.processor(|this, range: Range<usize>, _, cx| {
                                this.chat_rows_range(range, cx)
                            }),
                        )
                        .track_scroll(&self.chat_scroll)
                        .size_full(),
                    )
                    .vertical_scrollbar(&self.chat_scroll),
            )
    }

    fn top_actions(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                widgets::icon_button("search-chats", IconName::Search)
                    .tooltip("Search  ⌘K")
                    .on_click(cx.listener(|this, _, window, cx| this.open_palette(window, cx))),
            )
            .child(
                widgets::icon_button("saved-messages", IconName::Bookmark)
                    .tooltip("Saved Messages")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.send(Command::SavedMessages);
                        cx.notify();
                    })),
            )
    }

    fn chat_rows_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        // Infinite scroll: fetch the next page before the user reaches the end.
        if range.end + 12 >= self.chat_rows.len() && !self.chats_loading && !self.chats_end {
            cx.spawn(async move |this, cx| {
                let _ = this.update(cx, |this, cx| this.load_chats(cx));
            })
            .detach();
        }
        range
            .filter_map(|ix| {
                let id = *self.chat_rows.get(ix)?;
                let row = if id == ARCHIVE_ROW {
                    self.archive_row(cx)
                } else {
                    self.chat_row(id, cx)?
                };
                // Rows are inset so their rounded selection floats.
                Some(div().px(px(8.)).py(px(1.)).child(row).into_any_element())
            })
            .collect()
    }

    fn archive_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut archived: Vec<&Chat> = self
            .chats
            .values()
            .filter(|c| c.order(List::Archive) > 0)
            .collect();
        archived.sort_unstable_by_key(|c| std::cmp::Reverse(c.order(List::Archive)));
        let names = archived
            .iter()
            .take(6)
            .map(|c| c.info.title.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let unread = archived
            .iter()
            .filter(|c| c.info.unread_count > 0 || c.info.is_marked_as_unread)
            .count();
        row_shell("archive-row".into(), false)
            .on_click(
                cx.listener(|this, _, window, cx| this.switch_list(List::Archive, window, cx)),
            )
            .child(icon_avatar(IconName::Archive, ui::BADGE_MUTED, AVATAR))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(ui::TEXT_MD))
                            .child("Archived Chats"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(ui::hsla(ui::MUTED))
                                    .child(names),
                            )
                            .when(unread > 0, |el| {
                                el.child(badge(unread.to_string(), true, false))
                            }),
                    ),
            )
            .into_any_element()
    }

    fn chat_row(&self, id: i64, cx: &mut Context<Self>) -> Option<AnyElement> {
        let chat = self.chats.get(&id)?;
        let selected = self.selected == Some(id);
        let muted = chat.info.notification_settings.mute_for > 0;
        let pinned = chat
            .info
            .positions
            .iter()
            .any(|p| p.list == self.list.td() && p.is_pinned);
        let last = chat.info.last_message.as_ref();
        let meta_color = if selected {
            ui::SELECTED_META
        } else {
            ui::MUTED
        };
        let text_color = if selected {
            ui::SELECTED_TEXT
        } else {
            ui::MUTED
        };

        // Preview line: typing, draft, or "Sender: text".
        let mut prefix: Option<(String, ui::Color)> = None;
        let mut tinted = false;
        let body = if let Some(typing) = self.typing_label(chat) {
            tinted = true;
            typing
        } else if let Some(draft) = chat.info.draft_message.as_ref().filter(|_| !selected)
            && let tdlib_rs::enums::InputMessageContent::InputMessageText(text) =
                &draft.input_message_text
        {
            prefix = Some(("Draft: ".into(), ui::DRAFT));
            text.text.text.replace('\n', " ")
        } else if let Some(message) = last {
            let (text, kind) = preview(&message.content);
            tinted = kind;
            let service = crate::model::is_service(&message.content);
            if service {
                format!("{} {text}", self.sender_name(message))
            } else {
                if message.is_outgoing && !self.is_self_chat(chat) && !self.is_channel(chat) {
                    if !matches!(chat.info.r#type, tdlib_rs::enums::ChatType::Private(_)) {
                        prefix = Some((
                            "You: ".into(),
                            if selected {
                                ui::SELECTED_TEXT
                            } else {
                                ui::ACCENT
                            },
                        ));
                    }
                } else if !message.is_outgoing
                    && !self.is_channel(chat)
                    && !matches!(chat.info.r#type, tdlib_rs::enums::ChatType::Private(_))
                {
                    let name = self.sender_name(message);
                    let first = name.split_whitespace().next().unwrap_or(&name).to_owned();
                    prefix = Some((
                        format!("{first}: "),
                        if selected {
                            ui::SELECTED_TEXT
                        } else {
                            ui::SECONDARY
                        },
                    ));
                }
                text
            }
        } else {
            String::new()
        };

        let ticks = last
            .filter(|m| m.is_outgoing && !self.is_channel(chat))
            .map(|m| match &m.sending_state {
                Some(tdlib_rs::enums::MessageSendingState::Pending(_)) => IconName::Clock,
                Some(tdlib_rs::enums::MessageSendingState::Failed(_)) => IconName::CircleAlert,
                None if self.is_self_chat(chat)
                    || m.id <= chat.info.last_read_outbox_message_id =>
                {
                    IconName::CheckCheck
                }
                None => IconName::Check,
            });
        let unread = chat.info.unread_count;
        let mentions = chat.info.unread_mention_count;

        Some(
            row_shell(("chat", id as u64).into(), selected)
                .on_click(cx.listener(move |this, _, window, cx| this.select_chat(id, window, cx)))
                .child(self.chat_avatar(chat, AVATAR))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_size(px(ui::TEXT_MD))
                                        .child(self.chat_title(chat)),
                                )
                                .when(muted, |el| {
                                    el.child(
                                        Icon::new(IconName::BellOff)
                                            .size(px(ui::ICON_SM))
                                            .text_color(ui::hsla(meta_color)),
                                    )
                                })
                                .child(div().flex_1())
                                .when_some(ticks, |el, icon| {
                                    let color = match icon {
                                        IconName::CircleAlert => ui::DANGER,
                                        _ if selected => ui::SELECTED_TEXT,
                                        _ => ui::ACCENT,
                                    };
                                    el.child(
                                        Icon::new(icon)
                                            .size(px(ui::ICON_SM))
                                            .text_color(ui::hsla(color)),
                                    )
                                })
                                .when_some(last, |el, message| {
                                    el.child(
                                        div()
                                            .flex_shrink_0()
                                            .text_size(px(ui::TEXT_SM))
                                            .text_color(ui::hsla(meta_color))
                                            .child(ui::list_date(message.date)),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(ui::hsla(text_color))
                                        .when_some(prefix, |el, (prefix, color)| {
                                            el.child(
                                                div()
                                                    .flex_none()
                                                    .text_color(ui::hsla(color))
                                                    .child(prefix),
                                            )
                                            .flex()
                                        })
                                        .child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .when(tinted && !selected, |el| {
                                                    el.text_color(ui::hsla(ui::SECONDARY))
                                                })
                                                .when(
                                                    tinted
                                                        && self.typing_label(chat).is_some()
                                                        && !selected,
                                                    |el| el.text_color(ui::hsla(ui::ACCENT)),
                                                )
                                                .child(body),
                                        ),
                                )
                                .when(mentions > 0, |el| {
                                    el.child(badge("@".into(), false, selected))
                                })
                                .when(unread > 0 || chat.info.is_marked_as_unread, |el| {
                                    el.child(badge(
                                        if unread > 0 {
                                            unread.to_string()
                                        } else {
                                            String::new()
                                        },
                                        muted,
                                        selected,
                                    ))
                                })
                                .when(
                                    pinned && unread == 0 && !chat.info.is_marked_as_unread,
                                    |el| {
                                        el.child(
                                            Icon::new(IconName::Pin)
                                                .size(px(ui::ICON_SM))
                                                .text_color(ui::hsla(meta_color)),
                                        )
                                    },
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn chat_avatar(&self, chat: &Chat, size: f32) -> AnyElement {
        if self.is_self_chat(chat) {
            return icon_avatar(IconName::Bookmark, ui::BADGE, size);
        }
        let fallback = || letter_avatar(chat.info.id, &chat.info.title, size);
        let Some(photo) = &chat.info.photo else {
            return fallback();
        };
        let file = self.file(&photo.small);
        if !file.local.is_downloading_completed || file.local.path.is_empty() {
            return fallback();
        }
        let id = chat.info.id;
        let title = chat.info.title.clone();
        img(std::path::PathBuf::from(&file.local.path))
            .size(px(size))
            .flex_shrink_0()
            .rounded_full()
            .object_fit(ObjectFit::Cover)
            .with_fallback(move || letter_avatar(id, &title, size))
            .into_any_element()
    }

    pub(super) fn user_avatar(&self, user_id: i64, size: f32) -> AnyElement {
        let name = self.user_name(user_id);
        if let Some(user) = self.users.get(&user_id)
            && let Some(photo) = &user.profile_photo
        {
            let file = self.file(&photo.small);
            if file.local.is_downloading_completed && !file.local.path.is_empty() {
                return img(std::path::PathBuf::from(&file.local.path))
                    .size(px(size))
                    .flex_shrink_0()
                    .rounded_full()
                    .object_fit(ObjectFit::Cover)
                    .with_fallback(move || letter_avatar(user_id, &name, size))
                    .into_any_element();
            }
        }
        letter_avatar(user_id, &name, size)
    }
}

fn row_shell(id: ElementId, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .w_full()
        .overflow_hidden()
        .h(px(ROW_HEIGHT))
        .px(px(8.))
        .rounded(px(ui::RADIUS_MD))
        .flex()
        .items_center()
        .gap(px(12.))
        .cursor_pointer()
        .when(selected, |el| {
            el.bg(ui::hsla(ui::SELECTED))
                .text_color(ui::hsla(ui::SELECTED_TEXT))
        })
        .when(!selected, |el| {
            el.hover(|s| s.bg(ui::hsla(ui::HOVER)))
                .active(|s| s.bg(ui::hsla(ui::RAISED)))
        })
}

fn badge(text: String, muted: bool, selected: bool) -> Div {
    widgets::badge(
        text,
        match (selected, muted) {
            (true, _) => widgets::Tone::Inverted,
            (false, true) => widgets::Tone::Muted,
            (false, false) => widgets::Tone::Accent,
        },
    )
}

pub(super) fn letter_avatar(seed: i64, title: &str, size: f32) -> AnyElement {
    div()
        .size(px(size))
        .flex_shrink_0()
        .rounded_full()
        .bg(ui::hsla(ui::peer_avatar_color(seed)))
        .text_color(ui::hsla(ui::WHITE))
        .text_size(px(size * 0.38))
        .font_weight(FontWeight::SEMIBOLD)
        .flex()
        .items_center()
        .justify_center()
        .child(ui::initials(title))
        .into_any_element()
}

pub(super) fn icon_avatar(icon: IconName, color: ui::Color, size: f32) -> AnyElement {
    div()
        .size(px(size))
        .flex_shrink_0()
        .rounded_full()
        .bg(ui::hsla(color))
        .flex()
        .items_center()
        .justify_center()
        .child(
            Icon::new(icon)
                .size(px(size * 0.46))
                .text_color(ui::hsla(ui::WHITE)),
        )
        .into_any_element()
}
