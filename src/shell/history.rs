use std::path::PathBuf;

use chrono::Datelike;
use gpui_kit::{
    assets::IconName,
    component::{Icon, Sizable, spinner::Spinner},
    prelude::*,
    *,
};
use tdlib_rs::{
    enums::{MessageContent as C, MessageSendingState},
    types::{FormattedText, Message},
};

use super::{
    Messenger, Row, RowsKey, RowsSource, menu::MenuEntry, sticker_file, video_thumbnail, widgets,
};
use crate::{
    model::{Chat, is_service, message_text, preview, service_action},
    telegram::{Command, Reaction},
    ui,
};

const MAX_BUBBLE: f32 = 460.;
/// Longest side of a sticker.
const STICKER: f32 = 168.;
const GROUP_AVATAR: f32 = ui::AVATAR_SM;
/// How many rows from the top of the loaded history the next page starts loading.
const PREFETCH_ROWS: usize = 20;
/// Consecutive messages from one sender within this window share a group.
const GROUP_SECONDS: i32 = 300;

/// How a bubble sits within a run of messages from one sender.
#[derive(Clone, Copy)]
struct Place {
    first: bool,
    last: bool,
}

impl Messenger {
    fn rows_for(&self, chat: &Chat) -> Vec<Row> {
        let messages = self.visible_messages(chat);
        let loading = match self.topic_by_chat.get(&chat.info.id) {
            Some(topic) => self.topic_loading.contains(&(chat.info.id, *topic)),
            None => chat.history_loading,
        };
        let mut rows = Vec::with_capacity(messages.len() * 11 / 10 + 2);
        if loading && !self.message_search_open {
            rows.push(Row::Loading);
        }
        let anchor = (!self.message_search_open)
            .then(|| self.unread_anchor.get(&chat.info.id).copied())
            .flatten();
        let mut unread_placed = false;
        let mut day = None;
        for message in messages {
            let today = ui::local(message.date).map(|t| t.date_naive().num_days_from_ce());
            if today != day {
                day = today;
                if let Some(today) = today {
                    rows.push(Row::Day(today));
                }
            }
            if !unread_placed
                && anchor.is_some_and(|anchor| message.id > anchor && !message.is_outgoing)
            {
                unread_placed = true;
                rows.push(Row::Unread);
            }
            rows.push(Row::Message(message.id));
        }
        rows
    }

    /// After rows change: finish a pending jump or scroll to the unread divider.
    fn settle_scroll(&mut self, cx: &mut Context<Self>) {
        if let Some((chat, target)) = self.jump_target
            && self.selected == Some(chat)
            && let Some(ix) = self.rows.iter().position(|r| *r == Row::Message(target))
        {
            self.jump_target = None;
            self.scroll_to_unread = false;
            // Leave a little context above the target.
            self.scroller.update(cx, |state, cx| {
                state.scroll_to_item(ix.saturating_sub(2), cx)
            });
            self.flash(target, cx);
            return;
        }
        if self.scroll_to_unread
            && let Some(ix) = self.rows.iter().position(|r| *r == Row::Unread)
        {
            self.scroll_to_unread = false;
            self.scroller
                .update(cx, |state, cx| state.scroll_to_item(ix, cx));
        }
    }

    /// Diffs the history rows against the scroller so prepending history or
    /// receiving messages keeps the scroll position.
    pub(super) fn sync_rows(&mut self, cx: &mut Context<Self>) {
        let Some(chat) = self.selected.and_then(|id| self.chats.get(&id)) else {
            self.rows_key = None;
            self.rows_source = None;
            self.rows.clear();
            return;
        };
        let key = RowsKey {
            chat: chat.info.id,
            topic: self.topic_by_chat.get(&chat.info.id).copied(),
            search: self.message_search_open && self.message_results.is_some(),
        };
        let source = RowsSource {
            key,
            messages: chat.messages.len(),
            // Ids change in place (a sent message gets its server id), so
            // fingerprint them all; a pass over keys is cheap next to a rebuild.
            ids: chat.messages.keys().fold(0u64, |hash, &id| {
                hash.wrapping_mul(0x100_0000_01b3).wrapping_add(id as u64)
            }),
            loading: match key.topic {
                Some(topic) => self.topic_loading.contains(&(chat.info.id, topic)),
                None => chat.history_loading,
            },
            anchor: self.unread_anchor.get(&chat.info.id).copied(),
            searching: self.message_search_open,
            results: self
                .message_results
                .as_ref()
                .map_or((0, 0, 0), |(_, _, found)| {
                    (
                        found.len(),
                        found.first().map_or(0, |m| m.id),
                        found.last().map_or(0, |m| m.id),
                    )
                }),
        };
        if self.rows_source == Some(source) && self.rows_key == Some(key) {
            return;
        }
        self.rows_source = Some(source);
        let rows = self.rows_for(chat);
        if self.rows_key != Some(key) {
            self.rows_key = Some(key);
            let count = rows.len();
            self.rows = rows;
            self.scroller.update(cx, |state, cx| state.reset(count, cx));
            self.settle_scroll(cx);
            return;
        }
        if rows == self.rows {
            return;
        }
        let old = &self.rows;
        let prefix = old.iter().zip(&rows).take_while(|(a, b)| a == b).count();
        let max_suffix = old.len().min(rows.len()) - prefix;
        let suffix = old
            .iter()
            .rev()
            .zip(rows.iter().rev())
            .take(max_suffix)
            .take_while(|(a, b)| a == b)
            .count();
        let removed = prefix..old.len() - suffix;
        let inserted = rows.len() - prefix - suffix;
        self.rows = rows;
        self.scroller
            .update(cx, |state, cx| state.splice(removed, inserted, cx));
        self.settle_scroll(cx);
    }

