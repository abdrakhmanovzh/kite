use std::collections::BTreeMap;
use tdlib_rs::{
    enums::{ChatList, MessageContent},
    types::{Chat as TelegramChat, ChatPosition, Message},
};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum List {
    Main,
    Archive,
    Folder(i32),
}
impl List {
    pub fn td(self) -> ChatList {
        match self {
            Self::Main => ChatList::Main,
            Self::Archive => ChatList::Archive,
            Self::Folder(id) => {
                ChatList::Folder(tdlib_rs::types::ChatListFolder { chat_folder_id: id })
            }
        }
    }
}

pub struct Chat {
    pub info: TelegramChat,
    pub messages: BTreeMap<i64, Message>,
    pub history_loading: bool,
    pub history_end: bool,
    /// The loaded window ends before the chat's latest message (after a
    /// jump); newer pages load on scroll and live messages wait until then.
    pub has_newer: bool,
    /// Generation of the loaded window. History pages carry the generation
    /// they were requested for, so a page arriving after the window was
    /// replaced (a jump) is dropped instead of mixed in.
    pub window: u32,
}
impl Chat {
    pub fn new(info: TelegramChat) -> Self {
        Self {
            info,
            messages: BTreeMap::new(),
            history_loading: false,
            history_end: false,
            has_newer: false,
            window: 0,
        }
    }

    /// Drops the loaded messages to start a new window; pages still in
    /// flight for the old one will be ignored.
    pub fn reset_window(&mut self) {
        self.messages.clear();
        self.has_newer = false;
        self.history_end = false;
        self.history_loading = false;
        self.window = self.window.wrapping_add(1);
    }
    pub fn order(&self, list: List) -> i64 {
        self.info
            .positions
            .iter()
            .find(|p| p.list == list.td())
            .map_or(0, |p| p.order)
    }
    pub fn position(&mut self, position: ChatPosition) {
        self.info.positions.retain(|p| p.list != position.list);
        if position.order != 0 {
            self.info.positions.push(position);
        }
    }
}
pub fn message_text(content: &MessageContent) -> String {
    match content {
        MessageContent::MessageText(message) => message.text.text.clone(),
        MessageContent::MessagePhoto(message) => caption("Photo", &message.caption.text),
        MessageContent::MessageVideo(message) => caption("Video", &message.caption.text),
        MessageContent::MessageDocument(message) => {
            caption(&message.document.file_name, &message.caption.text)
        }
        MessageContent::MessageAnimation(message) => caption("GIF", &message.caption.text),
        MessageContent::MessageVoiceNote(message) => {
            caption("Voice message", &message.caption.text)
        }
        MessageContent::MessageSticker(message) => format!("{} Sticker", message.sticker.emoji),
        MessageContent::MessageAudio(m) => caption(
            &format!("♫ {} · {}", m.audio.performer, m.audio.title),
            &m.caption.text,
        ),
        MessageContent::MessageVideoNote(m) => {
            format!("Video message · {}s", m.video_note.duration)
        }
        MessageContent::MessageContact(m) => format!(
            "{} {}\n+{}",
            m.contact.first_name, m.contact.last_name, m.contact.phone_number
        ),
        MessageContent::MessageLocation(m) => format!(
            "Location · {:.5}, {:.5}",
            m.location.latitude, m.location.longitude
        ),
        MessageContent::MessagePoll(m) => format!(
            "{}\n{}\n{} votes",
            m.poll.question.text,
            m.poll
                .options
                .iter()
                .map(|o| format!("{} · {}%", o.text.text, o.vote_percentage))
                .collect::<Vec<_>>()
                .join("\n"),
            m.poll.total_voter_count
        ),
        MessageContent::MessageCall(m) => format!(
            "{} call · {}s",
            if m.is_video { "Video" } else { "Voice" },
            m.duration
        ),
        MessageContent::MessageChatChangeTitle(m) => format!("Chat renamed to {}", m.title),
        MessageContent::MessageChatAddMembers(m) => {
            format!("{} member(s) added", m.member_user_ids.len())
        }
        MessageContent::MessageChatDeleteMember(_) => "A member left the chat".into(),
        MessageContent::MessageChatJoinByLink => "Joined via invite link".into(),
        MessageContent::MessagePinMessage(_) => "A message was pinned".into(),
        _ => "Service or unsupported message".into(),
    }
}
fn caption(kind: &str, text: &str) -> String {
    if text.is_empty() {
        kind.into()
    } else {
        format!("{kind}\n{text}")
    }
}

