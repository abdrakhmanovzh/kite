//! Full-window photo viewer with keyboard navigation through the chat's photos.

use std::path::PathBuf;

use gpui_kit::{
    assets::IconName,
    component::{Disableable, Icon},
    prelude::*,
    *,
};
use tdlib_rs::{enums::MessageContent as C, types::Message};

use super::{Messenger, open_file, widgets};
use crate::{telegram::Command, ui};

pub(super) struct Viewer {
    pub chat_id: i64,
    pub message_id: i64,
    /// Showing the chat's profile photos rather than a message's photo.
    pub avatar: bool,
    /// Position in the profile photos, newest first.
    pub index: usize,
}

/// Keys the viewer handles while open.
pub(super) enum ViewerKey {
    Previous,
    Next,
    Close,
}

impl ViewerKey {
    pub(super) fn from(keystroke: &Keystroke) -> Option<Self> {
        let m = &keystroke.modifiers;
        if m.platform || m.alt || m.control || m.shift {
            return None;
        }
        Some(match keystroke.key.as_str() {
            "left" => Self::Previous,
            "right" => Self::Next,
            "escape" => Self::Close,
            _ => return None,
        })
    }
}

/// What the viewer displays: the full-size file, the best one already on
/// disk, and the text around it.
struct Shown {
    full_file: tdlib_rs::types::File,
    shown: Option<tdlib_rs::types::File>,
    width: i32,
    height: i32,
    sender: String,
    date: String,
    caption: String,
}

/// The highest-resolution size of a photo, for viewing.
fn full_size(message: &Message) -> Option<&tdlib_rs::types::PhotoSize> {
    match &message.content {
        C::MessagePhoto(m) => m.photo.sizes.iter().max_by_key(|s| s.width),
        _ => None,
    }
}

impl Messenger {
    pub(super) fn open_viewer(&mut self, chat_id: i64, message_id: i64, cx: &mut Context<Self>) {
        self.viewer = Some(Viewer {
            chat_id,
            message_id,
            avatar: false,
            index: 0,
        });
        self.prefetch_viewer();
        cx.notify();
    }

    /// Opens the chat's profile photo full size; older ones load alongside
    /// and page with the arrows.
    pub(super) fn open_avatar(&mut self, chat_id: i64, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(&chat_id) else {
            return;
        };
        let Some(photo) = chat.info.photo.clone() else {
            return;
        };
        let kind = chat.info.r#type.clone();
        self.request_file(&photo.big, 32, false);
        if !self.profile_photos.contains_key(&chat_id) {
            self.send(Command::ProfilePhotos { chat_id, kind });
        }
        self.viewer = Some(Viewer {
            chat_id,
            message_id: 0,
            avatar: true,
            index: 0,
        });
        self.prefetch_viewer();
        cx.notify();
    }

    /// The chat's profile photos, once loaded.
    fn avatars(&self, chat_id: i64) -> &[tdlib_rs::types::ChatPhoto] {
        self.profile_photos
            .get(&chat_id)
            .map_or(&[], |photos| photos.as_slice())
    }

    /// (position, count) of what the viewer shows, for the counter and arrows.
    fn viewer_position(&self) -> Option<(usize, usize)> {
        let viewer = self.viewer.as_ref()?;
        if viewer.avatar {
            let count = self.avatars(viewer.chat_id).len().max(1);
            return Some((viewer.index.min(count - 1), count));
        }
        let photos = self.viewer_photos();
        let index = photos.iter().position(|&id| id == viewer.message_id)?;
        Some((index, photos.len()))
    }

    /// Returns whether the key was consumed.
    pub(super) fn viewer_key(&mut self, key: ViewerKey, cx: &mut Context<Self>) -> bool {
        if self.viewer.is_none() {
            return false;
        }
        match key {
            ViewerKey::Close => self.viewer = None,
            ViewerKey::Previous => self.step_viewer(-1),
            ViewerKey::Next => self.step_viewer(1),
        }
        cx.notify();
        true
    }

    /// Photo message ids of the viewed chat, oldest first.
    fn viewer_photos(&self) -> Vec<i64> {
        let Some(viewer) = self.viewer.as_ref().filter(|v| !v.avatar) else {
            return Vec::new();
        };
        self.chats
            .get(&viewer.chat_id)
            .map_or_else(Vec::new, |chat| {
                chat.messages
                    .values()
                    .filter(|m| matches!(m.content, C::MessagePhoto(_)))
                    .map(|m| m.id)
                    .collect()
            })
    }

    fn step_viewer(&mut self, delta: isize) {
        let photos = self.viewer_photos();
        let count = self
            .viewer
            .as_ref()
            .map_or(0, |v| self.avatars(v.chat_id).len());
        let Some(viewer) = &mut self.viewer else {
            return;
        };
        if viewer.avatar {
            if let Some(next) = viewer.index.checked_add_signed(delta)
                && next < count
            {
                viewer.index = next;
            }
            self.prefetch_viewer();
            return;
        }
        if let Some(index) = photos.iter().position(|&id| id == viewer.message_id)
            && let Some(&next) = photos.get(index.wrapping_add_signed(delta))
        {
            viewer.message_id = next;
        }
        self.prefetch_viewer();
    }

