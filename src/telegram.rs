use crate::model::List;
use anyhow::{Context, Result};
use async_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::PathBuf,
    thread,
};
use tdlib_rs::{
    enums::{self, Update},
    functions, types,
};

#[derive(Serialize, Deserialize)]
pub struct Credentials {
    pub api_id: i32,
    pub api_hash: String,
}

pub enum Command {
    SendPoll {
        chat_id: i64,
        topic: Option<i32>,
        question: String,
        options: Vec<String>,
        anonymous: bool,
        multiple: bool,
    },
    /// Recent and favorite stickers.
    Stickers,
    /// Installed stickers matching an emoji or keyword.
    SearchStickers {
        query: String,
        chat_id: i64,
    },
    SendSticker {
        chat_id: i64,
        topic: Option<i32>,
        file_id: i32,
        width: i32,
        height: i32,
        emoji: String,
    },
    Vote {
        chat_id: i64,
        message_id: i64,
        options: Vec<i32>,
    },
    Topics {
        chat_id: i64,
        offset: (i32, i64, i32),
    },
    TopicHistory {
        chat_id: i64,
        topic: i32,
        from: i64,
    },
    Configure(Credentials),
    Phone(String),
    Code(String),
    ResendCode,
    QrLogin,
    Password(String),
    Email(String),
    EmailCode(String),
    SendMessage {
        chat_id: i64,
        topic: Option<i32>,
        text: String,
        reply_to: Option<i64>,
        attachment: Option<PathBuf>,
    },
    Edit {
        chat_id: i64,
        message_id: i64,
        text: String,
        /// Media messages edit their caption instead of their text.
        caption: bool,
    },
    Forward {
        chat_id: i64,
        topic: Option<i32>,
        from: i64,
        message_id: i64,
    },
    Delete {
        chat_id: i64,
        message_id: i64,
    },
    Retry {
        chat_id: i64,
        message_id: i64,
    },
    React {
        chat_id: i64,
        message_id: i64,
        reaction: Reaction,
        remove: bool,
    },
    /// Stickers for custom emoji (reactions show them as images).
    CustomEmoji(Vec<i64>),
    SearchChats(String),
    SearchMessages {
        chat_id: i64,
        topic: Option<i32>,
        query: String,
    },
    Download {
        file: i32,
        /// 1–32; higher downloads first.
        priority: i32,
    },
    SharedMedia(i64),
    Profile {
        chat_id: i64,
        kind: enums::ChatType,
    },
    /// The emoji reactions the current user may add to a message.
    AvailableReactions {
        chat_id: i64,
        message_id: i64,
        /// Whether Premium-only reactions are usable.
        premium: bool,
    },
    /// A user's profile photos, or a group's past chat photos; newest first.
    ProfilePhotos {
        chat_id: i64,
        kind: enums::ChatType,
    },
    Read {
        chat_id: i64,
        message_id: i64,
    },
    Draft {
        chat_id: i64,
        topic: Option<i32>,
        text: String,
    },
    Archive {
        chat_id: i64,
        archive: bool,
    },
    Pin {
        chat_id: i64,
        list: List,
        pinned: bool,
    },
    Mute {
        chat_id: i64,
        settings: types::ChatNotificationSettings,
    },
    SavedMessages,
    Leave(i64),
    LoadChats(List),
    History {
        chat_id: i64,
        from: i64,
        page: Page,
        /// The chat's window generation, returned with the page.
        window: u32,
    },
    /// Pinned messages of a chat, newest first.
    Pinned(i64),
    PinMessage {
        chat_id: i64,
        message_id: i64,
        pin: bool,
    },
    OpenChat(i64),
    CloseChat(i64),
}
/// Which slice of history a `History` command loads relative to `from`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// `from` and older (or the newest page when `from` is 0).
    Older,
    /// Roughly half before and half after `from`.
    Around,
    /// `from` and newer.
    Newer,
}