pub fn media(content: &MessageContent) -> Option<(&tdlib_rs::types::File, bool)> {
    match content {
        MessageContent::MessagePhoto(m) => photo_size(&m.photo).map(|s| (&s.photo, true)),
        MessageContent::MessageSticker(m) => sticker_file(&m.sticker).map(|f| (f, true)),
        MessageContent::MessageDocument(m) => Some((&m.document.document, false)),
        MessageContent::MessageVideo(m) => Some((&m.video.video, false)),
        MessageContent::MessageAnimation(m) => Some((&m.animation.animation, false)),
        MessageContent::MessageVoiceNote(m) => Some((&m.voice_note.voice, false)),
        MessageContent::MessageAudio(m) => Some((&m.audio.audio, false)),
        MessageContent::MessageVideoNote(m) => Some((&m.video_note.video, false)),
        _ => None,
    }
}

/// Events rendered as centered service pills rather than bubbles.
pub fn is_service(content: &MessageContent) -> bool {
    matches!(
        content,
        MessageContent::MessageChatChangeTitle(_)
            | MessageContent::MessageChatAddMembers(_)
            | MessageContent::MessageChatDeleteMember(_)
            | MessageContent::MessageChatJoinByLink
            | MessageContent::MessageChatJoinByRequest
            | MessageContent::MessagePinMessage(_)
            | MessageContent::MessageChatChangePhoto(_)
            | MessageContent::MessageChatDeletePhoto
            | MessageContent::MessageBasicGroupChatCreate(_)
            | MessageContent::MessageSupergroupChatCreate(_)
            | MessageContent::MessageScreenshotTaken
            | MessageContent::MessageContactRegistered
            | MessageContent::MessageForumTopicCreated(_)
            | MessageContent::MessageForumTopicEdited(_)
            | MessageContent::MessageChatUpgradeTo(_)
            | MessageContent::MessageChatUpgradeFrom(_)
            | MessageContent::MessageChatSetMessageAutoDeleteTime(_)
    )
}

/// Service text without the actor; callers prefix the sender's name.
pub fn service_action(content: &MessageContent) -> String {
    match content {
        MessageContent::MessageChatChangeTitle(m) => {
            format!("changed the group name to “{}”", m.title)
        }
        MessageContent::MessageChatAddMembers(_) => "joined the group".into(),
        MessageContent::MessageChatDeleteMember(_) => "left the group".into(),
        MessageContent::MessageChatJoinByLink => "joined the group via invite link".into(),
        MessageContent::MessageChatJoinByRequest => "was accepted to the group".into(),
        MessageContent::MessagePinMessage(_) => "pinned a message".into(),
        MessageContent::MessageChatChangePhoto(_) => "updated the group photo".into(),
        MessageContent::MessageChatDeletePhoto => "removed the group photo".into(),
        MessageContent::MessageBasicGroupChatCreate(_)
        | MessageContent::MessageSupergroupChatCreate(_) => "created the group".into(),
        MessageContent::MessageScreenshotTaken => "took a screenshot".into(),
        MessageContent::MessageContactRegistered => "joined Telegram".into(),
        MessageContent::MessageForumTopicCreated(m) => format!("created the topic “{}”", m.name),
        MessageContent::MessageForumTopicEdited(_) => "edited the topic".into(),
        MessageContent::MessageChatUpgradeTo(_) | MessageContent::MessageChatUpgradeFrom(_) => {
            "upgraded the group".into()
        }
        MessageContent::MessageChatSetMessageAutoDeleteTime(_) => {
            "changed the auto-delete timer".into()
        }
        _ => String::new(),
    }
}

/// One-line chat-list preview. The flag marks a media kind with no caption,
/// which the list tints like Telegram does.
pub fn preview(content: &MessageContent) -> (String, bool) {
    let (text, kind) = preview_raw(content);
    // One line: collapse line breaks and runs of spaces.
    (text.split_whitespace().collect::<Vec<_>>().join(" "), kind)
}