    pub(super) fn render_row(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        // Load older history a screenful before the top comes into view,
        // so scrolling up rarely reaches the spinner (once per page: rows
        // re-render on every frame while visible).
        if ix < PREFETCH_ROWS
            && !self.message_search_open
            && let Some(id) = self.selected
            && self.chats.get(&id).is_some_and(|chat| {
                self.topic_by_chat.contains_key(&id) || !(chat.history_loading || chat.history_end)
            })
        {
            cx.spawn(async move |this, cx| {
                let _ = this.update(cx, |this, cx| this.load_history(id, cx));
            })
            .detach();
        }
        // Load newer history as the bottom comes into view after a jump.
        if ix + 5 >= self.rows.len()
            && !self.message_search_open
            && let Some(id) = self.selected
            && self.chats.get(&id).is_some_and(|chat| chat.has_newer)
            && !self.newer_loading.contains(&id)
        {
            cx.spawn(async move |this, cx| {
                let _ = this.update(cx, |this, cx| this.load_newer(id, cx));
            })
            .detach();
        }
        // Decode animated stickers as they scroll into view.
        if let (Some(Row::Message(id)), Some(chat_id)) = (self.rows.get(ix).copied(), self.selected)
            && let Some(C::MessageSticker(m)) = self.find_message(chat_id, id).map(|m| &m.content)
        {
            let sticker = m.sticker.clone();
            self.ensure_animation(&sticker, true, cx);
            self.schedule_sticker_frame(&sticker, cx);
        }
        let Some(chat) = self.selected.and_then(|id| self.chats.get(&id)) else {
            return div().into_any_element();
        };
        let bottom_pad = if ix + 1 == self.rows.len() {
            px(10.)
        } else {
            px(0.)
        };
        let element = match self.rows.get(ix).copied() {
            Some(Row::Loading) => div()
                .py_3()
                .flex()
                .justify_center()
                .child(Spinner::new().small().color(ui::hsla(ui::MUTED)))
                .into_any_element(),
            Some(Row::Unread) => div()
                .my(px(8.))
                .py(px(5.))
                .flex()
                .justify_center()
                .bg(ui::hsla_alpha(ui::ACCENT, 0x14))
                .text_size(px(ui::TEXT_SM))
                .font_weight(FontWeight::MEDIUM)
                .text_color(ui::hsla(ui::ACCENT))
                .child("Unread messages")
                .into_any_element(),
            Some(Row::Day(day)) => {
                let label = chrono::NaiveDate::from_num_days_from_ce_opt(day)
                    .and_then(|d| d.and_hms_opt(12, 0, 0))
                    .and_then(|d| d.and_local_timezone(chrono::Local).single())
                    .map_or_else(String::new, |d| ui::day_label(d.timestamp() as i32));
                div()
                    .pt(px(10.))
                    .pb(px(6.))
                    .flex()
                    .justify_center()
                    .child(service_pill(label))
                    .into_any_element()
            }
            Some(Row::Message(id)) => {
                let Some(message) = self.find_message(chat.info.id, id) else {
                    return div().into_any_element();
                };
                let neighbor = |offset: isize| {
                    let index = ix.checked_add_signed(offset)?;
                    match self.rows.get(index)? {
                        Row::Message(id) => self.find_message(chat.info.id, *id),
                        _ => None,
                    }
                };
                let place = Place {
                    first: neighbor(-1).is_none_or(|m| !grouped(m, message)),
                    last: neighbor(1).is_none_or(|m| !grouped(message, m)),
                };
                self.message_row(message, chat, place, cx)
            }
            None => div().into_any_element(),
        };
        div().pb(bottom_pad).child(element).into_any_element()
    }

    fn message_row(
        &self,
        message: &Message,
        chat: &Chat,
        place: Place,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if is_service(&message.content) {
            return div()
                .py(px(4.))
                .flex()
                .justify_center()
                .child(service_pill(format!(
                    "{} {}",
                    self.sender_name(message),
                    service_action(&message.content)
                )))
                .into_any_element();
        }
        let outgoing = message.is_outgoing && !self.is_channel(chat);
        let show_avatars = !outgoing
            && !self.is_channel(chat)
            && !matches!(chat.info.r#type, tdlib_rs::enums::ChatType::Private(_));
        let chat_id = chat.info.id;
        let id = message.id;
        let bubble = self.bubble(message, chat, outgoing, place, cx);

        let can_edit = message.is_outgoing
            && matches!(
                message.content,
                C::MessageText(_)
                    | C::MessagePhoto(_)
                    | C::MessageVideo(_)
                    | C::MessageDocument(_)
                    | C::MessageAnimation(_)
                    | C::MessageAudio(_)
            )
            && message.forward_info.is_none();
        let can_delete = message.is_outgoing
            || matches!(chat.info.r#type, tdlib_rs::enums::ChatType::Private(_))
            || self.group(chat).is_some_and(|g| g.can_post && g.channel);
        let copy = match &message.content {
            C::MessageText(text) => text.text.text.clone(),
            C::MessagePhoto(m) => m.caption.text.clone(),
            C::MessageVideo(m) => m.caption.text.clone(),
            C::MessageDocument(m) => m.caption.text.clone(),
            other => message_text(other),
        };
        let retry =
            matches!(&message.sending_state, Some(MessageSendingState::Failed(f)) if f.can_retry);
        let pinned = message.is_pinned;
        // Emoji the user already reacted with; the menu offers to remove them.
        let chosen: Vec<String> = reactions(message)
            .into_iter()
            .filter(|chip| chip.chosen)
            .filter_map(|chip| match chip.kind {
                Chip::Emoji(emoji) => Some(emoji),
                _ => None,
            })
            .collect();
        let bubble = div()
            .id(("bubble", id as u64))
            .flex_shrink(1.)
            .min_w_0()
            .child(bubble)
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.start_reply(chat_id, id, window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    // The exact list arrives shortly; the row starts from the
                    // chat's own settings (see `reaction_row`).
                    let premium = this.users.get(&this.me).is_some_and(|u| u.is_premium);
                    this.send(Command::AvailableReactions {
                        chat_id,
                        message_id: id,
                        premium,
                    });
                    let mut entries = vec![
                        MenuEntry::Reactions {
                            chat_id,
                            message_id: id,
                            chosen: chosen.clone(),
                        },
                        MenuEntry::Separator,
                        MenuEntry::item(IconName::Reply, "Reply", move |this, window, cx| {
                            this.start_reply(chat_id, id, window, cx)
                        }),
                    ];
                    if can_edit {
                        entries.push(MenuEntry::item(
                            IconName::Pencil,
                            "Edit",
                            move |this, window, cx| this.start_edit(chat_id, id, window, cx),
                        ));
                    }
                    if !copy.is_empty() {
                        let copy = copy.clone();
                        entries.push(MenuEntry::item(
                            IconName::Copy,
                            "Copy Text",
                            move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                            },
                        ));
                    }
                    entries.push(MenuEntry::item(
                        IconName::Forward,
                        "Forward",
                        move |this, _, _| {
                            this.forward_from = Some((chat_id, id));
                        },
                    ));
                    entries.push(MenuEntry::item(
                        if pinned {
                            IconName::PinOff
                        } else {
                            IconName::Pin
                        },
                        if pinned { "Unpin" } else { "Pin" },
                        move |this, _, _| {
                            this.send(Command::PinMessage {
                                chat_id,
                                message_id: id,
                                pin: !pinned,
                            });
                        },
                    ));
                    if retry {
                        entries.push(MenuEntry::item(
                            IconName::RotateCw,
                            "Retry",
                            move |this, _, _| {
                                this.send(Command::Retry {
                                    chat_id,
                                    message_id: id,
                                });
                            },
                        ));
                    }
                    if can_delete {
                        entries.push(MenuEntry::Separator);
                        entries.push(
                            MenuEntry::item(IconName::Trash, "Delete", move |this, window, cx| {
                                this.confirm_delete(chat_id, id, window, cx)
                            })
                            .danger(),
                        );
                    }
                    this.open_menu(event.position, Anchor::TopLeft, entries, cx);
                    cx.stop_propagation();
                }),
            );

