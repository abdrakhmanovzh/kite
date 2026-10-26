use std::path::PathBuf;

use gpui_kit::{
    assets::IconName,
    component::{
        Disableable, Icon, Selectable, Sizable, WindowExt,
        button::{Button, ButtonVariants},
        checkbox::Checkbox,
        input::{Input, Textarea},
        message_scroller::MessageScroller,
        scroll::ScrollableElement,
    },
    prelude::*,
    *,
};

use super::{
    ComposeAction, Messenger, drag_strip, history::service_pill, menu::MenuEntry, sticker_file,
    widgets,
};
use crate::{
    model::{List, preview},
    telegram::Command,
    ui,
};

impl Messenger {
    pub(super) fn conversation(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(chat) = self.selected.and_then(|id| self.chats.get(&id)) else {
            return div()
                .relative()
                .flex_1()
                .h_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .child(drag_strip("empty-drag"))
                .child(
                    div()
                        .size(px(72.))
                        .rounded(px(22.))
                        .bg(ui::hsla(ui::RAISED))
                        .border_1()
                        .border_color(ui::hsla(ui::STROKE_STRONG))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::MessageCircle)
                                .size(px(30.))
                                .text_color(ui::hsla(ui::ACCENT)),
                        ),
                )
                .child(
                    div()
                        .mt_2()
                        .text_size(px(ui::TEXT_LG))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("No chat selected"),
                )
                .child(
                    div()
                        .text_size(px(ui::TEXT_SM))
                        .text_color(ui::hsla(ui::MUTED))
                        .child("Pick a conversation on the left, or jump to one"),
                )
                .child(div().mt_1().child(widgets::hint("⌘K", "Search chats")))
                .into_any_element();
        };
        let id = chat.info.id;
        let forum = matches!(&chat.info.r#type, tdlib_rs::enums::ChatType::Supergroup(g) if self.forum_groups.contains(&g.supergroup_id));
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(self.header(window, cx))
            .when(self.message_search_open, |el| el.child(self.search_bar(cx)))
            .when(!self.message_search_open, |el| {
                el.children(self.pinned_bar(id, cx))
            })
            .when(forum && !self.message_search_open, |el| {
                el.child(self.topic_strip(id, cx))
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .bg(ui::hsla(ui::HISTORY))
                    .when(
                        self.message_search_open
                            && self
                                .message_results
                                .as_ref()
                                .is_some_and(|(_, _, found)| found.is_empty()),
                        |el| {
                            el.child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(service_pill("No messages found".into())),
                            )
                        },
                    )
                    .when(
                        !self.message_search_open && self.rows.is_empty() && !chat.history_loading,
                        |el| {
                            el.child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(service_pill("No messages here yet".into())),
                            )
                        },
                    )
                    .child(
                        MessageScroller::new(
                            ("history", id as u64),
                            self.scroller.clone(),
                            cx.processor(|this, ix: usize, _, cx| this.render_row(ix, cx)),
                        )
                        .with_row_style(StyleRefinement::default().px_0().pb_0())
                        // Ours below: the built-in one only scrolls the loaded
                        // window, which may end before the latest message.
                        .jump_button(false)
                        .size_full(),
                    )
                    .when(
                        self.scroller.read(cx).is_scrolled_up()
                            || self.chats.get(&id).is_some_and(|chat| chat.has_newer),
                        |el| {
                            el.child(
                                // A floating control in the bottom-right corner,
                                // clear of the message column.
                                Button::new("jump-to-latest")
                                    .secondary()
                                    .icon(IconName::ChevronDown)
                                    .rounded_full()
                                    .absolute()
                                    .right(px(ui::GUTTER))
                                    .bottom(px(ui::GUTTER))
                                    .bg(ui::hsla(ui::OVERLAY))
                                    .border_1()
                                    .border_color(ui::hsla(ui::STROKE_STRONG))
                                    .text_color(ui::hsla(ui::SECONDARY))
                                    .shadow_md()
                                    .tooltip("Jump to latest")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.jump_to_latest(id, cx)
                                    })),
                            )
                        },
                    ),
            )
            .child(self.composer_area(id, cx))
            .into_any_element()
    }

    fn header(&self, _: &mut Window, cx: &mut Context<Self>) -> Div {
        let Some(chat) = self.selected.and_then(|id| self.chats.get(&id)) else {
            return div();
        };
        let id = chat.info.id;
        let (status, active) = self.chat_status(chat);
        let muted = chat.info.notification_settings.mute_for > 0;
        let mut settings = chat.info.notification_settings.clone();
        settings.use_default_mute_for = false;
        settings.mute_for = if muted { 0 } else { i32::MAX };
        let list = self.list;
        let pinned = chat
            .info
            .positions
            .iter()
            .any(|p| p.list == list.td() && p.is_pinned);
        let archived = chat.order(List::Archive) > 0;
        let last = chat.info.last_message.as_ref().map(|m| m.id);
        let leave_label = match &chat.info.r#type {
            tdlib_rs::enums::ChatType::Private(_) | tdlib_rs::enums::ChatType::Secret(_) => None,
            _ if self.is_channel(chat) => Some("Leave Channel"),
            _ => Some("Leave Group"),
        };
        div()
            .relative()
            .h(px(ui::HEADER))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .pl(px(ui::GUTTER))
            .pr(px(ui::GUTTER - 8.))
            .border_b_1()
            .border_color(ui::hsla(ui::STROKE))
            .child(drag_strip("header-drag").h_full())
            .child(
                div()
                    .id("chat-title")
                    .relative()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_details(cx)))
                    .child(self.chat_avatar(chat, ui::AVATAR_MD))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(ui::TEXT_LG))
                                    .child(self.chat_title(chat)),
                            )
                            .when(!status.is_empty(), |el| {
                                el.child(
                                    div()
                                        .truncate()
                                        .text_size(px(ui::TEXT_SM))
                                        .text_color(ui::hsla(if active {
                                            ui::ACCENT
                                        } else {
                                            ui::MUTED
                                        }))
                                        .child(status),
                                )
                            }),
                    ),
            )
            .child(div().flex_1())
            .child(
                widgets::icon_button("search-messages", IconName::Search)
                    .tooltip("Search")
                    .selected(self.message_search_open)
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_search(window, cx))),
            )
            .child(
                widgets::icon_button("details", IconName::PanelRight)
                    .tooltip("Info")
                    .selected(self.details)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_details(cx))),
            )
            .child(
                widgets::icon_button("chat-more", IconName::EllipsisVertical)
                    .tooltip("More")
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        let mute = settings.clone();
                        let mut entries = vec![
                            MenuEntry::item(
                                if muted {
                                    IconName::Bell
                                } else {
                                    IconName::BellOff
                                },
                                if muted { "Unmute" } else { "Mute" },
                                move |this, _, _| {
                                    this.send(Command::Mute {
                                        chat_id: id,
                                        settings: mute.clone(),
                                    });
                                },
                            ),
                            MenuEntry::item(
                                if pinned {
                                    IconName::PinOff
                                } else {
                                    IconName::Pin
                                },
                                if pinned { "Unpin" } else { "Pin" },
                                move |this, _, _| {
                                    this.send(Command::Pin {
                                        chat_id: id,
                                        list,
                                        pinned: !pinned,
                                    });
                                },
                            ),
                            MenuEntry::item(
                                if archived {
                                    IconName::ArchiveRestore
                                } else {
                                    IconName::Archive
                                },
                                if archived { "Unarchive" } else { "Archive" },
                                move |this, _, _| {
                                    this.send(Command::Archive {
                                        chat_id: id,
                                        archive: !archived,
                                    });
                                },
                            ),
                        ];
                        if let Some(message_id) = last {
                            entries.push(MenuEntry::item(
                                IconName::CheckCheck,
                                "Mark as Read",
                                move |this, _, _| {
                                    this.send(Command::Read {
                                        chat_id: id,
                                        message_id,
                                    });
                                },
                            ));
                        }
                        entries.push(MenuEntry::Separator);
                        entries.push(MenuEntry::item(
                            IconName::Search,
                            "Search Messages",
                            |this, window, cx| {
                                if !this.message_search_open {
                                    this.toggle_search(window, cx);
                                }
                            },
                        ));
                        entries.push(MenuEntry::item(IconName::X, "Close Chat", |this, _, cx| {
                            this.close_chat(cx)
                        }));
                        if let Some(label) = leave_label {
                            entries.push(MenuEntry::Separator);
                            entries.push(
                                MenuEntry::item(
                                    IconName::LogOut,
                                    label,
                                    move |this, window, cx| this.confirm_leave(id, window, cx),
                                )
                                .danger(),
                            );
                        }
                        let at = event.position() + point(px(4.), px(14.));
                        this.open_menu(at, Anchor::TopRight, entries, cx);
                    })),
            )
    }

    /// The pinned message bar: click jumps to the shown pin, then moves on to
    /// the next older one, like Telegram.
    fn pinned_bar(&self, chat_id: i64, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let list = self.pinned.get(&chat_id).filter(|list| !list.is_empty())?;
        let count = list.len();
        let index = self
            .pinned_index
            .get(&chat_id)
            .copied()
            .unwrap_or(0)
            .min(count - 1);
        let message = &list[index];
        let target = message.id;
        let title = if count > 1 {
            format!("Pinned Message #{}", count - index)
        } else {
            "Pinned Message".to_owned()
        };
        // Position marker: a thumb sliding along a short track.
        let track = 32.;
        let thumb = if count > 1 {
            (track / count as f32).max(6.)
        } else {
            track
        };
        let top = if count > 1 {
            (track - thumb) * (count - 1 - index) as f32 / (count - 1) as f32
        } else {
            0.
        };
        Some(
            div()
                .id("pinned-bar")
                .flex_shrink_0()
                .h(px(48.))
                .px(px(ui::GUTTER))
                .flex()
                .items_center()
                .gap(px(12.))
                .border_b_1()
                .border_color(ui::hsla(ui::STROKE))
                .cursor_pointer()
                .hover(|s| s.bg(ui::hsla_alpha(ui::RAISED, 0x80)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.jump_to(chat_id, target, cx);
                    let index = this.pinned_index.entry(chat_id).or_default();
                    *index = (*index + 1) % count;
                }))
                .child(
                    div()
                        .relative()
                        .w(px(3.))
                        .h(px(track))
                        .rounded_full()
                        .bg(ui::hsla_alpha(ui::ACCENT, 0x40))
                        .child(
                            div()
                                .absolute()
                                .top(px(top))
                                .w_full()
                                .h(px(thumb))
                                .rounded_full()
                                .bg(ui::hsla(ui::ACCENT)),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(ui::TEXT_SM))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ui::hsla(ui::ACCENT))
                                .child(title),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(ui::TEXT_SM))
                                .text_color(ui::hsla(ui::SECONDARY))
                                .child(preview(&message.content).0),
                        ),
                )
                .child(
                    Icon::new(IconName::Pin)
                        .size(px(ui::ICON_MD))
                        .text_color(ui::hsla(ui::MUTED)),
                ),
        )
    }

    pub(super) fn toggle_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.message_search_open = !self.message_search_open;
        self.message_results = None;
        self.message_search.update(cx, |input, cx| {
            input.set_value("", window, cx);
            if self.message_search_open {
                input.focus(window, cx);
            }
        });
        cx.notify();
    }

    fn search_bar(&self, cx: &mut Context<Self>) -> Div {
        let found = self
            .message_results
            .as_ref()
            .map(|(_, _, found)| found.len());
        div()
            .h(px(48.))
            .flex_shrink_0()
            .pl(px(ui::GUTTER))
            .pr(px(ui::GUTTER - 8.))
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(ui::hsla(ui::STROKE))
            .child(
                widgets::search_field().flex_1().child(
                    div().flex_1().min_w_0().child(
                        Input::new(&self.message_search)
                            .appearance(false)
                            .cleanable(true),
                    ),
                ),
            )
            .when_some(found, |el, found| {
                el.child(
                    div()
                        .text_size(px(ui::TEXT_SM))
                        .text_color(ui::hsla(ui::MUTED))
                        .child(match found {
                            0 => "No results".to_owned(),
                            1 => "1 result".to_owned(),
                            n => format!("{n} results"),
                        }),
                )
            })
            .child(
                Button::new("close-search")
                    .ghost()
                    .small()
                    .label("Cancel")
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_search(window, cx))),
            )
    }

    fn topic_strip(&self, id: i64, cx: &mut Context<Self>) -> Stateful<Div> {
        let selected = self.topic_by_chat.get(&id).copied();
        let chip = |key: ElementId, label: String, unread: i32, active: bool| {
            widgets::chip(key, label, active).when(unread > 0, |el| {
                el.child(widgets::badge(unread.to_string(), widgets::Tone::Accent).h(px(16.)))
            })
        };
        div()
            .id("topics")
            .flex_shrink_0()
            .h(px(44.))
            .px(px(ui::GUTTER))
            .flex()
            .items_center()
            .gap(px(6.))
            .overflow_x_scroll()
            .border_b_1()
            .border_color(ui::hsla(ui::STROKE))
            .child(
                chip("topic-all".into(), "All".into(), 0, selected.is_none()).on_click(
                    cx.listener(move |this, _, window, cx| this.select_topic(id, None, window, cx)),
                ),
            )
            .children(self.topics.get(&id).into_iter().flatten().map(|topic| {
                let topic_id = topic.info.forum_topic_id;
                chip(
                    ("topic", topic_id as u64).into(),
                    topic.info.name.clone(),
                    topic.unread_count,
                    selected == Some(topic_id),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select_topic(id, Some(topic_id), window, cx)
                }))
            }))
            .when(
                self.topic_offsets
                    .get(&id)
                    .is_some_and(|offset| *offset != (0, 0, 0)),
                |el| {
                    el.child(
                        chip("more-topics".into(), "More…".into(), 0, false).on_click(cx.listener(
                            move |this, _, _, cx| {
                                if let Some(&offset) = this.topic_offsets.get(&id) {
                                    this.send(Command::Topics {
                                        chat_id: id,
                                        offset,
                                    });
                                }
                                cx.notify();
                            },
                        )),
                    )
                },
            )
    }

    fn composer_area(&self, id: i64, cx: &mut Context<Self>) -> Div {
        let Some(chat) = self.chats.get(&id) else {
            return div();
        };
        // Channels where we can't post get Telegram's mute/unmute bar.
        if self.group(chat).is_some_and(|g| !g.can_post) {
            let muted = chat.info.notification_settings.mute_for > 0;
            let mut settings = chat.info.notification_settings.clone();
            settings.use_default_mute_for = false;
            settings.mute_for = if muted { 0 } else { i32::MAX };
            return div()
                .flex_shrink_0()
                .h(px(52.))
                // GPUI doesn't clip children to the panel's rounded corners.
                .rounded_b(px(ui::RADIUS_PANEL - 1.))
                .border_t_1()
                .border_color(ui::hsla(ui::BORDER))
                .child(
                    div()
                        .id("channel-mute")
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .text_color(ui::hsla(ui::ACCENT))
                        .font_weight(FontWeight::SEMIBOLD)
                        .hover(|s| s.bg(ui::hsla(ui::HOVER)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.send(Command::Mute {
                                chat_id: id,
                                settings: settings.clone(),
                            });
                            cx.notify();
                        }))
                        .child(if muted { "Unmute" } else { "Mute" }),
                );
        }
        let Some(input) = self.composers.get(&id) else {
            return div();
        };
        let busy = self.sending.contains(&id);
        let action = self.compose_actions.get(&id);
        let forwarding = matches!(action, Some(ComposeAction::Forward { .. }));
        let editing = matches!(action, Some(ComposeAction::Edit { .. }));
        let ready = forwarding
            || self.attachments.contains_key(&id)
            || !input.read(cx).value().trim().is_empty();

        let context = match action {
            Some(ComposeAction::Reply(message_id)) => self.find_message(id, *message_id).map(|m| {
                (
                    IconName::Reply,
                    format!("Reply to {}", self.sender_name(m)),
                    preview(&m.content).0,
                )
            }),
            Some(ComposeAction::Edit { id: message_id, .. }) => Some((
                IconName::Pencil,
                "Edit message".into(),
                self.find_message(id, *message_id)
                    .map_or_else(String::new, |m| preview(&m.content).0),
            )),
            Some(ComposeAction::Forward {
                from,
                id: message_id,
            }) => Some((
                IconName::Forward,
                "Forward message".into(),
                self.find_message(*from, *message_id)
                    .map_or_else(String::new, |m| preview(&m.content).0),
            )),
            None => None,
        }
        .or_else(|| {
            self.attachments.get(&id).map(|path| {
                (
                    IconName::Paperclip,
                    path.file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                    "Will be sent with your message".into(),
                )
            })
        });

        div()
            .flex_shrink_0()
            .px(px(ui::GUTTER))
            .pb(px(ui::GUTTER))
            .pt(px(6.))
            .bg(ui::hsla(ui::HISTORY))
            // GPUI doesn't clip children to the panel's rounded corners.
            .rounded_b(px(ui::RADIUS_PANEL - 1.))
            .child(
                div()
                    // A capsule at one line; keeps that radius as it grows.
                    .rounded(px(ui::CONTROL_LG / 2.))
                    .bg(ui::hsla(ui::COMPOSER))
                    .border_1()
                    .border_color(ui::hsla(ui::STROKE_STRONG))
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    // Same column grid as the input row below: the icon over the
                    // attach button, the quote on the text column (the field
                    // insets its text 8pt), cancel over the trailing button.
                    .when_some(context, |el, (icon, title, detail)| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(2.))
                                .px(px(6.))
                                .pt(px(6.))
                                .child(
                                    div()
                                        .size(px(ui::CONTROL_SM))
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            Icon::new(icon)
                                                .size(px(ui::ICON_MD))
                                                .text_color(ui::hsla(ui::ACCENT)),
                                        ),
                                )
                                .child(div().flex_1().min_w_0().pl(px(8.)).child(widgets::quote(
                                    ui::ACCENT,
                                    title,
                                    detail,
                                    ui::SECONDARY,
                                )))
                                .child(
                                    // In the composer's corner: concentric, like its buttons.
                                    widgets::icon_button("cancel-compose-action", IconName::X)
                                        .rounded_full()
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.cancel_compose_action(id, window, cx)
                                        })),
                                ),
                        )
                    })
                    .child(
                        div()
                            // One line of text is exactly as tall as the
                            // 32px buttons, so text and icons share a center;
                            // longer messages grow upward over pinned buttons.
                            .flex()
                            .items_end()
                            .gap(px(2.))
                            .p(px(6.))
                            .child(
                                composer_button("attach", IconName::Paperclip, ui::MUTED)
                                    .disabled(busy || editing || forwarding)
                                    .tooltip("Attach")
                                    .on_click(cx.listener(
                                        move |this, event: &ClickEvent, _, cx| {
                                            let entries = vec![
                                                MenuEntry::item(
                                                    IconName::Image,
                                                    "Photo or File",
                                                    move |this, window, cx| {
                                                        this.choose_attachment(id, window, cx)
                                                    },
                                                ),
                                                MenuEntry::item(
                                                    IconName::ListChecks,
                                                    "Poll",
                                                    move |this, window, cx| {
                                                        this.open_poll_dialog(id, window, cx)
                                                    },
                                                ),
                                            ];
                                            let at = event.position() + point(px(-4.), px(-14.));
                                            this.open_menu(at, Anchor::BottomLeft, entries, cx);
                                        },
                                    )),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .py(px(4.))
                                    // Capture runs before the field's own
                                    // right-click menu, which we replace.
                                    .capture_any_mouse_down(cx.listener(
                                        |this, event: &MouseDownEvent, _, cx| {
                                            if event.button == MouseButton::Right {
                                                this.composer_menu(event.position, cx);
                                                cx.stop_propagation();
                                            }
                                        },
                                    ))
                                    .child(
                                        Textarea::new(input)
                                            .appearance(false)
                                            .small()
                                            .disabled(busy || forwarding),
                                    ),
                            )
                            .child(
                                // Open state as a filled background, like the header's toggles.
                                composer_button(
                                    "stickers",
                                    IconName::FaceSlightlySmiling,
                                    ui::MUTED,
                                )
                                .selected(self.sticker_open)
                                .tooltip("Stickers")
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_stickers(cx))),
                            )
                            .when(ready || busy, |el| {
                                el.child(
                                    Button::new("send-message")
                                        .primary()
                                        .rounded_full()
                                        .loading(busy)
                                        .icon(
                                            Icon::new(if editing {
                                                IconName::Check
                                            } else {
                                                IconName::SendHorizontal
                                            })
                                            .text_color(ui::hsla(ui::WHITE)),
                                        )
                                        .tooltip(if editing { "Save" } else { "Send" })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.send_message(id, cx)
                                        })),
                                )
                            }),
                    ),
            )
    }

    /// Stickers as a side panel next to the conversation: recents, or search
    /// results while the search field has text.
    pub(super) fn sticker_panel(&self, cx: &mut Context<Self>) -> Div {
        let Some(chat_id) = self.selected else {
            return div();
        };
        let searching = !self.sticker_search.read(cx).value().trim().is_empty();
        let stickers = self.picker_stickers(cx);
        let empty = if searching {
            if self.sticker_results.is_some() {
                "No stickers found"
            } else {
                "Searching…"
            }
        } else {
            "Stickers you send show up here"
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(widgets::panel_header(
                "Stickers",
                widgets::icon_button("close-stickers", IconName::X)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_stickers(cx))),
            ))
            .child(
                div().px(px(ui::GUTTER)).pt(px(12.)).child(
                    widgets::search_field().child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.sticker_search)
                                .appearance(false)
                                .cleanable(true),
                        ),
                    ),
                ),
            )
            .child(
                widgets::section_label(if searching { "Results" } else { "Recent" })
                    .px(px(ui::GUTTER))
                    .pt(px(16.)),
            )
            .child(
                div()
                    .id("sticker-grid")
                    .flex_1()
                    .min_h_0()
                    // Cells pad their image by 6px, so images sit on the gutter.
                    .px(px(ui::GUTTER - 6.))
                    .pb(px(12.))
                    .overflow_y_scrollbar()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(4.))
                            .when(stickers.is_empty(), |el| {
                                el.child(
                                    div()
                                        .w_full()
                                        .pt_10()
                                        .flex()
                                        .justify_center()
                                        .text_size(px(ui::TEXT_SM))
                                        .text_color(ui::hsla(ui::MUTED))
                                        .child(empty),
                                )
                            })
                            .children(stickers.into_iter().map(|sticker| {
                                let file = sticker_file(sticker).map(|f| self.file(f));
                                let ready = file.filter(|f| f.local.is_downloading_completed);
                                let sticker = sticker.clone();
                                div()
                                    .id(("pick-sticker", sticker.id as u64))
                                    // Four per row in the 340px panel
                                    // (minus its border and padding).
                                    .size(px(74.))
                                    .p(px(6.))
                                    .rounded(px(ui::RADIUS_MD))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(ui::hsla(ui::HOVER)))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(match (self.animation(&sticker, false), ready) {
                                        (Some(frame), _) => img(frame)
                                            .size(px(62.))
                                            .object_fit(ObjectFit::Contain)
                                            .into_any_element(),
                                        (None, Some(file)) => {
                                            let emoji = sticker.emoji.clone();
                                            img(PathBuf::from(&file.local.path))
                                                .size(px(62.))
                                                // Concentric with the cell (RADIUS_MD) around its 6pt padding.
                                                .rounded(px(ui::RADIUS_MD - 6.))
                                                .object_fit(ObjectFit::Contain)
                                                .with_fallback(move || {
                                                    div()
                                                        .text_size(px(30.))
                                                        .child(emoji.clone())
                                                        .into_any_element()
                                                })
                                                .into_any_element()
                                        }
                                        (None, None) => div()
                                            .text_size(px(30.))
                                            .child(sticker.emoji.clone())
                                            .into_any_element(),
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.send_sticker(chat_id, sticker.clone(), cx)
                                    }))
                            })),
                    ),
            )
    }

    fn open_poll_dialog(&mut self, chat_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let entity = cx.entity();
        let question = self.poll_question.clone();
        let options = self.poll_options.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let state = entity.read(cx);
            let (anonymous, multiple) = (state.poll_anonymous, state.poll_multiple);
            let toggle = |weak: WeakEntity<Messenger>, anonymous: bool| {
                move |checked: &bool, window: &mut Window, cx: &mut App| {
                    let checked = *checked;
                    let _ = weak.update(cx, |this, cx| {
                        if anonymous {
                            this.poll_anonymous = checked;
                        } else {
                            this.poll_multiple = checked;
                        }
                        cx.notify();
                    });
                    window.refresh();
                }
            };
            let submit = weak.clone();
            dialog
                .title("New poll")
                .w(px(420.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(Input::new(&question))
                        .child(Textarea::new(&options))
                        .child(
                            Checkbox::new("poll-anonymous")
                                .label("Anonymous voting")
                                .checked(anonymous)
                                .on_click(toggle(weak.clone(), true)),
                        )
                        .child(
                            Checkbox::new("poll-multiple")
                                .label("Multiple answers")
                                .checked(multiple)
                                .on_click(toggle(weak.clone(), false)),
                        ),
                )
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("poll-cancel")
                                .ghost()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("poll-create")
                                .primary()
                                .label("Create")
                                .on_click(move |_, window, cx| {
                                    let sent = submit
                                        .update(cx, |this, cx| this.submit_poll(chat_id, cx))
                                        .unwrap_or(false);
                                    if sent {
                                        window.close_dialog(cx);
                                    }
                                }),
                        ),
                )
        });
    }

    fn submit_poll(&mut self, chat_id: i64, cx: &mut Context<Self>) -> bool {
        let question = self.poll_question.read(cx).value().trim().to_owned();
        let options: Vec<String> = self
            .poll_options
            .read(cx)
            .value()
            .lines()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .collect();
        if question.is_empty()
            || question.chars().count() > 255
            || !(2..=10).contains(&options.len())
            || options.iter().any(|v| v.chars().count() > 100)
        {
            self.error = Some(
                "Use a question of 1–255 characters and 2–10 options of up to 100 characters each."
                    .into(),
            );
            cx.notify();
            return false;
        }
        let sent = self.send(Command::SendPoll {
            chat_id,
            topic: self.topic_by_chat.get(&chat_id).copied(),
            question,
            options,
            anonymous: self.poll_anonymous,
            multiple: self.poll_multiple,
        });
        cx.notify();
        sent
    }
}