impl Page {
    fn offset(self) -> i32 {
        match self {
            Page::Older => 0,
            Page::Around => -25,
            Page::Newer => -49,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Request {
    SendPoll,
    Features,
    Topics(i64),
    TopicHistory(i64, i32),
    Auth,
    Chats(List),
    History(i64),
    Chat,
    Send(i64),
    SearchChats,
    SearchMessages(i64),
    Download(i32),
    Profile(i64),
    ProfilePhotos(i64),
    AvailableReactions(i64, i64),
    CustomEmoji,
    SharedMedia(i64),
    Pinned(i64),
    Background,
}
pub enum Reply {
    Stickers(Vec<types::Sticker>),
    StickerSearch(String, Vec<types::Sticker>),
    Topics(types::ForumTopics),
    TopicHistory(Vec<types::Message>),
    Done,
    OpenChat(i64),
    ChatsLoaded(bool),
    /// Messages, with the `from` id the page was requested from.
    History(i64, Page, u32, Vec<types::Message>),
    Pinned(Vec<types::Message>),
    SearchChats(String, Vec<i64>),
    SearchMessages(String, Vec<types::Message>),
    File(types::File),
    Profile(Profile),
    ProfilePhotos(Vec<types::ChatPhoto>),
    /// Emoji in display order; empty when reacting isn't allowed.
    AvailableReactions(Vec<String>),
    CustomEmoji(Vec<types::Sticker>),
    SharedMedia(SharedMedia),
}

/// A reaction the user can add or remove.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Reaction {
    Emoji(String),
    Custom(i64),
}

#[derive(Default)]
pub struct Profile {
    pub about: String,
    pub members: Option<i32>,
}

#[derive(Default)]
pub struct SharedMedia {
    pub photos: i32,
    pub videos: i32,
    pub files: i32,
    pub links: i32,
    pub voice: i32,
    pub recent: Vec<types::Message>,
}
pub enum Event {
    Credentials(Result<Option<Credentials>, String>),
    Update(Box<Update>),
    Completed(Request, Result<Reply, String>),
}
pub struct Client {
    commands: Sender<Command>,
    closed: Receiver<()>,
}

impl Client {
    pub fn start() -> Result<(Self, Receiver<Event>)> {
        let id = tdlib_rs::create_client();
        let (events, updates) = async_channel::bounded(256);
        let (commands, requests) = async_channel::unbounded();
        let (finished, closed) = async_channel::bounded(1);
        let incoming = events.clone();
        let receiver = thread::Builder::new().name("telegram-updates".into()).spawn(move || {
            loop {
                let Some((update, client_id)) = tdlib_rs::receive() else { continue };
                if client_id != id { continue; }
                let closed = matches!(&update, Update::AuthorizationState(state) if matches!(state.authorization_state, enums::AuthorizationState::Closed));
                // Continue draining on shutdown so TDLib can flush its database.
                let _ = incoming.send_blocking(Event::Update(Box::new(update)));
                if closed { break; }
            }
        })?;
        thread::Builder::new()
            .name("telegram-requests".into())
            .spawn(move || {
                // Requests run concurrently: a slow download or search must not
                // hold up opening a chat. TDLib orders sends on its side.
                let executor = smol::LocalExecutor::new();
                smol::block_on(executor.run(async {
                    let directory = data_directory().map_err(|e| format!("{e:#}"));
                    let credentials = directory
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|dir| load_credentials(dir).map_err(|e| format!("{e:#}")));
                    let _ = events.send(Event::Credentials(credentials)).await;
                    // This initial request also starts the authorization update stream.
                    let _ = functions::set_log_verbosity_level(0, id).await;
                    while let Ok(command) = requests.recv().await {
                        let request = request_kind(&command);
                        let events = events.clone();
                        let directory = directory.clone();
                        executor
                            .spawn(async move {
                                let result = match directory {
                                    Ok(directory) => execute(command, id, &directory).await,
                                    Err(error) => Err(error),
                                };
                                let _ = events.send(Event::Completed(request, result)).await;
                            })
                            .detach();
                    }
                    let _ = functions::close(id).await;
                }));
                let _ = receiver.join();
                drop(finished);
            })?;
        Ok((Self { commands, closed }, updates))
    }
    pub fn send(&self, command: Command) -> Result<()> {
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                async_channel::TrySendError::Full(_) | async_channel::TrySendError::Closed(_) => {
                    anyhow::anyhow!("The connection has stopped. Restart the app.")
                }
            })
    }
    pub fn close(&self) -> Receiver<()> {
        self.commands.close();
        self.closed.clone()
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.commands.close();
    }
}