        div()
            .w_full()
            .flex()
            .items_end()
            .gap(px(7.))
            .when(self.highlight == Some(id), |el| {
                el.bg(ui::hsla_alpha(ui::ACCENT, 0x24))
            })
            .pl(px(if show_avatars { 10. } else { 16. }))
            .pr(px(16.))
            .pt(px(if place.first { 6. } else { 2. }))
            .when(outgoing, |el| el.justify_end().pl(px(56.)))
            .when(!outgoing, |el| el.pr(px(56.)))
            .when(show_avatars, |el| {
                el.child(
                    div()
                        .w(px(GROUP_AVATAR))
                        .flex_shrink_0()
                        .when(place.last, |el| {
                            el.child(match &message.sender_id {
                                tdlib_rs::enums::MessageSender::User(user) => {
                                    self.user_avatar(user.user_id, GROUP_AVATAR)
                                }
                                tdlib_rs::enums::MessageSender::Chat(sender) => {
                                    match self.chats.get(&sender.chat_id) {
                                        Some(chat) => self.chat_avatar(chat, GROUP_AVATAR),
                                        None => super::sidebar::letter_avatar(
                                            sender.chat_id,
                                            "?",
                                            GROUP_AVATAR,
                                        ),
                                    }
                                }
                            })
                        }),
                )
            })
            .child(bubble)
            .into_any_element()
    }

    fn bubble(
        &self,
        message: &Message,
        chat: &Chat,
        outgoing: bool,
        place: Place,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = message.id;
        let bg = if outgoing {
            ui::BUBBLE_OUT
        } else {
            ui::BUBBLE_IN
        };
        let meta_color = if outgoing { ui::META_OUT } else { ui::META_IN };

        // Header lines inside the bubble.
        let sender = (!outgoing
            && place.first
            && !self.is_channel(chat)
            && !matches!(chat.info.r#type, tdlib_rs::enums::ChatType::Private(_)))
        .then(|| {
            div()
                .text_size(px(ui::TEXT_MD))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ui::hsla(ui::peer_name_color(Self::sender_id(message))))
                .truncate()
                .child(self.sender_name(message))
        });
        let forwarded = message.forward_info.as_ref().map(|info| {
            div()
                .text_size(px(ui::TEXT_SM))
                .text_color(ui::hsla(if outgoing { ui::OUT_ACCENT } else { ui::LINK }))
                .truncate()
                .child(format!("Forwarded from {}", self.origin_name(&info.origin)))
        });
        let reply = self.reply_quote(message, chat, outgoing, cx);
        let has_header = sender.is_some() || forwarded.is_some() || reply.is_some();
        let header = div()
            .px(px(11.))
            .pt(px(7.))
            .flex()
            .flex_col()
            .gap(px(3.))
            .children(sender)
            .children(forwarded)
            .children(reply);

        // A run from one sender reads as one shape: corners on the sender's
        // side tighten between bubbles and to a tail at the end.
        let joined = |continues: bool| {
            px(if continues {
                ui::RADIUS_BUBBLE_JOINED
            } else {
                ui::RADIUS_BUBBLE_TAIL
            })
        };
        let open = px(ui::RADIUS_BUBBLE);
        let corners = if outgoing {
            Corners {
                tl: open,
                tr: if place.first { open } else { joined(true) },
                bl: open,
                br: joined(!place.last),
            }
        } else {
            Corners {
                tl: if place.first { open } else { joined(true) },
                tr: open,
                bl: joined(!place.last),
                br: open,
            }
        };
        let rounded = |el: Div| {
            el.rounded_tl(corners.tl)
                .rounded_tr(corners.tr)
                .rounded_bl(corners.bl)
                .rounded_br(corners.br)
        };

        // Stickers float without a bubble.
        if let C::MessageSticker(m) = &message.content {
            let file = sticker_file(&m.sticker).map(|f| self.file(f));
            let ready = file.filter(|f| f.local.is_downloading_completed);
            // Fit the sticker's own aspect ratio into a 168pt square. Images
            // need explicit pixel sizes: with percentage sizes GPUI lets the
            // image's natural ratio override the parent and it overflows.
            let (w, h) = (
                m.sticker.width.max(1) as f32,
                m.sticker.height.max(1) as f32,
            );
            let scale = STICKER / w.max(h);
            let (w, h) = ((w * scale).round(), (h * scale).round());
            let animation = self.animation(&m.sticker, true);
            let sticker = div()
                .relative()
                .w(px(w))
                .h(px(h))
                .child(match (animation, ready) {
                    // The current frame, advanced on our clock.
                    (Some(frame), _) => img(frame)
                        .w(px(w))
                        .h(px(h))
                        .rounded(px(ui::RADIUS_LG))
                        .object_fit(ObjectFit::Contain)
                        .id(("sticker-frames", id as u64))
                        .into_any_element(),
                    (None, ready) => match ready {
                        Some(file) => {
                            let emoji = m.sticker.emoji.clone();
                            img(PathBuf::from(&file.local.path))
                                .w(px(w))
                                .h(px(h))
                                .rounded(px(ui::RADIUS_LG))
                                .object_fit(ObjectFit::Contain)
                                .with_fallback(move || emoji_sticker(emoji.clone()))
                                .into_any_element()
                        }
                        None => div()
                            .size_full()
                            .rounded(px(ui::RADIUS_LG))
                            .bg(ui::hsla_alpha(ui::RAISED, 0x80))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(64.))
                            .child(m.sticker.emoji.clone())
                            .into_any_element(),
                    },
                })
                .child(
                    div()
                        .absolute()
                        .bottom(px(4.))
                        .right(px(4.))
                        .child(self.meta(message, chat, MetaStyle::Overlay)),
                );
            // Stickers have no bubble: reactions sit under them.
            return match self.reaction_chips(message, false, cx) {
                Some(chips) => div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .when(outgoing, |el| el.items_end())
                    .child(sticker)
                    .child(chips)
                    .into_any_element(),
                None => sticker.into_any_element(),
            };
        }

        let mut body = rounded(div())
            .relative()
            .max_w(px(MAX_BUBBLE))
            .min_w(px(64.))
            .bg(ui::hsla(bg))
            .shadow(ui::lift())
            .flex()
            .flex_col();

        if let Some(visual) = self.visual(message) {
            let caption = caption(&message.content).filter(|c| !c.text.is_empty());
            let (w, h) = fit(visual.width, visual.height, visual.round);
            // Media fills the bubble edge to edge, so it takes the bubble's own
            // corners (GPUI doesn't clip children to a parent's radius); edges
            // meeting a header or caption stay square.
            let square = px(0.);
            let (tl, tr) = if has_header {
                (square, square)
            } else {
                (corners.tl, corners.tr)
            };
            let (bl, br) = if caption.is_some() {
                (square, square)
            } else {
                (corners.bl, corners.br)
            };
            body = body
                .w(px(w))
                .when(has_header, |el| el.child(header.pb(px(6.))));
            let file = visual.open.clone();
            let photo = matches!(message.content, C::MessagePhoto(_));
            let round = visual.round;
            let chat_id = chat.info.id;
            let playback = self.playback_of(chat_id, id);
            let playing = playback.is_some_and(|p| p.is_playing());
            let live = playback.and_then(|p| p.frame.clone());
            let elapsed = playback.map(|p| ui::duration(p.time() as i32));
            let mut frame = div()
                .id(("media", id as u64))
                .relative()
                .w(px(w))
                .h(px(h))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if photo {
                        this.open_viewer(chat_id, id, cx);
                    } else if round {
                        this.toggle_play(chat_id, id, cx);
                    } else {
                        this.open_media(&file, cx);
                    }
                }));
            let shape = |el: Img| {
                if visual.round {
                    el.rounded_full()
                } else {
                    el.rounded_tl(tl)
                        .rounded_tr(tr)
                        .rounded_bl(bl)
                        .rounded_br(br)
                }
            };
            frame = match (visual.ready, visual.placeholder) {
                (Some(path), _) => frame.child(shape(
                    img(path).w(px(w)).h(px(h)).object_fit(ObjectFit::Cover),
                )),
                (None, Some(placeholder)) => frame.child(shape(
                    img(placeholder)
                        .w(px(w))
                        .h(px(h))
                        .object_fit(ObjectFit::Cover),
                )),
                (None, None) => frame.child(
                    div()
                        .size_full()
                        .bg(ui::hsla_alpha(0x000000, 0x30))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Spinner::new().color(ui::hsla(ui::MUTED))),
                ),
            };
            // A playing round video draws its live frames over the thumbnail.
            if let Some(live) = live {
                frame = frame.child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(img(live).w(px(w)).h(px(h)).rounded_full()),
                );
            }
            if visual.playable {
                let downloading =
                    self.downloading.contains(&visual.open.id) || self.play_pending(chat_id, id);
                let label = elapsed.or(visual.label);
                frame = frame
                    .when(!playing, |el| {
                        el.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    div()
                                        .size(px(48.))
                                        .rounded_full()
                                        .bg(ui::hsla_alpha(0x000000, 0x80))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(if downloading {
                                            Spinner::new()
                                                .color(ui::hsla(ui::WHITE))
                                                .into_any_element()
                                        } else {
                                            Icon::new(IconName::Play)
                                                .size(px(22.))
                                                .text_color(ui::hsla(ui::WHITE))
                                                .into_any_element()
                                        }),
                                ),
                        )
                    })
                    .when_some(label, |el, label| {
                        el.child(
                            div()
                                .absolute()
                                .top(px(6.))
                                .left(px(6.))
                                .px(px(6.))
                                .py(px(1.))
                                .rounded_full()
                                .bg(ui::hsla_alpha(0x000000, 0x80))
                                .text_size(px(ui::TEXT_SM))
                                .text_color(ui::hsla(ui::WHITE))
                                .child(label),
                        )
                    });
            }
            body = body.child(frame);
            match caption {
                Some(caption) => {
                    body = body.child(self.text_block(message, caption, outgoing, chat))
                }
                None => {
                    body = body.child(
                        div()
                            .absolute()
                            .bottom(px(6.))
                            .right(px(6.))
                            .child(self.meta(message, chat, MetaStyle::Overlay)),
                    );
                    if visual.round {
                        body = body.bg(gpui_kit::transparent_black()).shadow_none();
                    }
                }
            }
            return self.with_reactions(body, message, outgoing, cx);
        }

        body = body.child(header.when(!has_header, |el| el.pt_0()));

        let file_caption = caption(&message.content).filter(|c| !c.text.is_empty());
        // Without a caption the time sits on the file's status line.
        let file_meta = file_caption
            .is_none()
            .then(|| self.meta(message, chat, MetaStyle::Inline(meta_color)));
        if let Some(file_row) = self.file_row(message, outgoing, file_meta, cx) {
            body = body.child(
                div()
                    .px(px(11.))
                    .pt(px(8.))
                    .when(file_caption.is_none(), |el| el.pb(px(8.)))
                    .child(file_row),
            );
            if let Some(caption) = file_caption {
                body = body.child(self.text_block(message, caption, outgoing, chat));
            }
            return self.with_reactions(body, message, outgoing, cx);
        }

        if let C::MessagePoll(m) = &message.content {
            body = body
                .min_w(px(280.))
                .child(self.poll(message, &m.poll, outgoing, cx))
                .child(
                    div()
                        .px(px(11.))
                        .pb(px(6.))
                        .flex()
                        .justify_end()
                        .child(self.meta(message, chat, MetaStyle::Inline(meta_color))),
                );
            return self.with_reactions(body, message, outgoing, cx);
        }

        let text = match &message.content {
            C::MessageText(m) => m.text.clone(),
            other => FormattedText {
                text: message_text(other),
                entities: Vec::new(),
            },
        };
        body = body.child(self.text_block(message, &text, outgoing, chat));
        self.with_reactions(body, message, outgoing, cx)
    }

    /// Message text with the time tucked into the last line, like Telegram:
    /// trailing invisible spaces reserve room for the absolutely placed meta.
    fn text_block(
        &self,
        message: &Message,
        formatted: &FormattedText,
        outgoing: bool,
        chat: &Chat,
    ) -> AnyElement {
        let rich = ui::rich_text(formatted, if outgoing { ui::OUT_LINK } else { ui::LINK });
        let mut text = rich.text;
        let mut highlights = rich.highlights;
        let reserve = self.meta_width(message, chat);
        let start = text.len();
        // U+2007 FIGURE SPACE does not break, so the reserve stays on the last line.
        text.push(' ');
        text.push_str(&"\u{2007}".repeat((reserve / 7.).ceil() as usize));
        highlights.push((
            start..text.len(),
            HighlightStyle {
                color: Some(gpui_kit::transparent_black()),
                ..Default::default()
            },
        ));
        let mut styled = StyledText::new(text).with_highlights(highlights);
        if !rich.code.is_empty() {
            styled = styled.with_font_family_overrides(
                rich.code
                    .iter()
                    .map(|range| (range.clone(), "Menlo".into())),
            );
        }
        let element = if rich.links.is_empty() {
            styled.into_any_element()
        } else {
            let ranges = rich.links.iter().map(|l| l.range.clone()).collect();
            let urls: Vec<String> = rich.links.into_iter().map(|l| l.url).collect();
            InteractiveText::new(("text", message.id as u64), styled)
                .on_click(ranges, move |index, _, cx| {
                    if let Some(url) = urls.get(index) {
                        cx.open_url(url);
                    }
                })
                .into_any_element()
        };
        let meta_color = if outgoing { ui::META_OUT } else { ui::META_IN };
        div()
            .relative()
            .px(px(11.))
            .pt(px(6.))
            .pb(px(7.))
            .text_size(px(ui::TEXT_MD))
            .line_height(px(20.))
            .child(element)
            .child(
                div()
                    .absolute()
                    .right(px(9.))
                    .bottom(px(5.))
                    .child(self.meta(message, chat, MetaStyle::Inline(meta_color))),
            )
            .into_any_element()
    }

    fn meta_width(&self, message: &Message, chat: &Chat) -> f32 {
        let mut width = 30.;
        if message.edit_date > 0 {
            width += 38.;
        }
        if message.is_outgoing && !self.is_channel(chat) {
            width += 18.;
        }
        if let Some(views) = self.views(message, chat) {
            width += 22. + views.len() as f32 * 6.5;
        }
        width + 8.
    }

    fn views(&self, message: &Message, chat: &Chat) -> Option<String> {
        if !self.is_channel(chat) {
            return None;
        }
        let views = message.interaction_info.as_ref()?.view_count;
        (views > 0).then(|| compact(views))
    }

    fn meta(&self, message: &Message, chat: &Chat, style: MetaStyle) -> Div {
        let (color, overlay) = match style {
            MetaStyle::Inline(color) => (color, false),
            MetaStyle::Overlay => (ui::WHITE, true),
        };
        let status =
            (message.is_outgoing && !self.is_channel(chat)).then(|| match &message.sending_state {
                Some(MessageSendingState::Pending(_)) => (IconName::Clock, color),
                Some(MessageSendingState::Failed(_)) => (IconName::CircleAlert, ui::DANGER),
                None if self.is_self_chat(chat)
                    || message.id <= chat.info.last_read_outbox_message_id =>
                {
                    (
                        IconName::CheckCheck,
                        if overlay { ui::WHITE } else { ui::CHECK_OUT },
                    )
                }
                None => (
                    IconName::Check,
                    if overlay { ui::WHITE } else { ui::CHECK_OUT },
                ),
            });
        div()
            .flex()
            .items_center()
            .gap(px(3.))
            .text_size(px(ui::TEXT_SM))
            .line_height(px(16.))
            .text_color(ui::hsla(color))
            .when(overlay, |el| {
                el.px(px(6.))
                    .py(px(1.))
                    .rounded_full()
                    .bg(ui::hsla_alpha(0x000000, 0x70))
            })
            .when_some(self.views(message, chat), |el, views| {
                el.child(Icon::new(IconName::Eye).size(px(13.)))
                    .child(div().mr(px(3.)).child(views))
            })
            .when(message.edit_date > 0, |el| el.child("edited"))
            .child(ui::clock(message.date))
            .when_some(status, |el, (icon, color)| {
                el.child(Icon::new(icon).size(px(15.)).text_color(ui::hsla(color)))
            })
    }

    fn reply_quote(
        &self,
        message: &Message,
        chat: &Chat,
        outgoing: bool,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let tdlib_rs::enums::MessageReplyTo::Message(reply) = message.reply_to.as_ref()? else {
            return None;
        };
        let local = (reply.chat_id == 0 || reply.chat_id == chat.info.id)
            .then(|| self.find_message(chat.info.id, reply.message_id))
            .flatten();
        let (name, sender, text) = if let Some(original) = local {
            (
                self.sender_name(original),
                Self::sender_id(original),
                preview(&original.content).0,
            )
        } else {
            let name = reply
                .origin
                .as_ref()
                .map_or_else(|| "Message".into(), |o| self.origin_name(o));
            let text = reply
                .content
                .as_ref()
                .map_or_else(|| "Message".into(), |c| preview(c).0);
            (name, 0, text)
        };
        let text = reply
            .quote
            .as_ref()
            .map_or(text, |q| q.text.text.replace('\n', " "));
        let accent = if outgoing {
            ui::OUT_ACCENT
        } else if sender != 0 {
            ui::peer_name_color(sender)
        } else {
            ui::LINK
        };
        // Same-chat replies can always be jumped to, loaded or not.
        let chat_id = chat.info.id;
        let target = (reply.chat_id == 0 || reply.chat_id == chat_id).then_some(reply.message_id);
        Some(
            widgets::quote(
                accent,
                name,
                text,
                if outgoing {
                    ui::OUT_SOFT_TEXT
                } else {
                    ui::SECONDARY
                },
            )
            .id(("reply", message.id as u64))
            .mt(px(2.))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(target) = target {
                    this.jump_to(chat_id, target, cx);
                }
            }))
            .into_any_element(),
        )
        .map(|el| div().child(el))
    }

    fn visual(&self, message: &Message) -> Option<Visual> {
        if message.self_destruct_type.is_some() {
            return None;
        }
        let ready = |file: &tdlib_rs::types::File| {
            let file = self.file(file);
            (file.local.is_downloading_completed && !file.local.path.is_empty())
                .then(|| PathBuf::from(&file.local.path))
        };
        let thumb = video_thumbnail(&message.content).and_then(ready);
        let mini = |file: &tdlib_rs::types::File| self.minithumbs.get(&file.id).cloned();
        match &message.content {
            C::MessagePhoto(m) => {
                let size = crate::model::photo_size(&m.photo)?;
                Some(Visual {
                    width: size.width,
                    height: size.height,
                    ready: ready(&size.photo),
                    placeholder: mini(&size.photo),
                    open: size.photo.clone(),
                    playable: false,
                    label: None,
                    round: false,
                })
            }
            C::MessageVideo(m) => Some(Visual {
                width: m.video.width,
                height: m.video.height,
                ready: thumb,
                placeholder: mini(&m.video.video),
                open: m.video.video.clone(),
                playable: true,
                label: Some(ui::duration(m.video.duration)),
                round: false,
            }),
            C::MessageAnimation(m) => Some(Visual {
                width: m.animation.width,
                height: m.animation.height,
                ready: thumb,
                placeholder: mini(&m.animation.animation),
                open: m.animation.animation.clone(),
                playable: true,
                label: Some("GIF".into()),
                round: false,
            }),
            C::MessageVideoNote(m) => Some(Visual {
                width: 220,
                height: 220,
                ready: thumb,
                placeholder: mini(&m.video_note.video),
                open: m.video_note.video.clone(),
                playable: true,
                label: Some(ui::duration(m.video_note.duration)),
                round: true,
            }),
            _ => None,
        }
    }

    /// Document, audio and voice rows: round icon, name and size.
    fn file_row(
        &self,
        message: &Message,
        outgoing: bool,
        meta_row: Option<Div>,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let (file, title, subtitle, icon) = match &message.content {
            C::MessageDocument(m) => (
                &m.document.document,
                m.document.file_name.clone(),
                None,
                IconName::FileText,
            ),
            C::MessageAudio(m) => (
                &m.audio.audio,
                if m.audio.title.is_empty() {
                    m.audio.file_name.clone()
                } else {
                    m.audio.title.clone()
                },
                Some(if m.audio.performer.is_empty() {
                    ui::duration(m.audio.duration)
                } else {
                    format!("{} · {}", m.audio.performer, ui::duration(m.audio.duration))
                }),
                IconName::Play,
            ),
            C::MessageVoiceNote(m) => (
                &m.voice_note.voice,
                "Voice message".into(),
                Some(ui::duration(m.voice_note.duration)),
                IconName::Play,
            ),
            _ => return None,
        };
        let current = self.file(file).clone();
        let size = current.size.max(current.expected_size);
        let downloading = self.downloading.contains(&current.id);
        let complete = current.local.is_downloading_completed;
        let mut status = if downloading && size > 0 {
            format!(
                "{} / {}",
                ui::file_size(current.local.downloaded_size),
                ui::file_size(size)
            )
        } else {
            subtitle.unwrap_or_else(|| ui::file_size(size))
        };
        let icon = if complete || matches!(icon, IconName::Play) {
            icon
        } else {
            IconName::ArrowDown
        };
        let circle = if outgoing { ui::OUT_FILE } else { ui::BADGE };
        let file = current.clone();
        let chat_id = message.chat_id;
        let id = message.id;
        let voice = match &message.content {
            C::MessageVoiceNote(m) => Some(m.voice_note.waveform.clone()),
            _ => None,
        };
        let playable = voice.is_some() || matches!(message.content, C::MessageAudio(_));
        let playback = self.playback_of(chat_id, id);
        let active = playback.is_some();
        let playing = playback.is_some_and(|p| p.is_playing());
        let progress = playback.map_or(0., |p| p.progress());
        let busy = downloading || self.play_pending(chat_id, id);
        let icon = if playable {
            if playing {
                IconName::Pause
            } else {
                IconName::Play
            }
        } else {
            icon
        };
        let meta = if outgoing { ui::META_OUT } else { ui::META_IN };
        let (played, unplayed) = if outgoing {
            (ui::hsla(ui::OUT_ACCENT), ui::hsla_alpha(ui::META_OUT, 0x80))
        } else {
            (ui::hsla(ui::ACCENT), ui::hsla_alpha(ui::META_IN, 0x70))
        };
        if let Some(playback) = playback.filter(|_| playable) {
            status = ui::duration(playback.time() as i32);
        }
        let status = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(
                div()
                    .text_size(px(ui::TEXT_SM))
                    .text_color(ui::hsla(meta))
                    .child(status),
            )
            .when_some(meta_row, |el, meta_row| el.child(meta_row));
        // Seekable track: equal clickable segments, filled up to the playhead.
        let segment = |index: usize, count: usize, height: f32| {
            let fraction = index as f32 / count as f32;
            div()
                .id(ElementId::Name(format!("seek-{id}-{index}").into()))
                .h(px(24.))
                .flex()
                .items_center()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.playback_of(chat_id, id).is_some() {
                        this.seek(chat_id, id, fraction, cx);
                    } else {
                        this.toggle_play(chat_id, id, cx);
                    }
                }))
                .child(div().w_full().h(px(height)).rounded_full().bg(
                    if active && fraction < progress {
                        played
                    } else {
                        unplayed
                    },
                ))
        };
        let detail: AnyElement =
            if let Some(waveform) = voice {
                let bars = super::playback::waveform(&waveform, 36);
                let count = bars.len();
                div()
                    .flex()
                    .flex_col()
                    .child(div().flex().items_center().gap(px(2.)).children(
                        bars.into_iter().enumerate().map(|(index, level)| {
                            segment(index, count, 4. + level * 18.).w(px(3.))
                        }),
                    ))
                    .child(status)
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .when(active, |el| {
                        el.child(
                            div()
                                .w(px(180.))
                                .flex()
                                .children((0..40).map(|index| segment(index, 40, 3.).flex_1())),
                        )
                    })
                    .child(status)
                    .into_any_element()
            };
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .min_w(px(220.))
                .child(
                    div()
                        .id(("file", message.id as u64))
                        .size(px(44.))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(ui::hsla(circle))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.85))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if playable {
                                this.toggle_play(chat_id, id, cx);
                            } else {
                                this.open_media(&file, cx);
                            }
                        }))
                        .child(if busy {
                            Spinner::new().color(ui::hsla(ui::WHITE)).into_any_element()
                        } else {
                            Icon::new(icon)
                                .size(px(20.))
                                .text_color(ui::hsla(ui::WHITE))
                                .into_any_element()
                        }),
                )
                .child(detail),
        )
    }

    fn poll(
        &self,
        message: &Message,
        poll: &tdlib_rs::types::Poll,
        outgoing: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let chat_id = message.chat_id;
        let id = message.id;
        let multiple = matches!(&poll.r#type, tdlib_rs::enums::PollType::Regular(p) if p.allow_multiple_answers);
        let quiz = matches!(poll.r#type, tdlib_rs::enums::PollType::Quiz(_));
        let voted = poll.options.iter().any(|o| o.is_chosen);
        let results = voted || poll.is_closed;
        let choices = self.poll_choices.get(&(chat_id, id));
        let meta = if outgoing { ui::META_OUT } else { ui::META_IN };
        let bar = if outgoing { ui::OUT_ACCENT } else { ui::ACCENT };
        let mut column = div()
            .px(px(11.))
            .pt(px(8.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(ui::TEXT_LG))
                    .child(poll.question.text.clone()),
            )
            .child(
                div()
                    .text_size(px(ui::TEXT_SM))
                    .text_color(ui::hsla(meta))
                    .mb(px(6.))
                    .child(match (quiz, poll.is_anonymous) {
                        (true, _) => "Quiz",
                        (false, true) => "Anonymous poll",
                        (false, false) => "Public poll",
                    }),
            );
        for (index, option) in poll.options.iter().enumerate() {
            let index = index as i32;
            let checked = choices.map_or(option.is_chosen, |c| c.contains(&index));
            let percent = option.vote_percentage;
            column = column.child(
                div()
                    .id(("poll-option", ((id as u64) << 8) | index as u64))
                    .py(px(5.))
                    .flex()
                    .gap(px(10.))
                    .when(!results, |el| {
                        el.cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if multiple {
                                    let choices =
                                        this.poll_choices.entry((chat_id, id)).or_default();
                                    if choices.contains(&index) {
                                        choices.retain(|v| *v != index);
                                    } else {
                                        choices.push(index);
                                    }
                                } else {
                                    this.send(Command::Vote {
                                        chat_id,
                                        message_id: id,
                                        options: vec![index],
                                    });
                                }
                                cx.notify();
                            }))
                    })
                    .child(if results {
                        // Same line box as the label, so both share a center.
                        div()
                            .w(px(34.))
                            .h(px(20.))
                            .flex()
                            .items_center()
                            .flex_shrink_0()
                            .text_size(px(ui::TEXT_SM))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{percent}%"))
                    } else {
                        div()
                            .size(px(18.))
                            .mt(px(1.))
                            .flex_shrink_0()
                            .when(multiple, |el| el.rounded(px(ui::RADIUS_SM)))
                            .when(!multiple, |el| el.rounded_full())
                            .border_2()
                            .border_color(ui::hsla(meta))
                            .when(checked, |el| {
                                el.bg(ui::hsla(bar)).border_color(ui::hsla(bar))
                            })
                    })
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .h(px(20.))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(option.text.text.clone())
                                    .when(results && option.is_chosen, |el| {
                                        el.child(
                                            Icon::new(IconName::CircleCheck)
                                                .size(px(ui::ICON_SM))
                                                .text_color(ui::hsla(bar)),
                                        )
                                    }),
                            )
                            .when(results, |el| {
                                el.child(
                                    div()
                                        .h(px(3.))
                                        .rounded_full()
                                        .bg(ui::hsla_alpha(bar, 0x30))
                                        .child(
                                            div()
                                                .h_full()
                                                .rounded_full()
                                                .w(relative(percent.clamp(1, 100) as f32 / 100.))
                                                .bg(ui::hsla(bar)),
                                        ),
                                )
                            }),
                    ),
            );
        }
        if multiple && !results {
            let ready = choices.is_some_and(|c| !c.is_empty());
            column = column.child(
                div()
                    .id(("poll-submit", id as u64))
                    .mt(px(4.))
                    .py(px(6.))
                    .flex()
                    .justify_center()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ui::hsla(if ready { bar } else { meta }))
                    .when(ready, |el| {
                        el.cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(options) = this.poll_choices.remove(&(chat_id, id)) {
                                    this.send(Command::Vote {
                                        chat_id,
                                        message_id: id,
                                        options,
                                    });
                                }
                                cx.notify();
                            }))
                    })
                    .child("Vote"),
            );
        }
        column.child(
            div()
                .mt(px(4.))
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(ui::TEXT_SM))
                .text_color(ui::hsla(meta))
                .child(ui::count_label(poll.total_voter_count, "vote", "votes"))
                .when(voted && !quiz && !poll.is_closed, |el| {
                    el.child(
                        div()
                            .id(("poll-retract", id as u64))
                            .cursor_pointer()
                            .text_color(ui::hsla(bar))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.send(Command::Vote {
                                    chat_id,
                                    message_id: id,
                                    options: Vec::new(),
                                });
                                cx.notify();
                            }))
                            .child("Retract vote"),
                    )
                })
                .when(poll.is_closed, |el| el.child("· Final results")),
        )
    }

    fn with_reactions(
        &self,
        body: Div,
        message: &Message,
        outgoing: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.reaction_chips(message, outgoing, cx) {
            Some(chips) => body
                .child(chips.px(px(11.)).pb(px(8.)).mt(px(-2.)))
                .into_any_element(),
            None => body.into_any_element(),
        }
    }

    /// Reaction chips: emoji, custom emoji (as their images, once fetched)
    /// and Telegram's paid star reaction, each with its count.
    fn reaction_chips(
        &self,
        message: &Message,
        outgoing: bool,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let list = reactions(message);
        if list.is_empty() {
            return None;
        }
        let chat_id = message.chat_id;
        let id = message.id;
        let chips = list.into_iter().enumerate().map(|(index, chip)| {
            let chosen = chip.chosen;
            // Quiet: a faint shade of whatever the chip sits on (the text
            // color at low alpha), so it never fights the bubble. Your own
            // reaction is marked with an accent outline and accent count.
            let accent = if outgoing { ui::OUT_ACCENT } else { ui::ACCENT };
            let bg = ui::hsla_alpha(ui::TEXT, 0x10);
            let (border, fg) = if chosen {
                (ui::hsla(accent), ui::hsla(accent))
            } else {
                (
                    gpui_kit::transparent_black(),
                    ui::hsla(if outgoing {
                        ui::OUT_SOFT_TEXT
                    } else {
                        ui::SECONDARY
                    }),
                )
            };
            let reaction = match &chip.kind {
                Chip::Emoji(emoji) => Some(Reaction::Emoji(emoji.clone())),
                Chip::Custom(custom) => Some(Reaction::Custom(*custom)),
                // Adding stars spends money; leave that to the official app.
                Chip::Paid => None,
            };
            div()
                .id(ElementId::Name(format!("reaction-{id}-{index}").into()))
                .h(px(28.))
                .pl(px(7.))
                .pr(px(10.))
                .rounded_full()
                .bg(bg)
                // Every chip has the border, so marking one doesn't resize it.
                .border_1()
                .border_color(border)
                .text_color(fg)
                .flex()
                .items_center()
                .gap(px(5.))
                .when_some(reaction, |el, reaction| {
                    el.cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.send(Command::React {
                                chat_id,
                                message_id: id,
                                reaction: reaction.clone(),
                                remove: chosen,
                            });
                            cx.notify();
                        }))
                })
                .child(self.reaction_glyph(&chip.kind))
                .child(
                    div()
                        .text_size(px(ui::TEXT_SM))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(compact(chip.count)),
                )
        });
        Some(div().flex().flex_wrap().gap(px(6.)).children(chips))
    }

    /// The reaction itself inside a chip: the emoji, or a custom emoji's
    /// image (its first frame for animated ones) with its plain emoji as the
    /// stand-in until it loads.
    fn reaction_glyph(&self, kind: &Chip) -> AnyElement {
        let text = |label: String| {
            div()
                .text_size(px(16.))
                .line_height(px(18.))
                .child(label)
                .into_any_element()
        };
        match kind {
            Chip::Emoji(emoji) => text(emoji.clone()),
            Chip::Paid => text("⭐".into()),
            Chip::Custom(custom) => {
                let Some(Some(sticker)) = self.custom_emoji.get(custom) else {
                    return text("•".into());
                };
                let size = px(18.);
                if let Some(frame) = self.animation(sticker, false) {
                    return img(frame).size(size).into_any_element();
                }
                match sticker_file(sticker)
                    .map(|f| self.file(f))
                    .filter(|f| f.local.is_downloading_completed)
                {
                    Some(file) => {
                        let emoji = sticker.emoji.clone();
                        img(PathBuf::from(&file.local.path))
                            .size(size)
                            .object_fit(ObjectFit::Contain)
                            .with_fallback(move || div().child(emoji.clone()).into_any_element())
                            .into_any_element()
                    }
                    None => text(sticker.emoji.clone()),
                }
            }
        }
    }
}