/// Composer toolbar button: larger target and icon than toolbar buttons.
/// Round: the composer is a capsule of radius `CONTROL_LG / 2` and its
/// 32pt buttons sit 6pt in, so concentric corners make them circles.
fn composer_button(id: &'static str, icon: IconName, color: ui::Color) -> Button {
    widgets::icon_button(id, icon)
        .rounded_full()
        .icon(Icon::new(icon).text_color(ui::hsla(color)))
}

/// Inline formatting the composer can apply; sent as Telegram markdown.
#[derive(Clone, Copy)]
pub(super) enum Format {
    Bold,
    Italic,
    Mono,
    Strike,
    Spoiler,
    Code,
}

impl Format {
    const ALL: [Format; 6] = [
        Format::Bold,
        Format::Italic,
        Format::Mono,
        Format::Strike,
        Format::Spoiler,
        Format::Code,
    ];

    fn markers(self) -> (&'static str, &'static str) {
        match self {
            Format::Bold => ("**", "**"),
            Format::Italic => ("__", "__"),
            Format::Mono => ("`", "`"),
            Format::Strike => ("~~", "~~"),
            Format::Spoiler => ("||", "||"),
            Format::Code => ("```\n", "\n```"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Format::Bold => "Bold",
            Format::Italic => "Italic",
            Format::Mono => "Monospace",
            Format::Strike => "Strikethrough",
            Format::Spoiler => "Spoiler",
            Format::Code => "Code Block",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Format::Bold => IconName::Bold,
            Format::Italic => IconName::Italic,
            Format::Mono => IconName::Code,
            Format::Strike => IconName::Strikethrough,
            Format::Spoiler => IconName::EyeOff,
            Format::Code => IconName::SquareCode,
        }
    }

    fn shortcut(self) -> &'static str {
        match self {
            Format::Bold => "⌘B",
            Format::Italic => "⌘I",
            Format::Mono => "⇧⌘M",
            Format::Strike => "⇧⌘X",
            Format::Spoiler => "⇧⌘P",
            Format::Code => "⇧⌘C",
        }
    }