    /// Downloads the full-size photo for the viewed message and its neighbours.
    pub(super) fn prefetch_viewer(&mut self) {
        let photos = self.viewer_photos();
        let Some(viewer) = &self.viewer else {
            return;
        };
        let chat_id = viewer.chat_id;
        if viewer.avatar {
            let index = viewer.index;
            let wanted: Vec<_> = [index.checked_sub(1), Some(index), Some(index + 1)]
                .into_iter()
                .flatten()
                .filter_map(|i| {
                    let photo = self.avatars(chat_id).get(i)?;
                    let full = photo.sizes.iter().max_by_key(|s| s.width)?;
                    Some((full.photo.clone(), i == index))
                })
                .collect();
            for (file, current) in wanted {
                self.request_file(&file, if current { 32 } else { 12 }, false);
            }
            return;
        }
        let Some(index) = photos.iter().position(|&id| id == viewer.message_id) else {
            return;
        };
        let wanted: Vec<_> = [index.checked_sub(1), Some(index), Some(index + 1)]
            .into_iter()
            .flatten()
            .filter_map(|i| photos.get(i))
            .filter_map(|&id| {
                self.find_message(chat_id, id)
                    .and_then(full_size)
                    .map(|size| (size.photo.clone(), id == viewer.message_id))
            })
            .collect();
        for (file, current) in wanted {
            self.request_file(&file, if current { 32 } else { 12 }, false);
        }
    }

    fn photo_shown(&self, chat_id: i64, message_id: i64) -> Option<Shown> {
        let message = self.find_message(chat_id, message_id)?;
        let C::MessagePhoto(content) = &message.content else {
            return None;
        };
        let full = full_size(message)?;
        // Show the best version already on disk while the full size downloads.
        let shown = std::iter::once(full)
            .chain(content.photo.sizes.iter().rev())
            .map(|size| self.file(&size.photo))
            .find(|file| file.local.is_downloading_completed && !file.local.path.is_empty())
            .cloned();
        Some(Shown {
            full_file: self.file(&full.photo).clone(),
            shown,
            width: full.width,
            height: full.height,
            sender: self.sender_name(message),
            date: ui::local(message.date)
                .map(|t| t.format("%b %-d, %Y at %H:%M").to_string())
                .unwrap_or_default(),
            caption: content.caption.text.clone(),
        })
    }

    fn avatar_shown(&self, chat_id: i64, index: usize) -> Option<Shown> {
        let chat = self.chats.get(&chat_id)?;
        let photo = chat.info.photo.as_ref()?;
        let ready = |file: &tdlib_rs::types::File| {
            file.local.is_downloading_completed && !file.local.path.is_empty()
        };
        if let Some(listed) = self.avatars(chat_id).get(index)
            && let Some(full) = listed.sizes.iter().max_by_key(|s| s.width)
        {
            // Best size on disk; the newest also falls back to the chat photo.
            let shown = std::iter::once(full)
                .chain(listed.sizes.iter().rev())
                .map(|size| self.file(&size.photo))
                .chain(
                    (index == 0)
                        .then_some([&photo.big, &photo.small])
                        .into_iter()
                        .flatten()
                        .map(|f| self.file(f)),
                )
                .find(|file| ready(file))
                .cloned();
            return Some(Shown {
                full_file: self.file(&full.photo).clone(),
                shown,
                width: full.width,
                height: full.height,
                sender: self.chat_title(chat),
                date: ui::local(listed.added_date)
                    .map(|t| format!("Profile photo · {}", t.format("%b %-d, %Y")))
                    .unwrap_or_else(|| "Profile photo".into()),
                caption: String::new(),
            });
        }
        let shown = [&photo.big, &photo.small]
            .into_iter()
            .map(|file| self.file(file))
            .find(|file| ready(file))
            .cloned();
        // Telegram profile photos are square, 640px at full size.
        Some(Shown {
            full_file: self.file(&photo.big).clone(),
            shown,
            width: 640,
            height: 640,
            sender: self.chat_title(chat),
            date: "Profile photo".into(),
            caption: String::new(),
        })
    }