enum MetaStyle {
    Inline(ui::Color),
    Overlay,
}

struct Visual {
    width: i32,
    height: i32,
    ready: Option<PathBuf>,
    placeholder: Option<std::sync::Arc<RenderImage>>,
    open: tdlib_rs::types::File,
    playable: bool,
    label: Option<String>,
    round: bool,
}

fn grouped(above: &Message, below: &Message) -> bool {
    !is_service(&above.content)
        && !is_service(&below.content)
        && above.is_outgoing == below.is_outgoing
        && Messenger::sender_id(above) == Messenger::sender_id(below)
        && (below.date - above.date).abs() < GROUP_SECONDS
        && ui::same_day(above.date, below.date)
}

/// Per-corner radii of a bubble.
#[derive(Clone, Copy)]
struct Corners {
    tl: Pixels,
    tr: Pixels,
    bl: Pixels,
    br: Pixels,
}

/// Fits media into Telegram's bubble bounds, keeping its aspect ratio.
fn fit(width: i32, height: i32, round: bool) -> (f32, f32) {
    if round {
        return (220., 220.);
    }
    let (w, h) = (width.max(1) as f32, height.max(1) as f32);
    let scale = (MAX_BUBBLE - 60.) / w;
    let scale = scale.min(420. / h);
    let (mut w, mut h) = (w * scale, h * scale);
    if w < 160. {
        h *= 160. / w;
        w = 160.;
    }
    (w.round(), h.clamp(90., 420.).round())
}

