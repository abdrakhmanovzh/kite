use std::path::PathBuf;

use gpui_kit::{
    assets::IconName,
    component::{Icon, scroll::ScrollableElement},
    prelude::*,
    *,
};

use super::{Messenger, grid_file, widgets};
use crate::{model::List, telegram::Command, ui};

/// Shared-media thumbnail side: three across the 340px panel (338 inside
/// its border) within the gutters, with `TILE_GAP` between.
const TILE: f32 = 98.;
const TILE_GAP: f32 = 6.;

impl Messenger {
    pub(super) fn detail_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(chat) = self.selected.and_then(|id| self.chats.get(&id)) else {
            return div().into_any_element();
        };
        let id = chat.info.id;
        let archived = chat.order(List::Archive) > 0;
        let list = self.list;
        let pinned = chat
            .info
            .positions
            .iter()
            .any(|p| p.list == list.td() && p.is_pinned);
        let muted = chat.info.notification_settings.mute_for > 0;
        let mut settings = chat.info.notification_settings.clone();
        settings.use_default_mute_for = false;
        settings.mute_for = if muted { 0 } else { i32::MAX };
        let user = match &chat.info.r#type {
            tdlib_rs::enums::ChatType::Private(user) => self.users.get(&user.user_id),
            _ => None,
        };
        let title = match &chat.info.r#type {
            tdlib_rs::enums::ChatType::Private(_) => "Info",
            _ if self.is_channel(chat) => "Channel",
            _ => "Group",
        };
        let (status, active) = self.chat_status(chat);
        let about = self
            .profiles
            .get(&id)
            .map(|p| p.about.clone())
            .filter(|a| !a.is_empty());
        let username = user
            .and_then(|u| u.usernames.as_ref())
            .and_then(|names| names.active_usernames.first())
            .map(|name| format!("@{name}"));
        let phone = user
            .filter(|u| !u.phone_number.is_empty())
            .map(|u| format!("+{}", u.phone_number));
        let shared = self.shared_media.get(&id);
        let counts: Vec<_> = shared
            .map(|m| {
                [
                    (IconName::Image, "Photos", m.photos),
                    (IconName::Film, "Videos", m.videos),
                    (IconName::FileText, "Files", m.files),
                    (IconName::Link, "Links", m.links),
                    (IconName::Mic, "Voice messages", m.voice),
                ]
                .into_iter()
                .filter(|(_, _, count)| *count > 0)
                .collect()
            })
            .unwrap_or_default();
        let grid: Vec<_> = shared
            .map(|m| {
                m.recent
                    .iter()
                    .filter_map(|message| {
                        grid_file(message).map(|f| (message.id, self.file(f).clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let info: Vec<(IconName, String, &'static str)> = [
            about.map(|a| {
                (
                    IconName::Info,
                    a,
                    if user.is_some() { "Bio" } else { "Description" },
                )
            }),
            username.map(|n| (IconName::AtSign, n, "Username")),
            phone.map(|p| (IconName::Phone, p, "Mobile")),
        ]
        .into_iter()
        .flatten()
        .collect();

        div()
            .id("profile-panel")
            .size_full()
            .flex()
            .flex_col()
            .child(widgets::panel_header(
                title,
                widgets::icon_button("close-details", IconName::X).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.details = false;
                        cx.notify();
                    },
                )),
            ))
            .child(
                div()
                    .id("profile-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        div()
                            .px(px(ui::GUTTER))
                            .pt(px(24.))
                            .pb(px(ui::GUTTER))
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            // Identity
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .child(
                                        div()
                                            .id("profile-avatar")
                                            .rounded_full()
                                            .when(chat.info.photo.is_some(), |el| {
                                                el.cursor_pointer()
                                                    .hover(|s| s.opacity(0.85))
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.open_avatar(id, cx)
                                                    }))
                                            })
                                            .child(self.chat_avatar(chat, ui::AVATAR_XL)),
                                    )
                                    .child(
                                        div()
                                            .mt(px(14.))
                                            .text_size(px(ui::TEXT_XL))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_center()
                                            .child(self.chat_title(chat)),
                                    )
                                    .when(!status.is_empty(), |el| {
                                        el.child(
                                            div()
                                                .mt(px(2.))
                                                .text_size(px(ui::TEXT_SM))
                                                .text_color(ui::hsla(if active {
                                                    ui::ACCENT
                                                } else {
                                                    ui::MUTED
                                                }))
                                                .child(status),
                                        )
                                    }),
                            )
                            // Quick actions
                            .child(
                                div()
                                    .mt(px(8.))
                                    .flex()
                                    .gap(px(8.))
                                    .child(
                                        action_tile(
                                            "profile-mute",
                                            if muted {
                                                IconName::Bell
                                            } else {
                                                IconName::BellOff
                                            },
                                            if muted { "Unmute" } else { "Mute" },
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.send(Command::Mute {
                                                    chat_id: id,
                                                    settings: settings.clone(),
                                                });
                                                cx.notify();
                                            }),
                                        ),
                                    )
                                    .child(
                                        action_tile("profile-search", IconName::Search, "Search")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                if !this.message_search_open {
                                                    this.toggle_search(window, cx);
                                                }
                                            })),
                                    )
                                    .child(
                                        action_tile(
                                            "profile-pin",
                                            if pinned {
                                                IconName::PinOff
                                            } else {
                                                IconName::Pin
                                            },
                                            if pinned { "Unpin" } else { "Pin" },
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.send(Command::Pin {
                                                    chat_id: id,
                                                    list,
                                                    pinned: !pinned,
                                                });
                                                cx.notify();
                                            }),
                                        ),
                                    )
                                    .child(
                                        action_tile(
                                            "profile-archive",
                                            if archived {
                                                IconName::ArchiveRestore
                                            } else {
                                                IconName::Archive
                                            },
                                            if archived { "Unarchive" } else { "Archive" },
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.send(Command::Archive {
                                                    chat_id: id,
                                                    archive: !archived,
                                                });
                                                cx.notify();
                                            }),
                                        ),
                                    ),
                            )
                            // About, username, phone
                            .when(!info.is_empty(), |el| {
                                el.child(group(info.into_iter().map(|(icon, value, label)| {
                                    group_row(icon)
                                        .items_start()
                                        .py(px(10.))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .flex()
                                                .flex_col()
                                                .gap(px(2.))
                                                .child(
                                                    div()
                                                        .text_size(px(ui::TEXT_MD))
                                                        .line_height(px(20.))
                                                        .child(value),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(ui::TEXT_SM))
                                                        .text_color(ui::hsla(ui::MUTED))
                                                        .child(label),
                                                ),
                                        )
                                        .into_any_element()
                                })))
                            })
                            // Media counts
                            .when(!counts.is_empty(), |el| {
                                el.child(group(counts.into_iter().map(|(icon, label, count)| {
                                    group_row(icon)
                                        .h(px(ui::CONTROL_LG))
                                        .child(
                                            div().flex_1().text_size(px(ui::TEXT_MD)).child(label),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(ui::TEXT_SM))
                                                .text_color(ui::hsla(ui::MUTED))
                                                .child(count.to_string()),
                                        )
                                        .into_any_element()
                                })))
                            })
                            // Recent photos and videos
                            .when(!grid.is_empty(), |el| {
                                el.child(div().child(widgets::section_label("Shared media")).child(
                                    div().flex().flex_wrap().gap(px(TILE_GAP)).children(
                                        grid.into_iter().map(|(message_id, file)| {
                                            self.media_tile(id, message_id, file, cx)
                                        }),
                                    ),
                                ))
                            }),
                    ),
            )
            .into_any_element()
    }

    fn media_tile(
        &self,
        chat_id: i64,
        message_id: i64,
        file: tdlib_rs::types::File,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let ready = file.local.is_downloading_completed;
        let path = file.local.path.clone();
        div()
            .id(("shared", message_id as u64))
            .size(px(TILE))
            .rounded(px(ui::RADIUS_MD))
            .bg(ui::hsla(ui::RAISED))
            .cursor_pointer()
            .hover(|s| s.opacity(0.85))
            .on_click(cx.listener(move |this, _, _, cx| {
                // Photos already in the loaded history open in the viewer.
                if this.find_message(chat_id, message_id).is_some_and(|m| {
                    matches!(m.content, tdlib_rs::enums::MessageContent::MessagePhoto(_))
                }) {
                    this.open_viewer(chat_id, message_id, cx);
                    return;
                }
                if let Some(message) = this
                    .shared_media
                    .get(&chat_id)
                    .and_then(|m| m.recent.iter().find(|m| m.id == message_id))
                    && let Some((file, _)) = crate::model::media(&message.content)
                {
                    let file = file.clone();
                    this.open_media(&file, cx);
                }
            }))
            .when(ready, |el| {
                el.child(
                    img(PathBuf::from(path))
                        .size(px(TILE))
                        .rounded(px(ui::RADIUS_MD))
                        .object_fit(ObjectFit::Cover),
                )
            })
    }
}