fn request_kind(command: &Command) -> Request {
    match command {
        Command::SendPoll { .. } => Request::SendPoll,
        Command::Stickers
        | Command::SearchStickers { .. }
        | Command::SendSticker { .. }
        | Command::Vote { .. } => Request::Features,
        Command::Topics { chat_id, .. } => Request::Topics(*chat_id),
        Command::TopicHistory { chat_id, topic, .. } => Request::TopicHistory(*chat_id, *topic),
        Command::SendMessage { chat_id, .. }
        | Command::Edit { chat_id, .. }
        | Command::Forward { chat_id, .. } => Request::Send(*chat_id),
        Command::LoadChats(list) => Request::Chats(*list),
        Command::SearchChats(_) => Request::SearchChats,
        Command::SearchMessages { chat_id, .. } => Request::SearchMessages(*chat_id),
        Command::Download { file, .. } => Request::Download(*file),
        Command::Profile { chat_id, .. } => Request::Profile(*chat_id),
        Command::ProfilePhotos { chat_id, .. } => Request::ProfilePhotos(*chat_id),
        Command::AvailableReactions {
            chat_id,
            message_id,
            ..
        } => Request::AvailableReactions(*chat_id, *message_id),
        Command::CustomEmoji(_) => Request::CustomEmoji,
        Command::SharedMedia(chat_id) => Request::SharedMedia(*chat_id),
        Command::Read { .. } | Command::Draft { .. } => Request::Background,
        Command::Archive { .. }
        | Command::Pin { .. }
        | Command::Mute { .. }
        | Command::Delete { .. }
        | Command::Retry { .. }
        | Command::React { .. }
        | Command::OpenChat(_)
        | Command::CloseChat(_)
        | Command::Leave(_)
        | Command::SavedMessages => Request::Chat,
        Command::History { chat_id, .. } => Request::History(*chat_id),
        Command::Pinned(chat_id) => Request::Pinned(*chat_id),
        Command::PinMessage { .. } => Request::Chat,
        Command::Configure(_)
        | Command::Phone(_)
        | Command::Code(_)
        | Command::ResendCode
        | Command::QrLogin
        | Command::Password(_)
        | Command::Email(_)
        | Command::EmailCode(_) => Request::Auth,
    }
}