fn caption(content: &C) -> Option<&FormattedText> {
    match content {
        C::MessagePhoto(m) => Some(&m.caption),
        C::MessageVideo(m) => Some(&m.caption),
        C::MessageAnimation(m) => Some(&m.caption),
        C::MessageDocument(m) => Some(&m.caption),
        C::MessageAudio(m) => Some(&m.caption),
        C::MessageVoiceNote(m) => Some(&m.caption),
        _ => None,
    }
}

/// What a reaction chip shows.
enum Chip {
    Emoji(String),
    Custom(i64),
    /// Telegram Stars sent to a channel post.
    Paid,
}

struct ReactionChip {
    kind: Chip,
    count: i32,
    chosen: bool,
}

fn reactions(message: &Message) -> Vec<ReactionChip> {
    message
        .interaction_info
        .as_ref()
        .and_then(|info| info.reactions.as_ref())
        .map(|reactions| {
            reactions
                .reactions
                .iter()
                .map(|r| ReactionChip {
                    kind: match &r.r#type {
                        tdlib_rs::enums::ReactionType::Emoji(e) => Chip::Emoji(e.emoji.clone()),
                        tdlib_rs::enums::ReactionType::CustomEmoji(c) => {
                            Chip::Custom(c.custom_emoji_id)
                        }
                        tdlib_rs::enums::ReactionType::Paid => Chip::Paid,
                    },
                    count: r.total_count,
                    chosen: r.is_chosen,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn compact(count: i32) -> String {
    if count < 1_000 {
        count.to_string()
    } else if count < 1_000_000 {
        format!("{:.1}K", count as f32 / 1_000.).replace(".0K", "K")
    } else {
        format!("{:.1}M", count as f32 / 1_000_000.).replace(".0M", "M")
    }
}

/// Stand-in for stickers GPUI can't draw (animated or undecodable).
pub(super) fn emoji_sticker(emoji: String) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(64.))
        .child(emoji)
        .into_any_element()
}

pub(super) fn service_pill(text: String) -> Div {
    super::widgets::pill(text)
}