    pub(super) fn from_keystroke(keystroke: &Keystroke) -> Option<Self> {
        let m = &keystroke.modifiers;
        if !m.platform || m.alt || m.control {
            return None;
        }
        Some(match (m.shift, keystroke.key.as_str()) {
            (false, "b") => Format::Bold,
            (false, "i") => Format::Italic,
            (true, "m") => Format::Mono,
            (true, "x") => Format::Strike,
            (true, "p") => Format::Spoiler,
            (true, "c") => Format::Code,
            _ => return None,
        })
    }
}

impl Messenger {
    /// Whether the open chat's composer has keyboard focus.
    pub(super) fn composer_focused(&self, window: &Window, cx: &App) -> bool {
        self.selected
            .and_then(|id| self.composers.get(&id))
            .is_some_and(|input| input.focus_handle(cx).is_focused(window))
    }

    /// Wraps the composer selection in the format's markers, or inserts an
    /// empty pair with the cursor between them.
    pub(super) fn apply_format(
        &mut self,
        format: Format,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(input) = self
            .selected
            .and_then(|id| self.composers.get(&id))
            .cloned()
        else {
            return;
        };
        let (open, close) = format.markers();
        input.update(cx, |state, cx| {
            let range = state.selected_range();
            let selected = state.selected_text().to_string();
            state.replace(format!("{open}{selected}{close}"), window, cx);
            if range.is_empty() {
                let cursor = range.start + open.len();
                state.set_selected_range(cursor..cursor, cx);
            }
            state.focus(window, cx);
        });
        cx.notify();
    }