    pub(super) fn render_viewer(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let viewer = self.viewer.as_ref()?;
        let shown = if viewer.avatar {
            self.avatar_shown(viewer.chat_id, viewer.index)?
        } else {
            self.photo_shown(viewer.chat_id, viewer.message_id)?
        };
        let Shown {
            full_file,
            shown,
            width,
            height,
            sender,
            date,
            caption,
        } = shown;
        let loading = !full_file.local.is_downloading_completed;

        let position = self.viewer_position();
        let counter = position
            .filter(|(_, count)| *count > 1)
            .map(|(i, count)| format!("{} of {count}", i + 1));
        let has_previous = position.is_some_and(|(i, _)| i > 0);
        let has_next = position.is_some_and(|(i, count)| i + 1 < count);

        // Fit the photo inside the window, leaving room for the bars.
        let bounds = window.bounds().size;
        let (max_w, max_h) = (
            f32::from(bounds.width) - 160.,
            f32::from(bounds.height) - 150.,
        );
        let (w, h) = (width.max(1) as f32, height.max(1) as f32);
        let scale = (max_w / w).min(max_h / h).min(2.);
        let (w, h) = ((w * scale).round(), (h * scale).round());

        let path = full_file.local.path.clone();
        let complete = full_file.local.is_downloading_completed;

        let nav = |id: &'static str, icon: IconName, delta: isize, enabled: bool| {
            div()
                .id(id)
                .size(px(44.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui_kit::hsla(0., 0., 1., 0.08))
                // The backdrop closes on mouse down; keep it from seeing ours.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .when(enabled, |el| {
                    el.cursor_pointer()
                        .hover(|s| s.bg(gpui_kit::hsla(0., 0., 1., 0.16)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.step_viewer(delta);
                            cx.notify();
                        }))
                })
                .when(!enabled, |el| el.opacity(0.))
                .child(Icon::new(icon).size(px(22.)).text_color(gpui_kit::white()))
        };

        Some(
            div()
                .id("viewer")
                .absolute()
                .inset_0()
                .bg(gpui_kit::hsla(0., 0., 0., 0.92))
                .text_color(gpui_kit::white())
                .flex()
                .flex_col()
                // Blocks hover and clicks for everything underneath.
                .occlude()
                // Close on a full click (down and up on the backdrop): closing
                // on mouse down let the release land on whatever was behind.
                // Children stop their own mouse downs, so clicks on the photo
                // or controls don't count.
                .on_click(cx.listener(|this, _, _, cx| {
                    this.viewer = None;
                    cx.notify();
                }))
                // Top bar: who and when, then actions.
                .child(
                    div()
                        .h(px(56.))
                        .flex_shrink_0()
                        // Clear the traffic lights.
                        .pl(px(88.))
                        .pr(px(20.))
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .truncate()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(sender),
                                )
                                .child(
                                    div()
                                        .text_size(px(ui::TEXT_SM))
                                        .text_color(gpui_kit::hsla(0., 0., 1., 0.6))
                                        .child(date),
                                ),
                        )
                        .when_some(counter, |el, counter| {
                            el.child(
                                div()
                                    .text_size(px(ui::TEXT_SM))
                                    .text_color(gpui_kit::hsla(0., 0., 1., 0.6))
                                    .child(counter),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                // Clicks here belong to the buttons, not the backdrop.
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(
                                    widgets::icon_button("viewer-open", IconName::ExternalLink)
                                        .icon(
                                            Icon::new(IconName::ExternalLink)
                                                .text_color(gpui_kit::white()),
                                        )
                                        .tooltip("Open in Preview")
                                        .disabled(!complete)
                                        .on_click(move |_, _, cx| open_file(&path, cx)),
                                )
                                .child(
                                    widgets::icon_button("viewer-close", IconName::X)
                                        .icon(Icon::new(IconName::X).text_color(gpui_kit::white()))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.viewer = None;
                                            cx.notify();
                                        })),
                                ),
                        ),
                )
                // Photo, with previous/next pinned to the window's edges so
                // they stay put while paging through differently sized photos.
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .id("viewer-photo")
                                .relative()
                                .w(px(w))
                                .h(px(h))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .when_some(shown, |el, file| {
                                    el.child(
                                        img(PathBuf::from(&file.local.path))
                                            .w(px(w))
                                            .h(px(h))
                                            .rounded(px(ui::RADIUS_SM))
                                            .object_fit(ObjectFit::Contain),
                                    )
                                })
                                .when(loading, |el| {
                                    el.child(
                                        div()
                                            .absolute()
                                            .bottom(px(12.))
                                            .right(px(12.))
                                            .px(px(8.))
                                            .py(px(2.))
                                            .rounded_full()
                                            .bg(gpui_kit::hsla(0., 0., 0., 0.6))
                                            .text_size(px(ui::TEXT_SM))
                                            .child("Loading full size…"),
                                    )
                                }),
                        )
                        .child(
                            nav("viewer-prev", IconName::ChevronLeft, -1, has_previous)
                                .absolute()
                                .left(px(ui::GUTTER + 8.)),
                        )
                        .child(
                            nav("viewer-next", IconName::ChevronRight, 1, has_next)
                                .absolute()
                                .right(px(ui::GUTTER + 8.)),
                        ),
                )
                // Caption.
                .child(
                    div()
                        .h(px(70.))
                        .flex_shrink_0()
                        .px(px(80.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_center()
                        .when(!caption.is_empty(), |el| {
                            el.child(div().max_w(px(720.)).child(caption))
                        }),
                )
                .into_any_element(),
        )
    }
}