/// Square-ish tile with an icon over a label; the profile's quick actions.
fn action_tile(id: &'static str, icon: IconName, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .h(px(60.))
        .rounded(px(ui::RADIUS_MD))
        .bg(ui::hsla(ui::BASE))
        .border_1()
        .border_color(ui::hsla(ui::STROKE))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(5.))
        .cursor_pointer()
        .hover(|s| s.bg(ui::hsla(ui::RAISED)))
        .active(|s| s.bg(ui::hsla(ui::HOVER_RAISED)))
        .child(
            Icon::new(icon)
                .size(px(ui::ICON_LG))
                .text_color(ui::hsla(ui::ACCENT)),
        )
        .child(
            div()
                .text_size(px(ui::TEXT_XS))
                .font_weight(FontWeight::MEDIUM)
                .text_color(ui::hsla(ui::SECONDARY))
                .child(label),
        )
}

/// Rounded block of rows separated by hairlines, settings style.
fn group(rows: impl Iterator<Item = AnyElement>) -> Div {
    let mut block = div()
        .rounded(px(ui::RADIUS_MD))
        .bg(ui::hsla(ui::BASE))
        .border_1()
        .border_color(ui::hsla(ui::STROKE))
        .flex()
        .flex_col();
    for (index, row) in rows.enumerate() {
        if index > 0 {
            block = block.child(div().ml(px(46.)).h(px(1.)).bg(ui::hsla(ui::STROKE)));
        }
        block = block.child(row);
    }
    block
}

fn group_row(icon: IconName) -> Div {
    div().px(px(14.)).flex().items_center().gap(px(14.)).child(
        Icon::new(icon)
            .size(px(ui::ICON_LG))
            .text_color(ui::hsla(ui::MUTED)),
    )
}