    /// Right-click menu of the message field: editing, then formatting.
    fn composer_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(input) = self
            .selected
            .and_then(|id| self.composers.get(&id))
            .cloned()
        else {
            return;
        };
        let selection = input.read(cx).selected_text().to_string();
        let mut entries = Vec::new();
        if !selection.is_empty() {
            let cut = input.clone();
            let text = selection.clone();
            entries.push(
                MenuEntry::item(IconName::Scissors, "Cut", move |_, window, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    cut.update(cx, |state, cx| state.replace("", window, cx));
                })
                .shortcut("⌘X"),
            );
            let text = selection.clone();
            entries.push(
                MenuEntry::item(IconName::Copy, "Copy", move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                })
                .shortcut("⌘C"),
            );
        }
        let paste = input.clone();
        entries.push(
            MenuEntry::item(IconName::ClipboardPaste, "Paste", move |_, window, cx| {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    paste.update(cx, |state, cx| state.replace(text, window, cx));
                }
            })
            .shortcut("⌘V"),
        );
        let all = input.clone();
        entries.push(
            MenuEntry::item(IconName::TextCursor, "Select All", move |_, window, cx| {
                all.update(cx, |state, cx| state.select_all(window, cx));
            })
            .shortcut("⌘A"),
        );
        entries.push(MenuEntry::Separator);
        entries.extend(Format::ALL.into_iter().map(|format| {
            MenuEntry::item(format.icon(), format.label(), move |this, window, cx| {
                this.apply_format(format, window, cx)
            })
            .shortcut(format.shortcut())
        }));
        self.open_menu(position, Anchor::BottomLeft, entries, cx);
    }
}