fn data_directory() -> Result<PathBuf> {
    let directory = crate::paths::data_dir().context("HOME is not set")?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)?;
    Ok(directory)
}
fn load_credentials(directory: &std::path::Path) -> Result<Option<Credentials>> {
    match fs::read(directory.join("application.json")) {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).context("Cannot read saved application credentials")?,
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
fn save_credentials(directory: &std::path::Path, credentials: &Credentials) -> Result<()> {
    let temporary = directory.join("application.json.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec(credentials)?)?;
    file.sync_all()?;
    fs::rename(temporary, directory.join("application.json"))?;
    Ok(())
}
async fn execute(command: Command, id: i32, directory: &std::path::Path) -> Result<Reply, String> {
    let result = match command {
        Command::SendPoll {
            chat_id,
            topic,
            question,
            options,
            anonymous,
            multiple,
        } => {
            let content = enums::InputMessageContent::InputMessagePoll(types::InputMessagePoll {
                question: types::FormattedText {
                    text: question,
                    entities: Vec::new(),
                },
                options: options
                    .into_iter()
                    .map(|text| types::FormattedText {
                        text,
                        entities: Vec::new(),
                    })
                    .collect(),
                is_anonymous: anonymous,
                r#type: enums::PollType::Regular(types::PollTypeRegular {
                    allow_multiple_answers: multiple,
                }),
            });
            functions::send_message(
                chat_id,
                topic.map(|forum_topic_id| {
                    enums::MessageTopic::Forum(types::MessageTopicForum { forum_topic_id })
                }),
                None,
                None,
                content,
                id,
            )
            .await
            .map_err(|e| e.message)?;
            return Ok(Reply::Done);
        }
        Command::Stickers => {
            let enums::Stickers::Stickers(mut recent) = functions::get_recent_stickers(false, id)
                .await
                .map_err(|e| e.message)?;
            let enums::Stickers::Stickers(favorites) = functions::get_favorite_stickers(id)
                .await
                .map_err(|e| e.message)?;
            for sticker in favorites.stickers {
                if !recent.stickers.iter().any(|s| s.id == sticker.id) {
                    recent.stickers.push(sticker);
                }
            }
            return Ok(Reply::Stickers(recent.stickers));
        }
        Command::SearchStickers { query, chat_id } => {
            let enums::Stickers::Stickers(found) = functions::get_stickers(
                enums::StickerType::Regular,
                query.clone(),
                50,
                chat_id,
                id,
            )
            .await
            .map_err(|e| e.message)?;
            return Ok(Reply::StickerSearch(query, found.stickers));
        }
        Command::SendSticker {
            chat_id,
            topic,
            file_id,
            width,
            height,
            emoji,
        } => {
            let content =
                enums::InputMessageContent::InputMessageSticker(types::InputMessageSticker {
                    sticker: enums::InputFile::Id(types::InputFileId { id: file_id }),
                    thumbnail: None,
                    width,
                    height,
                    emoji,
                });
            functions::send_message(
                chat_id,
                topic.map(|forum_topic_id| {
                    enums::MessageTopic::Forum(types::MessageTopicForum { forum_topic_id })
                }),
                None,
                None,
                content,
                id,
            )
            .await
            .map_err(|e| e.message)?;
            return Ok(Reply::Done);
        }
        Command::Vote {
            chat_id,
            message_id,
            options,
        } => functions::set_poll_answer(chat_id, message_id, options, id).await,
        Command::Topics { chat_id, offset } => {
            let enums::ForumTopics::ForumTopics(topics) = functions::get_forum_topics(
                chat_id,
                String::new(),
                offset.0,
                offset.1,
                offset.2,
                100,
                id,
            )
            .await
            .map_err(|e| e.message)?;
            return Ok(Reply::Topics(topics));
        }
        Command::TopicHistory {
            chat_id,
            topic,
            from,
        } => {
            let enums::Messages::Messages(messages) =
                functions::get_forum_topic_history(chat_id, topic, from, 0, 50, id)
                    .await
                    .map_err(|e| e.message)?;
            return Ok(Reply::TopicHistory(
                messages.messages.into_iter().flatten().collect(),
            ));
        }
        Command::Configure(credentials) => {
            functions::set_tdlib_parameters(
                false,
                directory.join("database").to_string_lossy().into_owned(),
                directory.join("files").to_string_lossy().into_owned(),
                String::new(),
                true,
                true,
                true,
                false,
                credentials.api_id,
                credentials.api_hash.clone(),
                "en".into(),
                "Mac".into(),
                String::new(),
                env!("CARGO_PKG_VERSION").into(),
                id,
            )
            .await
            .map_err(|error| error.message)?;
            save_credentials(directory, &credentials)
                .map_err(|error| format!("Could not save application settings: {error}"))?;
            return Ok(Reply::Done);
        }
        Command::Phone(value) => functions::set_authentication_phone_number(value, None, id).await,
        Command::Code(value) => functions::check_authentication_code(value, id).await,
        Command::QrLogin => functions::request_qr_code_authentication(Vec::new(), id).await,
        Command::ResendCode => functions::resend_authentication_code(None, id).await,
        Command::Password(value) => functions::check_authentication_password(value, id).await,
        Command::Email(value) => functions::set_authentication_email_address(value, id).await,
        Command::EmailCode(code) => {
            functions::check_authentication_email_code(
                enums::EmailAddressAuthentication::Code(types::EmailAddressAuthenticationCode {
                    code,
                }),
                id,
            )
            .await
        }
        Command::OpenChat(chat) => functions::open_chat(chat, id).await,
        Command::CloseChat(chat) => functions::close_chat(chat, id).await,
        Command::Leave(chat) => functions::leave_chat(chat, id).await,
        Command::SendMessage {
            chat_id,
            topic,
            text,
            reply_to,
            attachment,
        } => {
            let content = if let Some(path) = attachment {
                let photo = path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                    ["jpg", "jpeg", "png"].contains(&e.to_ascii_lowercase().as_str())
                });
                let file = enums::InputFile::Local(types::InputFileLocal {
                    path: path.to_string_lossy().into_owned(),
                });
                let caption = Some(markdown(text, id).await);
                if photo {
                    enums::InputMessageContent::InputMessagePhoto(types::InputMessagePhoto {
                        photo: file,
                        thumbnail: None,
                        added_sticker_file_ids: Vec::new(),
                        width: 0,
                        height: 0,
                        caption,
                        show_caption_above_media: false,
                        self_destruct_type: None,
                        has_spoiler: false,
                    })
                } else {
                    enums::InputMessageContent::InputMessageDocument(types::InputMessageDocument {
                        document: file,
                        thumbnail: None,
                        disable_content_type_detection: false,
                        caption,
                    })
                }
            } else {
                formatted_content(markdown(text, id).await)
            };
            let reply_to = reply_to.map(|message_id| {
                enums::InputMessageReplyTo::Message(types::InputMessageReplyToMessage {
                    message_id,
                    quote: None,
                    checklist_task_id: 0,
                })
            });
            functions::send_message(
                chat_id,
                topic.map(|forum_topic_id| {
                    enums::MessageTopic::Forum(types::MessageTopicForum { forum_topic_id })
                }),
                reply_to,
                None,
                content,
                id,
            )
            .await
            .map_err(|error| error.message)?;
            return Ok(Reply::Done);
        }
        Command::Edit {
            chat_id,
            message_id,
            text,
            caption,
        } => {
            let text = markdown(text, id).await;
            if caption {
                functions::edit_message_caption(chat_id, message_id, Some(text), false, id)
                    .await
                    .map_err(|e| e.message)?;
            } else {
                functions::edit_message_text(chat_id, message_id, formatted_content(text), id)
                    .await
                    .map_err(|e| e.message)?;
            }
            return Ok(Reply::Done);
        }
        Command::Forward {
            chat_id,
            topic,
            from,
            message_id,
        } => {
            functions::forward_messages(
                chat_id,
                topic.map(|forum_topic_id| {
                    enums::MessageTopic::Forum(types::MessageTopicForum { forum_topic_id })
                }),
                from,
                vec![message_id],
                None,
                false,
                false,
                id,
            )
            .await
            .map_err(|e| e.message)?;
            return Ok(Reply::Done);
        }
        Command::Delete {
            chat_id,
            message_id,
        } => functions::delete_messages(chat_id, vec![message_id], true, id).await,
        Command::Retry {
            chat_id,
            message_id,
        } => {
            functions::resend_messages(chat_id, vec![message_id], None, 0, id)
                .await
                .map_err(|e| e.message)?;
            return Ok(Reply::Done);
        }
        Command::React {
            chat_id,
            message_id,
            reaction,
            remove,
        } => {
            let reaction = match reaction {
                Reaction::Emoji(emoji) => {
                    enums::ReactionType::Emoji(types::ReactionTypeEmoji { emoji })
                }
                Reaction::Custom(custom_emoji_id) => {
                    enums::ReactionType::CustomEmoji(types::ReactionTypeCustomEmoji {
                        custom_emoji_id,
                    })
                }
            };
            if remove {
                functions::remove_message_reaction(chat_id, message_id, reaction, id).await
            } else {
                functions::add_message_reaction(chat_id, message_id, reaction, false, true, id)
                    .await
            }
        }
        Command::SharedMedia(chat_id) => {
            let count = async |filter| match functions::get_chat_message_count(
                chat_id, None, filter, false, id,
            )
            .await
            {
                Ok(enums::Count::Count(count)) => count.count,
                Err(_) => 0,
            };
            let enums::FoundChatMessages::FoundChatMessages(found) =
                functions::search_chat_messages(
                    chat_id,
                    None,
                    String::new(),
                    None,
                    0,
                    0,
                    9,
                    Some(enums::SearchMessagesFilter::PhotoAndVideo),
                    id,
                )
                .await
                .map_err(|e| e.message)?;
            return Ok(Reply::SharedMedia(SharedMedia {
                photos: count(enums::SearchMessagesFilter::Photo).await,
                videos: count(enums::SearchMessagesFilter::Video).await,
                files: count(enums::SearchMessagesFilter::Document).await,
                links: count(enums::SearchMessagesFilter::Url).await,
                voice: count(enums::SearchMessagesFilter::VoiceNote).await,
                recent: found.messages,
            }));
        }
        Command::Download { file, priority } => {
            let enums::File::File(file) = functions::download_file(file, priority, 0, 0, false, id)
                .await
                .map_err(|e| e.message)?;
            return Ok(Reply::File(file));
        }
        Command::SearchChats(query) => {
            let enums::Chats::Chats(chats) = functions::search_chats(query.clone(), 100, id)
                .await
                .map_err(|e| e.message)?;
            return Ok(Reply::SearchChats(query, chats.chat_ids));
        }
        Command::SearchMessages {
            chat_id,
            topic,
            query,
        } => {
            let enums::FoundChatMessages::FoundChatMessages(found) =
                functions::search_chat_messages(
                    chat_id,
                    topic.map(|forum_topic_id| {
                        enums::MessageTopic::Forum(types::MessageTopicForum { forum_topic_id })
                    }),
                    query.clone(),
                    None,
                    0,
                    0,
                    100,
                    None,
                    id,
                )
                .await
                .map_err(|e| e.message)?;
            return Ok(Reply::SearchMessages(query, found.messages));
        }
        Command::Profile { chat_id: _, kind } => {
            let profile = match kind {
                enums::ChatType::Private(user) => {
                    let enums::UserFullInfo::UserFullInfo(info) =
                        functions::get_user_full_info(user.user_id, id)
                            .await
                            .map_err(|e| e.message)?;
                    Profile {
                        about: info.bio.map_or_else(String::new, |bio| bio.text),
                        members: None,
                    }
                }
                enums::ChatType::Supergroup(group) => {
                    let enums::SupergroupFullInfo::SupergroupFullInfo(info) =
                        functions::get_supergroup_full_info(group.supergroup_id, id)
                            .await
                            .map_err(|e| e.message)?;
                    Profile {
                        about: info.description,
                        members: Some(info.member_count),
                    }
                }
                enums::ChatType::BasicGroup(group) => {
                    let enums::BasicGroupFullInfo::BasicGroupFullInfo(info) =
                        functions::get_basic_group_full_info(group.basic_group_id, id)
                            .await
                            .map_err(|e| e.message)?;
                    Profile {
                        about: info.description,
                        members: Some(info.members.len() as i32),
                    }
                }
                enums::ChatType::Secret(_) => Profile::default(),
            };
            return Ok(Reply::Profile(profile));
        }
        Command::AvailableReactions {
            chat_id,
            message_id,
            premium,
        } => {
            let enums::AvailableReactions::AvailableReactions(available) =
                functions::get_message_available_reactions(chat_id, message_id, 7, id)
                    .await
                    .map_err(|e| e.message)?;
            if available.unavailability_reason.is_some() {
                return Ok(Reply::AvailableReactions(Vec::new()));
            }
            let mut emoji: Vec<String> = Vec::new();
            for reaction in available
                .top_reactions
                .into_iter()
                .chain(available.recent_reactions)
                .chain(available.popular_reactions)
                .filter(|r| premium || !r.needs_premium)
            {
                if let enums::ReactionType::Emoji(e) = reaction.r#type
                    && !emoji.contains(&e.emoji)
                {
                    emoji.push(e.emoji);
                }
            }
            return Ok(Reply::AvailableReactions(emoji));
        }
        Command::CustomEmoji(ids) => {
            let enums::Stickers::Stickers(found) = functions::get_custom_emoji_stickers(ids, id)
                .await
                .map_err(|e| e.message)?;
            return Ok(Reply::CustomEmoji(found.stickers));
        }
        Command::ProfilePhotos { chat_id, kind } => {
            let photos = match kind {
                enums::ChatType::Private(user) => {
                    let enums::ChatPhotos::ChatPhotos(found) =
                        functions::get_user_profile_photos(user.user_id, 0, 100, id)
                            .await
                            .map_err(|e| e.message)?;
                    found.photos
                }
                _ => {
                    let enums::FoundChatMessages::FoundChatMessages(found) =
                        functions::search_chat_messages(
                            chat_id,
                            None,
                            String::new(),
                            None,
                            0,
                            0,
                            100,
                            Some(enums::SearchMessagesFilter::ChatPhoto),
                            id,
                        )
                        .await
                        .map_err(|e| e.message)?;
                    found
                        .messages
                        .into_iter()
                        .filter_map(|message| match message.content {
                            enums::MessageContent::MessageChatChangePhoto(change) => {
                                Some(change.photo)
                            }
                            _ => None,
                        })
                        .collect()
                }
            };
            return Ok(Reply::ProfilePhotos(photos));
        }
        Command::Read {
            chat_id,
            message_id,
        } => {
            functions::view_messages(
                chat_id,
                vec![message_id],
                Some(enums::MessageSource::ChatHistory),
                true,
                id,
            )
            .await
        }
        Command::Draft {
            chat_id,
            topic,
            text,
        } => {
            let draft = if text.is_empty() {
                None
            } else {
                Some(types::DraftMessage {
                    reply_to: None,
                    date: 0,
                    input_message_text: text_content(text),
                    effect_id: 0,
                    suggested_post_info: None,
                })
            };
            functions::set_chat_draft_message(
                chat_id,
                topic.map(|forum_topic_id| {
                    enums::MessageTopic::Forum(types::MessageTopicForum { forum_topic_id })
                }),
                draft,
                id,
            )
            .await
        }
        Command::Archive { chat_id, archive } => {
            functions::add_chat_to_list(
                chat_id,
                if archive {
                    enums::ChatList::Archive
                } else {
                    enums::ChatList::Main
                },
                id,
            )
            .await
        }
        Command::Pin {
            chat_id,
            list,
            pinned,
        } => functions::toggle_chat_is_pinned(list.td(), chat_id, pinned, id).await,
        Command::Mute { chat_id, settings } => {
            functions::set_chat_notification_settings(chat_id, settings, id).await
        }
        Command::SavedMessages => {
            let enums::User::User(user) = functions::get_me(id).await.map_err(|e| e.message)?;
            let enums::Chat::Chat(chat) = functions::create_private_chat(user.id, false, id)
                .await
                .map_err(|e| e.message)?;
            return Ok(Reply::OpenChat(chat.id));
        }
        Command::LoadChats(list) => {
            return match functions::load_chats(Some(list.td()), 50, id).await {
                Ok(()) => Ok(Reply::ChatsLoaded(false)),
                Err(error) if error.code == 404 => Ok(Reply::ChatsLoaded(true)),
                Err(error) => Err(error.message),
            };
        }
        Command::History {
            chat_id,
            from,
            page,
            window,
        } => {
            let enums::Messages::Messages(messages) =
                functions::get_chat_history(chat_id, from, page.offset(), 50, false, id)
                    .await
                    .map_err(|error| error.message)?;
            return Ok(Reply::History(
                from,
                page,
                window,
                messages.messages.into_iter().flatten().collect(),
            ));
        }
        Command::Pinned(chat_id) => {
            let enums::FoundChatMessages::FoundChatMessages(found) =
                functions::search_chat_messages(
                    chat_id,
                    None,
                    String::new(),
                    None,
                    0,
                    0,
                    50,
                    Some(enums::SearchMessagesFilter::Pinned),
                    id,
                )
                .await
                .map_err(|e| e.message)?;
            return Ok(Reply::Pinned(found.messages));
        }
        Command::PinMessage {
            chat_id,
            message_id,
            pin,
        } => {
            if pin {
                functions::pin_chat_message(chat_id, message_id, true, false, id).await
            } else {
                functions::unpin_chat_message(chat_id, message_id, id).await
            }
        }
    };
    result.map(|()| Reply::Done).map_err(|error| error.message)
}

fn text_content(text: String) -> enums::InputMessageContent {
    formatted_content(types::FormattedText {
        text,
        entities: Vec::new(),
    })
}

fn formatted_content(text: types::FormattedText) -> enums::InputMessageContent {
    enums::InputMessageContent::InputMessageText(types::InputMessageText {
        text,
        link_preview_options: None,
        clear_draft: true,
    })
}

/// Parses Telegram markdown: **bold**, __italic__, ~~strike~~, ||spoiler||,
/// `code`, ```pre``` and [text](url). Falls back to plain text.
async fn markdown(text: String, id: i32) -> types::FormattedText {
    let plain = types::FormattedText {
        text,
        entities: Vec::new(),
    };
    match functions::parse_markdown(plain.clone(), id).await {
        Ok(enums::FormattedText::FormattedText(parsed)) => parsed,
        Err(_) => plain,
    }
}