fn preview_raw(content: &MessageContent) -> (String, bool) {
    let with_caption = |kind: &str, caption: &str| {
        if caption.is_empty() {
            (kind.to_owned(), true)
        } else {
            (caption.replace('\n', " "), false)
        }
    };
    match content {
        MessageContent::MessageText(m) => (m.text.text.replace('\n', " "), false),
        MessageContent::MessagePhoto(m) => with_caption("Photo", &m.caption.text),
        MessageContent::MessageVideo(m) => with_caption("Video", &m.caption.text),
        MessageContent::MessageAnimation(m) => with_caption("GIF", &m.caption.text),
        MessageContent::MessageDocument(m) => with_caption(&m.document.file_name, &m.caption.text),
        MessageContent::MessageAudio(m) => with_caption(
            &if m.audio.title.is_empty() {
                m.audio.file_name.clone()
            } else {
                format!("{} – {}", m.audio.performer, m.audio.title)
            },
            &m.caption.text,
        ),
        MessageContent::MessageVoiceNote(m) => with_caption("Voice message", &m.caption.text),
        MessageContent::MessageVideoNote(_) => ("Video message".into(), true),
        MessageContent::MessageSticker(m) => (format!("{} Sticker", m.sticker.emoji), true),
        MessageContent::MessagePoll(m) => (format!("📊 {}", m.poll.question.text), false),
        MessageContent::MessageContact(_) => ("Contact".into(), true),
        MessageContent::MessageLocation(_) | MessageContent::MessageVenue(_) => {
            ("Location".into(), true)
        }
        MessageContent::MessageCall(m) => {
            (if m.is_video { "Video call" } else { "Call" }.into(), true)
        }
        other if is_service(other) => (service_action(other), false),
        other => (message_text(other).replace('\n', " "), false),
    }
}

/// The largest photo size suitable for display.
/// The size shown in a chat bubble: the smallest that still covers the
/// bubble's display size (at most 400×420pt, see `history::fit`) at 2x for
/// Retina. Bigger sizes only cost memory and bandwidth; the viewer loads
/// the largest separately.
pub fn photo_size(photo: &tdlib_rs::types::Photo) -> Option<&tdlib_rs::types::PhotoSize> {
    let largest = photo.sizes.iter().max_by_key(|s| s.width)?;
    let (w, h) = (largest.width.max(1) as f32, largest.height.max(1) as f32);
    let shown = (w * (400. / w).min(420. / h)).max(160.);
    let needed = (shown * 2.).round() as i32;
    photo
        .sizes
        .iter()
        .filter(|s| s.width >= needed && s.width <= 1280)
        .min_by_key(|s| s.width)
        .or_else(|| {
            photo
                .sizes
                .iter()
                .filter(|s| s.width <= 1280)
                .max_by_key(|s| s.width)
        })
        .or_else(|| photo.sizes.first())
}

/// A small photo size for grid thumbnails.
pub fn photo_thumb(photo: &tdlib_rs::types::Photo) -> Option<&tdlib_rs::types::PhotoSize> {
    photo
        .sizes
        .iter()
        .filter(|s| s.width >= 200)
        .min_by_key(|s| s.width)
        .or_else(|| photo.sizes.last())
}

/// A file GPUI can draw for a sticker: the WebP itself, or a static
/// thumbnail for animated (TGS) and video (WebM) stickers. None when only
/// animated formats exist; callers show the emoji instead.
pub fn sticker_file(sticker: &tdlib_rs::types::Sticker) -> Option<&tdlib_rs::types::File> {
    use tdlib_rs::enums::{StickerFormat, ThumbnailFormat};
    if matches!(sticker.format, StickerFormat::Webp) {
        return Some(&sticker.sticker);
    }
    sticker
        .thumbnail
        .as_ref()
        .filter(|t| {
            matches!(
                t.format,
                ThumbnailFormat::Webp
                    | ThumbnailFormat::Png
                    | ThumbnailFormat::Jpeg
                    | ThumbnailFormat::Gif
            )
        })
        .map(|t| &t.file)
}
