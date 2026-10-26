//! Fills the app from `crate::demo` seed data and opens a scene.

use gpui_kit::{prelude::*, *};

use super::{Group, Login, Messenger};
use crate::{
    demo,
    model::{Chat, List},
    telegram::{Profile, SharedMedia},
};

impl Messenger {
    pub(super) fn load_demo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let seed = match demo::seed() {
            Ok(seed) => seed,
            Err(error) => {
                self.login = Login::Blocked;
                self.error = Some(format!("Demo seed: {error:#}"));
                return;
            }
        };
        self.me = seed.me;
        for user in seed.users {
            if let Some(photo) = &user.profile_photo {
                self.files.insert(photo.small.id, photo.small.clone());
            }
            self.users.insert(user.id, user);
        }
        for (id, members, channel) in seed.groups {
            self.groups.insert(
                id,
                Group {
                    members,
                    channel,
                    can_post: !channel,
                },
            );
        }
        self.folders = seed.folders;
        for (info, messages) in seed.chats {
            if let Some(photo) = &info.photo {
                self.files.insert(photo.small.id, photo.small.clone());
            }
            let id = info.id;
            let mut chat = Chat::new(info);
            chat.history_end = true;
            for message in messages {
                self.prepare_media(&message);
                chat.messages.insert(message.id, message);
            }
            // Repeat the chat's media so the grid shows a few full rows.
            let media: Vec<_> = chat
                .messages
                .values()
                .filter(|m| super::grid_file(m).is_some())
                .collect();
            let photos: Vec<_> = media
                .iter()
                .cycle()
                .take(if media.is_empty() { 0 } else { 8 })
                .enumerate()
                .map(|(index, message)| {
                    let mut message = (*message).clone();
                    message.id = 10_000 + index as i64;
                    message
                })
                .collect();
            self.shared_media.insert(
                id,
                SharedMedia {
                    photos: photos.len() as i32 + 23,
                    files: 4,
                    links: 9,
                    voice: 2,
                    recent: photos,
                    ..SharedMedia::default()
                },
            );
            self.chats.insert(id, chat);
        }
        for (id, members) in [(demo::MAYA, None), (demo::WEEKEND, Some(4))] {
            self.profiles.insert(
                id,
                Profile {
                    about: "Trail runner, coffee snob, weekend photographer.".into(),
                    members,
                },
            );
        }
        self.pinned.extend(seed.pinned);
        self.profile_photos.extend(seed.profile_photos);
        self.prepare_stickers(&seed.stickers, cx);
        self.stickers = seed.stickers;
        self.stickers_loaded = true;
        self.chats_end = true;
        self.connection = "Connected";
        self.login = Login::Ready;
        self.demo_scene(window, cx);
    }

    fn demo_scene(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let scene = demo::scene();
        let chat = match scene.as_str() {
            "list" | "archive" | "login" => None,
            "group" => Some(demo::WEEKEND),
            "channel" => Some(demo::TRAIL_NEWS),
            // A chat allowing many reactions: the menu's full list.
            "reactions" => Some(demo::ALEX),
            _ => Some(demo::MAYA),
        };
        if let Some(chat) = chat {
            self.select_chat(chat, window, cx);
        }
        match scene.as_str() {
            "details" => self.toggle_details(cx),
            "nostickers" => self.toggle_stickers(cx),
            "palette" => self.open_palette(window, cx),
            "search" => self.toggle_search(window, cx),
            // The composer with text and a reply, or a long multi-line draft.
            "compose" | "compose-long" => {
                let text = if scene == "compose" {
                    "Sounds great, see you at 7!"
                } else {
                    "Packing list so far:\n- headlamp\n- two liters of water\n- trail snacks\n- rain jacket, just in case"
                };
                if let Some(input) = self.composers.get(&demo::MAYA) {
                    input.update(cx, |input, cx| input.set_value(text, window, cx));
                }
                if scene == "compose" {
                    self.compose_actions
                        .insert(demo::MAYA, super::ComposeAction::Reply(16));
                }
            }
            "avatar" => self.open_avatar(demo::MAYA, cx),
            "viewer" => self.open_viewer(demo::MAYA, 13, cx),
            "playing" => self.toggle_play(demo::MAYA, 15, cx),
            "archive" => self.list = List::Archive,
            "login" => {
                self.login = Login::Phone;
                self.login_hint = "Enter your phone number with the country code.".into();
                self.login_input.update(cx, |input, cx| {
                    input.set_placeholder(super::login::placeholder(Login::Phone), window, cx)
                });
            }
            _ => {}
        }
    }
}
