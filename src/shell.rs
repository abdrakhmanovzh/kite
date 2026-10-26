mod conversation;
mod demo;
mod details;
mod history;
mod login;
mod menu;
mod palette;
mod pane;
mod playback;
mod selectable;
mod sidebar;
mod viewer;
mod widgets;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui_kit::{
    component::{
        WindowExt,
        input::{InputEvent, InputState, TextareaState},
        message_scroller::MessageScrollerState,
        notification::Notification,
    },
    prelude::*,
    *,
};
use tdlib_rs::{
    enums::{AuthenticationCodeType, AuthorizationState, ConnectionState, Update},
    types::Message,
};

use crate::{
    model::{Chat, List, media, message_text},
    telegram::{Client, Command, Credentials, Event, Page, Profile, Reply, Request, SharedMedia},
    ui,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Login {
    Waiting,
    Application,
    Phone,
    Code,
    Qr,
    Password,
    Email,
    EmailCode,
    Ready,
    Blocked,
}

#[derive(Clone)]
enum ComposeAction {
    Reply(i64),
    Edit { id: i64, previous: String },
    Forward { from: i64, id: i64 },
}

struct Group {
    members: i32,
    channel: bool,
    /// False for channels where we may only read.
    can_post: bool,
}

/// A decoded animated or video sticker.
enum Anim {
    Loading,
    Ready(crate::stickers::Frames),
    Failed,
}

/// An animation cache entry; `last_used` drives least-recently-used eviction.
struct CachedAnim {
    anim: Anim,
    last_used: std::cell::Cell<u64>,
}

/// Messages a chat keeps once it's no longer open.
const HISTORY_KEEP: usize = 60;

/// How many recent stickers the picker keeps.
const STICKER_HISTORY: usize = 50;

fn sticker_history_path() -> Option<PathBuf> {
    Some(crate::paths::data_dir()?.join("recent_stickers.json"))
}

fn load_sticker_history() -> Vec<tdlib_rs::types::Sticker> {
    sticker_history_path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_sticker_history(stickers: &[tdlib_rs::types::Sticker]) -> anyhow::Result<()> {
    let path = sticker_history_path().ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec(stickers)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

/// Decoded animations kept, per kind: full chat animations hold up to ~90
/// 240px frames (~20 MB) each; picker first frames are one 128px frame.
const MAX_ANIMATIONS: usize = 24;
const MAX_FIRST_FRAMES: usize = 160;

/// One entry of the virtualized history.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    Loading,
    /// "Unread messages" divider before the first unread incoming message.
    Unread,
    /// Local calendar day, as days since the common era.
    Day(i32),
    Message(i64),
}

/// What the history rows were built for; a change resets the scroller.
#[derive(Clone, Copy, PartialEq, Eq)]
struct RowsKey {
    chat: i64,
    topic: Option<i32>,
    search: bool,
}

/// Everything the rows are derived from, cheap to compare: when it matches
/// the last build, renders (playback ticks, hovers) skip rebuilding.
#[derive(Clone, Copy, PartialEq, Eq)]
struct RowsSource {
    key: RowsKey,
    messages: usize,
    ids: u64,
    loading: bool,
    anchor: Option<i64>,
    searching: bool,
    /// Search results: count, newest and oldest id.
    results: (usize, i64, i64),
}

pub struct Messenger {
    poll_question: Entity<InputState>,
    poll_options: Entity<TextareaState>,
    poll_anonymous: bool,
    poll_multiple: bool,
    sticker_search: Entity<InputState>,
    sticker_open: bool,
    /// Recents have been fetched this session.
    stickers_loaded: bool,
    /// Whether stickers showed before the profile took the panel.
    stickers_before_details: bool,
    stickers: Vec<tdlib_rs::types::Sticker>,
    /// Stickers this app sent, newest first; kept on disk so recents can go
    /// beyond what Telegram's server remembers.
    sticker_history: Vec<tdlib_rs::types::Sticker>,
    sticker_results: Option<(String, Vec<tdlib_rs::types::Sticker>)>,
    sticker_search_task: Option<Task<()>>,
    topics: HashMap<i64, Vec<tdlib_rs::types::ForumTopic>>,
    topic_offsets: HashMap<i64, (i32, i64, i32)>,
    topic_by_chat: HashMap<i64, i32>,
    topic_text: HashMap<(i64, Option<i32>), String>,
    topic_loading: HashSet<(i64, i32)>,
    topic_from: HashMap<(i64, i32), i64>,
    topic_end: HashSet<(i64, i32)>,
    forum_groups: HashSet<i64>,
    poll_choices: HashMap<(i64, i64), Vec<i32>>,
    notifications_enabled: bool,
    started_at: i64,
    read_requested: HashMap<i64, i64>,
    list: List,
    folders: Vec<tdlib_rs::types::ChatFolderInfo>,
    me: i64,
    users: HashMap<i64, tdlib_rs::types::User>,
    groups: HashMap<i64, Group>,
    /// Per chat: (sender, action label, when) for "typing…" indicators.
    typing: HashMap<i64, Vec<(i64, &'static str, Instant)>>,
    typing_timer: Option<Task<()>>,
    files: HashMap<i32, tdlib_rs::types::File>,
    downloading: HashSet<i32>,
    /// Decoded sticker animations, keyed by sticker file id and whether all
    /// frames (chat) or only the first (picker) were rendered.
    animations: HashMap<(i32, bool), CachedAnim>,
    animation_clock: std::cell::Cell<u64>,
    /// The window's regions as cached views; see `pane`.
    panes: Option<pane::Panes>,
    /// The pending redraw for the next sticker frame.
    frame_task: Option<Task<()>>,
    /// False while the window is inactive or the system asks for reduced motion.
    animate_stickers: bool,
    started: Instant,
    /// Decoded minithumbnails, keyed by the id of the file they preview.
    minithumbs: HashMap<i32, Arc<RenderImage>>,
    /// Files to open once their download completes.
    open_when_ready: HashSet<i32>,
    /// Decoded photos and avatars, bounded; see `crate::image_cache`.
    image_cache: Entity<crate::image_cache::LruImageCache>,
    profiles: HashMap<i64, Profile>,
    /// Custom emoji stickers by id, for reactions; None while requested.
    custom_emoji: HashMap<i64, Option<tdlib_rs::types::Sticker>>,
    /// The message menu's reactions expanded to the full list.
    reactions_expanded: bool,
    /// Unread messages in unmuted chats, for the menu bar badge.
    unread_total: i32,
    /// Reactions Telegram allows on the message whose menu is open.
    reaction_options: Option<((i64, i64), Vec<String>)>,
    /// Profile photos per chat, newest first, for the avatar viewer.
    profile_photos: HashMap<i64, Vec<tdlib_rs::types::ChatPhoto>>,
    shared_media: HashMap<i64, SharedMedia>,
    compose_actions: HashMap<i64, ComposeAction>,
    attachments: HashMap<i64, PathBuf>,
    draft_tasks: HashMap<i64, Task<()>>,
    search_task: Option<Task<()>>,
    search_results: Option<(String, Vec<i64>)>,
    message_search: Entity<InputState>,
    message_results: Option<(i64, String, Vec<Message>)>,
    message_search_open: bool,
    forward_from: Option<(i64, i64)>,
    client: Option<Client>,
    login: Login,
    login_hint: String,
    qr: Option<Arc<crate::qr::QrCode>>,
    resend: Option<(Instant, String)>,
    resend_timer: Option<Task<()>>,
    credentials: Option<Credentials>,
    auth_busy: bool,
    api_id: Entity<InputState>,
    api_hash: Entity<InputState>,
    login_input: Entity<InputState>,
    search: Entity<InputState>,
    menu: Option<menu::OpenMenu>,
    viewer: Option<viewer::Viewer>,
    playback: Option<playback::Playback>,
    /// (chat, message, file id) waiting for a download or conversion to play.
    pending_play: Option<(i64, i64, i32)>,
    palette_open: bool,
    palette_index: usize,
    palette_scroll: ScrollHandle,
    chats: HashMap<i64, Chat>,
    /// Chat ids shown in the sidebar, in order; rebuilt every render.
    chat_rows: Vec<i64>,
    chat_scroll: UniformListScrollHandle,
    composers: HashMap<i64, Entity<TextareaState>>,
    sending: HashSet<i64>,
    scroller: Entity<MessageScrollerState>,
    rows: Vec<Row>,
    rows_key: Option<RowsKey>,
    rows_source: Option<RowsSource>,
    initial_history: HashSet<i64>,
    /// Per chat: last read incoming message when the chat was opened; the
    /// unread divider sits after it.
    unread_anchor: HashMap<i64, i64>,
    /// Scroll to the unread divider once it appears.
    scroll_to_unread: bool,
    /// Pinned messages per chat (newest first) and which one the bar shows.
    pinned: HashMap<i64, Vec<Message>>,
    pinned_index: HashMap<i64, usize>,
    /// A message to scroll to once its history is loaded.
    jump_target: Option<(i64, i64)>,
    /// A message briefly highlighted after a jump.
    highlight: Option<i64>,
    highlight_task: Option<Task<()>>,
    /// Chats loading a newer page.
    newer_loading: HashSet<i64>,
    selected: Option<i64>,
    chats_loading: bool,
    chats_end: bool,
    details: bool,
    connection: &'static str,
    error: Option<String>,
    /// Seed data instead of a Telegram session (`TG_DEMO=1`).
    demo: bool,
    _events: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Messenger {
    pub fn quit(&mut self, cx: &mut Context<Self>) {
        let drafts: Vec<_> = self
            .composers
            .iter()
            .filter(|(id, _)| {
                !matches!(
                    self.compose_actions.get(id),
                    Some(ComposeAction::Edit { .. } | ComposeAction::Forward { .. })
                )
            })
            .map(|(&id, input)| (id, input.read(cx).value().to_string()))
            .collect();
        self.draft_tasks.clear();
        for (chat_id, text) in drafts {
            self.send(Command::Draft {
                chat_id,
                topic: self.topic_by_chat.get(&chat_id).copied(),
                text,
            });
        }
        self.login = Login::Waiting;
        self.login_hint = "Closing the Telegram session…".into();
        let closed = self.client.as_ref().map(Client::close);
        cx.spawn(async move |_, cx| {
            if let Some(closed) = closed {
                let _ = closed.recv().await;
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
        cx.notify();
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let api_id = cx.new(|cx| InputState::new(window, cx).placeholder("API ID"));
        let api_hash = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("API hash")
                .masked(true)
        });
        let login_input = cx.new(|cx| InputState::new(window, cx));
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search chats and actions"));
        let message_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search messages"));
        let subscriptions = vec![
            cx.subscribe(&search, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = input.read(cx).value().trim().to_owned();
                    this.search_results = None;
                    this.palette_index = 0;
                    this.search_task = Some(cx.spawn(async move |this, cx| {
                        smol::Timer::after(Duration::from_millis(250)).await;
                        let _ = this.update(cx, |this, cx| {
                            if !query.is_empty() {
                                this.send(Command::SearchChats(query));
                            }
                            cx.notify();
                        });
                    }));
                }
                cx.notify();
            }),
            cx.subscribe(&message_search, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change | InputEvent::PressEnter { .. })
                    && let Some(chat_id) = this.selected
                {
                    let query = input.read(cx).value().trim().to_owned();
                    if query.is_empty() {
                        this.message_results = None;
                    } else {
                        this.send(Command::SearchMessages {
                            chat_id,
                            topic: this.topic_by_chat.get(&chat_id).copied(),
                            query,
                        });
                    }
                }
                cx.notify();
            }),
            cx.on_app_quit(|this, _| {
                let closed = this.client.as_ref().map(Client::close);
                async move {
                    if let Some(closed) = closed {
                        let _ = closed.recv().await;
                    }
                }
            }),
        ];
        let poll_question = cx.new(|cx| InputState::new(window, cx).placeholder("Ask a question"));
        let poll_options = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Options — one per line")
                .auto_grow(4, 10)
        });
        let sticker_search = cx
            .new(|cx| InputState::new(window, cx).placeholder("Search stickers by emoji or word"));
        let sticker_subscription =
            cx.subscribe(&sticker_search, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = input.read(cx).value().trim().to_owned();
                    if query.is_empty() {
                        this.sticker_results = None;
                        this.sticker_search_task = None;
                    } else {
                        this.sticker_search_task = Some(cx.spawn(async move |this, cx| {
                            smol::Timer::after(Duration::from_millis(250)).await;
                            let _ = this.update(cx, |this, _| {
                                let chat_id = this.selected.unwrap_or(0);
                                this.send(Command::SearchStickers { query, chat_id });
                            });
                        }));
                    }
                }
                cx.notify();
            });
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        let demo = crate::demo::enabled();
        let mut app = Self {
            sticker_search,
            poll_question,
            poll_options,
            poll_anonymous: true,
            poll_multiple: false,
            // The sticker panel is open by default.
            sticker_open: true,
            stickers_loaded: false,
            stickers_before_details: false,
            stickers: Vec::new(),
            sticker_history: if demo {
                Vec::new()
            } else {
                load_sticker_history()
            },
            sticker_results: None,
            sticker_search_task: None,
            topics: HashMap::new(),
            topic_offsets: HashMap::new(),
            topic_by_chat: HashMap::new(),
            topic_text: HashMap::new(),
            topic_loading: HashSet::new(),
            topic_from: HashMap::new(),
            topic_end: HashSet::new(),
            forum_groups: HashSet::new(),
            poll_choices: HashMap::new(),
            notifications_enabled: false,
            started_at: chrono::Utc::now().timestamp(),
            read_requested: HashMap::new(),
            list: List::Main,
            folders: Vec::new(),
            me: 0,
            users: HashMap::new(),
            groups: HashMap::new(),
            typing: HashMap::new(),
            typing_timer: None,
            files: HashMap::new(),
            downloading: HashSet::new(),
            animations: HashMap::new(),
            animation_clock: std::cell::Cell::new(0),
            panes: None,
            frame_task: None,
            animate_stickers: true,
            started: Instant::now(),
            minithumbs: HashMap::new(),
            open_when_ready: HashSet::new(),
            image_cache: crate::image_cache::LruImageCache::new(cx),
            profiles: HashMap::new(),
            profile_photos: HashMap::new(),
            reaction_options: None,
            unread_total: 0,
            custom_emoji: HashMap::new(),
            reactions_expanded: false,
            shared_media: HashMap::new(),
            compose_actions: HashMap::new(),
            attachments: HashMap::new(),
            draft_tasks: HashMap::new(),
            search_task: None,
            search_results: None,
            message_search,
            message_results: None,
            message_search_open: false,
            forward_from: None,
            client: None,
            login: Login::Waiting,
            login_hint: "Connecting to Telegram…".into(),
            qr: None,
            resend: None,
            resend_timer: None,
            credentials: None,
            auth_busy: false,
            api_id,
            api_hash,
            login_input,
            search,
            menu: None,
            viewer: None,
            playback: None,
            pending_play: None,
            palette_open: false,
            palette_index: 0,
            palette_scroll: ScrollHandle::new(),
            chats: HashMap::new(),
            chat_rows: Vec::new(),
            chat_scroll: UniformListScrollHandle::new(),
            composers: HashMap::new(),
            sending: HashSet::new(),
            scroller,
            rows: Vec::new(),
            rows_key: None,
            rows_source: None,
            initial_history: HashSet::new(),
            unread_anchor: HashMap::new(),
            scroll_to_unread: false,
            pinned: HashMap::new(),
            pinned_index: HashMap::new(),
            jump_target: None,
            highlight: None,
            highlight_task: None,
            newer_loading: HashSet::new(),
            selected: None,
            chats_loading: false,
            chats_end: false,
            details: false,
            connection: "Connecting…",
            error: None,
            demo,
            _events: None,
            _subscriptions: subscriptions,
        };
        app._subscriptions.push(sticker_subscription);
        // Stickers pause while the window is inactive; resume on return.
        app._subscriptions
            .push(cx.observe_window_activation(window, |_, _, cx| cx.notify()));
        // Follow the macOS light/dark setting, live.
        ui::apply_theme(is_light(window.appearance()), cx);
        app._subscriptions
            .push(cx.observe_window_appearance(window, |_, window, cx| {
                ui::apply_theme(is_light(window.appearance()), cx);
                window.refresh();
            }));
        // Cmd+K and palette navigation run before inputs claim the keys.
        let weak = cx.entity().downgrade();
        app._subscriptions
            .push(cx.intercept_keystrokes(move |event, window, cx| {
                if let Some(key) = viewer::ViewerKey::from(&event.keystroke) {
                    let handled = weak
                        .update(cx, |this, cx| this.viewer_key(key, cx))
                        .unwrap_or(false);
                    if handled {
                        cx.stop_propagation();
                        return;
                    }
                }
                if let Some(format) = conversation::Format::from_keystroke(&event.keystroke) {
                    let handled = weak
                        .update(cx, |this, cx| {
                            let focused = this.composer_focused(window, cx);
                            if focused {
                                this.apply_format(format, window, cx);
                            }
                            focused
                        })
                        .unwrap_or(false);
                    if handled {
                        cx.stop_propagation();
                    }
                    return;
                }
                let Some(key) = palette::PaletteKey::from(&event.keystroke) else {
                    return;
                };
                let handled = weak
                    .update(cx, |this, cx| this.palette_key(key, window, cx))
                    .unwrap_or(false);
                if handled {
                    cx.stop_propagation();
                }
            }));
        if demo {
            app.load_demo(window, cx);
            return app;
        }
        cx.background_executor()
            .spawn(async { crate::media::prune_audio_cache() })
            .detach();
        let permission = crate::notifications::authorization(false);
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(allowed)) = permission.recv().await {
                let _ = this.update(cx, |this, cx| {
                    this.notifications_enabled = allowed;
                    cx.notify();
                });
            }
        })
        .detach();
        match Client::start() {
            Ok((client, events)) => {
                app.client = Some(client);
                app._events = Some(cx.spawn_in(window, async move |this, cx| {
                    // Apply every queued update, then render once: during sync
                    // TDLib sends hundreds of updates per second.
                    while let Ok(first) = events.recv().await {
                        let mut batch = vec![first];
                        while batch.len() < 512
                            && let Ok(event) = events.try_recv()
                        {
                            batch.push(event);
                        }
                        // Download progress alone only changes the conversation;
                        // it arrives per chunk, so leave the other panes cached.
                        let progress_only = batch.iter().all(is_progress);
                        let applied = this.update_in(cx, |this, window, cx| {
                            for event in batch {
                                this.event(event, window, cx);
                            }
                            if progress_only {
                                this.redraw_conversation(cx);
                            } else {
                                cx.notify();
                            }
                        });
                        if applied.is_err() {
                            break;
                        }
                    }
                }));
            }
            Err(error) => {
                app.login = Login::Blocked;
                app.error = Some(format!("Could not start Telegram: {error}"));
            }
        }
        app
    }

    fn send(&mut self, command: Command) -> bool {
        if self.demo {
            return false;
        }
        let result = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Telegram is not running"))
            .and_then(|client| client.send(command));
        match result {
            Ok(()) => true,
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        }
    }

    fn submit_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.auth_busy {
            return;
        }
        self.error = None;
        let text = self.login_input.read(cx).value().to_string();
        let command = match self.login {
            Login::Application => {
                let id = self.api_id.read(cx).value().trim().parse::<i32>();
                let hash = self.api_hash.read(cx).value().trim().to_owned();
                let Ok(api_id) = id else {
                    self.error = Some("Enter the numeric API ID from my.telegram.org.".into());
                    cx.notify();
                    return;
                };
                if api_id <= 0 || hash.len() != 32 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                    self.error = Some("Check your API ID and 32-character API hash.".into());
                    cx.notify();
                    return;
                }
                Command::Configure(Credentials {
                    api_id,
                    api_hash: hash,
                })
            }
            Login::Phone => Command::Phone(text.trim().into()),
            Login::Code => Command::Code(text.trim().into()),
            Login::Password => Command::Password(text),
            Login::Email => Command::Email(text.trim().into()),
            Login::EmailCode => Command::EmailCode(text.trim().into()),
            _ => return,
        };
        self.auth_busy = self.send(command);
        if self.auth_busy && self.login == Login::Password {
            self.login_input
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        cx.notify();
    }

    fn authorization(
        &mut self,
        state: AuthorizationState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.qr = None;
        self.resend = None;
        self.resend_timer = None;
        let (login, hint) = match state {
            AuthorizationState::WaitTdlibParameters => (Login::Application, "Enter your application credentials from my.telegram.org → API development tools. They’re saved on this Mac.".into()),
            AuthorizationState::WaitPhoneNumber => (Login::Phone, "Please confirm your country code and enter your phone number.".into()),
            AuthorizationState::WaitCode(code) => {
                let info = code.code_info;
                if let Some(next) = &info.next_type {
                    let deadline = Instant::now() + Duration::from_secs(info.timeout.max(0) as u64);
                    self.resend = Some((deadline, code_delivery(next)));
                    self.resend_timer = Some(cx.spawn(async move |this, cx| {
                        while Instant::now() < deadline {
                            smol::Timer::after(Duration::from_secs(1)).await;
                            if this.update(cx, |_, cx| cx.notify()).is_err() {
                                break;
                            }
                        }
                    }));
                }
                (Login::Code, format!("We’ve sent the code via {} to {}.", code_delivery(&info.r#type), info.phone_number))
            },
            AuthorizationState::WaitPassword(password) => (Login::Password, if password.password_hint.is_empty() { "Your account is protected with an additional password.".into() } else { format!("Your account is protected with an additional password. Hint: {}", password.password_hint) }),
            AuthorizationState::WaitEmailAddress(_) => (Login::Email, "Telegram requires an email address to continue signing in.".into()),
            AuthorizationState::WaitEmailCode(code) => (Login::EmailCode, format!("Enter the code sent to {}.", code.code_info.email_address_pattern)),
            AuthorizationState::Ready => (Login::Ready, String::new()),
            AuthorizationState::WaitRegistration(_) => (Login::Blocked, "This phone number needs a new Telegram account. Create it in the official app, then restart here.".into()),
            AuthorizationState::WaitOtherDeviceConfirmation(info) => {
                match crate::qr::QrCode::new(&info.link) {
                    Ok(qr) => {
                        self.qr = Some(Arc::new(qr));
                        (Login::Qr, "Open Telegram on your phone, go to Settings → Devices → Link Desktop Device, and point your phone at this screen.".into())
                    }
                    Err(error) => (Login::Blocked, format!("Could not display QR code: {error}")),
                }
            },
            AuthorizationState::WaitPremiumPurchase(_) => (Login::Blocked, "Telegram requires a Premium purchase for this login. Complete it in the official app.".into()),
            AuthorizationState::LoggingOut | AuthorizationState::Closing => (Login::Waiting, "Closing the Telegram session…".into()),
            AuthorizationState::Closed => (Login::Blocked, "The Telegram session has closed. Restart the app to reconnect.".into()),
        };
        let changed = self.login != login;
        self.login = login;
        self.login_hint = hint;
        if changed {
            self.error = None;
            self.login_input.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.set_masked(login == Login::Password, window, cx);
                input.set_placeholder(login::placeholder(login), window, cx);
            });
        }
        if login == Login::Application
            && let Some(credentials) = self.credentials.take()
        {
            self.auth_busy = self.send(Command::Configure(credentials));
        }
        if login == Login::Ready && changed {
            self.load_chats(cx);
            // Learn whether the archive has chats, for its row in the list.
            self.send(Command::LoadChats(List::Archive));
        }
    }

    fn event(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            Event::Credentials(result) => match result {
                Ok(credentials) => {
                    self.credentials = credentials;
                    if self.login == Login::Application
                        && !self.auth_busy
                        && let Some(credentials) = self.credentials.take()
                    {
                        self.auth_busy = self.send(Command::Configure(credentials));
                    }
                }
                Err(error) => self.error = Some(error),
            },
            Event::Completed(request, result) => self.completed(request, result, window, cx),
            Event::Update(update) => self.update(*update, window, cx),
        }
    }

    fn completed(
        &mut self,
        request: Request,
        result: Result<Reply, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match request {
            Request::SendPoll => {
                if result.is_ok() {
                    self.poll_question
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    self.poll_options
                        .update(cx, |input, cx| input.set_value("", window, cx));
                }
            }
            Request::TopicHistory(chat, topic) => {
                self.topic_loading.remove(&(chat, topic));
            }
            Request::Auth => self.auth_busy = false,
            Request::Chats(list) => {
                if list == self.list {
                    self.chats_loading = false;
                }
            }
            Request::History(id) => {
                if let Some(chat) = self.chats.get_mut(&id) {
                    chat.history_loading = false;
                }
            }
            Request::Download(id) => {
                if result.is_err() {
                    self.downloading.remove(&id);
                    self.open_when_ready.remove(&id);
                }
            }
            Request::Send(id) => {
                self.sending.remove(&id);
                if result.is_ok()
                    && let Some(input) = self.composers.get(&id)
                {
                    input.update(cx, |input, cx| {
                        input.set_value(
                            match self.compose_actions.get(&id) {
                                Some(ComposeAction::Edit { previous, .. }) => previous.clone(),
                                Some(ComposeAction::Forward { .. }) => input.value().to_string(),
                                _ => String::new(),
                            },
                            window,
                            cx,
                        )
                    });
                    self.compose_actions.remove(&id);
                    self.attachments.remove(&id);
                    if self.selected == Some(id) {
                        self.scroll_to_end(cx);
                    }
                }
            }
            _ => {}
        }
        match result {
            Err(error) => {
                if !matches!(
                    request,
                    Request::Background
                        | Request::Download(_)
                        | Request::SharedMedia(_)
                        | Request::CustomEmoji
                        | Request::AvailableReactions(..)
                ) {
                    self.error = Some(error);
                }
            }
            Ok(Reply::Stickers(stickers)) => {
                self.prepare_stickers(&stickers, cx);
                self.stickers = stickers;
            }
            Ok(Reply::StickerSearch(query, stickers)) => {
                if self.sticker_search.read(cx).value().trim() == query {
                    self.prepare_stickers(&stickers, cx);
                    self.sticker_results = Some((query, stickers));
                }
            }
            Ok(Reply::Topics(found)) => {
                if let Request::Topics(chat) = request {
                    self.topic_offsets.insert(
                        chat,
                        (
                            found.next_offset_date,
                            found.next_offset_message_id,
                            found.next_offset_forum_topic_id,
                        ),
                    );
                    let topics = self.topics.entry(chat).or_default();
                    for topic in found.topics {
                        topics.retain(|t| t.info.forum_topic_id != topic.info.forum_topic_id);
                        topics.push(topic);
                    }
                    topics.sort_by_key(|t| std::cmp::Reverse(t.order));
                }
            }
            Ok(Reply::TopicHistory(messages)) => {
                for message in &messages {
                    self.prepare_media(message);
                }
                if let Request::TopicHistory(chat_id, topic) = request {
                    if let Some(from) = messages.iter().map(|m| m.id).min() {
                        self.topic_from.insert((chat_id, topic), from);
                    }
                    if messages.is_empty() {
                        self.topic_end.insert((chat_id, topic));
                    }
                    if let Some(chat) = self.chats.get_mut(&chat_id) {
                        for message in messages {
                            chat.messages.insert(message.id, message);
                        }
                    }
                }
            }
            Ok(Reply::Done) => {}
            Ok(Reply::OpenChat(id)) => self.select_chat(id, window, cx),
            Ok(Reply::ChatsLoaded(end)) => {
                if matches!(request, Request::Chats(list) if list == self.list) {
                    self.chats_end = end;
                }
            }
            Ok(Reply::SearchChats(query, ids)) => {
                if self.search.read(cx).value().trim() == query {
                    self.search_results = Some((query, ids));
                }
            }
            Ok(Reply::SearchMessages(query, messages)) => {
                if let Request::SearchMessages(id) = request
                    && self.selected == Some(id)
                    && self.message_search.read(cx).value().trim() == query
                {
                    for message in &messages {
                        self.prepare_media(message);
                    }
                    self.message_results = Some((id, query, messages));
                }
            }
            Ok(Reply::File(file)) => self.cache_file(file, cx),
            Ok(Reply::Profile(profile)) => {
                if let Request::Profile(id) = request {
                    self.profiles.insert(id, profile);
                }
            }
            Ok(Reply::CustomEmoji(stickers)) => {
                self.prepare_stickers(&stickers, cx);
                for sticker in stickers {
                    if let tdlib_rs::enums::StickerFullType::CustomEmoji(custom) =
                        &sticker.full_type
                    {
                        self.custom_emoji
                            .insert(custom.custom_emoji_id, Some(sticker));
                    }
                }
            }
            Ok(Reply::AvailableReactions(emoji)) => {
                if let Request::AvailableReactions(chat_id, message_id) = request {
                    self.reaction_options = Some(((chat_id, message_id), emoji));
                }
            }
            Ok(Reply::ProfilePhotos(photos)) => {
                if let Request::ProfilePhotos(id) = request {
                    self.profile_photos.insert(id, photos);
                    self.prefetch_viewer();
                }
            }
            Ok(Reply::SharedMedia(media)) => {
                if let Request::SharedMedia(id) = request {
                    for message in &media.recent {
                        if let Some(file) = grid_file(message) {
                            let file = file.clone();
                            self.request_file(&file, 4, true);
                        }
                    }
                    self.shared_media.insert(id, media);
                }
            }
            Ok(Reply::History(from, page, window, messages)) => {
                if let Request::History(id) = request {
                    if page == Page::Newer {
                        self.newer_loading.remove(&id);
                    }
                    // A page for a window that was since replaced (a jump).
                    if self
                        .chats
                        .get(&id)
                        .is_some_and(|chat| chat.window != window)
                    {
                        return;
                    }
                    for message in &messages {
                        self.prepare_media(message);
                    }
                    let mut more = false;
                    if let Some(chat) = self.chats.get_mut(&id) {
                        let before = chat.messages.len();
                        // An empty newest-page refresh doesn't mean the start
                        // of history was reached; only paging (or an empty chat) does.
                        if page == Page::Older
                            && messages.is_empty()
                            && (from != 0 || chat.messages.is_empty())
                        {
                            chat.history_end = true;
                        }
                        for message in messages {
                            chat.messages.insert(message.id, message);
                        }
                        if page != Page::Older || from == 0 {
                            let newest = chat.messages.last_key_value().map_or(0, |(&id, _)| id);
                            chat.has_newer = chat
                                .info
                                .last_message
                                .as_ref()
                                .is_some_and(|m| m.id > newest);
                        }
                        more = page == Page::Older
                            && self.initial_history.contains(&id)
                            && chat.messages.len() < 40
                            && chat.messages.len() > before
                            && !chat.history_end;
                    }
                    if more {
                        self.load_history(id, cx);
                    } else if page == Page::Older {
                        self.initial_history.remove(&id);
                    }
                }
            }
            Ok(Reply::Pinned(mut messages)) => {
                if let Request::Pinned(id) = request {
                    messages.sort_by_key(|m| std::cmp::Reverse(m.id));
                    let index = self.pinned_index.entry(id).or_default();
                    if *index >= messages.len() {
                        *index = 0;
                    }
                    self.pinned.insert(id, messages);
                }
            }
        }
    }

    fn update(&mut self, update: Update, window: &mut Window, cx: &mut Context<Self>) {
        match update {
            Update::AuthorizationState(update) => {
                self.authorization(update.authorization_state, window, cx)
            }
            Update::ConnectionState(update) => {
                self.connection = match update.state {
                    ConnectionState::Ready => "Connected",
                    ConnectionState::Updating => "Updating…",
                    ConnectionState::WaitingForNetwork => "Waiting for network…",
                    _ => "Connecting…",
                }
            }
            Update::Option(update) => {
                if update.name == "my_id"
                    && let tdlib_rs::enums::OptionValue::Integer(value) = update.value
                {
                    self.me = value.value;
                }
            }
            Update::NotificationGroup(update) => {
                if self.notifications_enabled && !window.is_window_active() {
                    for notification in update.added_notifications {
                        if i64::from(notification.date) < self.started_at {
                            continue;
                        }
                        if let tdlib_rs::enums::NotificationType::NewMessage(info) =
                            notification.r#type
                        {
                            let title = self
                                .chats
                                .get(&update.chat_id)
                                .map_or("Kite", |c| c.info.title.as_str());
                            let body: String = if info.show_preview {
                                message_text(&info.message.content)
                                    .chars()
                                    .take(200)
                                    .collect()
                            } else {
                                "New message".into()
                            };
                            let result = crate::notifications::show(
                                notification.id,
                                title,
                                &body,
                                notification.is_silent,
                            );
                            cx.spawn(async move |this, cx| {
                                if let Ok(Err(error)) = result.recv().await {
                                    let _ = this.update(cx, |this, cx| {
                                        this.error = Some(format!("Notification: {error}"));
                                        cx.notify();
                                    });
                                }
                            })
                            .detach();
                        }
                    }
                }
            }
            Update::UserStatus(update) => {
                if let Some(user) = self.users.get_mut(&update.user_id) {
                    user.status = update.status;
                }
            }
            Update::BasicGroup(update) => {
                self.groups.insert(
                    update.basic_group.id,
                    Group {
                        members: update.basic_group.member_count,
                        channel: false,
                        can_post: true,
                    },
                );
            }
            Update::ChatAction(update) => {
                let sender = match &update.sender_id {
                    tdlib_rs::enums::MessageSender::User(user) => user.user_id,
                    tdlib_rs::enums::MessageSender::Chat(chat) => chat.chat_id,
                };
                let typing = self.typing.entry(update.chat_id).or_default();
                typing.retain(|(id, _, _)| *id != sender);
                if let Some(label) = chat_action(&update.action) {
                    typing.push((sender, label, Instant::now()));
                    // Telegram repeats actions every few seconds; expire stale ones.
                    self.typing_timer = Some(cx.spawn(async move |this, cx| {
                        smol::Timer::after(Duration::from_secs(6)).await;
                        let _ = this.update(cx, |this, cx| {
                            for list in this.typing.values_mut() {
                                list.retain(|(_, _, at)| at.elapsed() < Duration::from_secs(6));
                            }
                            cx.notify();
                        });
                    }));
                }
            }
            Update::Supergroup(update) => {
                let group = &update.supergroup;
                let can_post = !group.is_channel
                    || match &group.status {
                        tdlib_rs::enums::ChatMemberStatus::Creator(_) => true,
                        tdlib_rs::enums::ChatMemberStatus::Administrator(admin) => {
                            admin.rights.can_post_messages
                        }
                        _ => false,
                    };
                self.groups.insert(
                    group.id,
                    Group {
                        members: group.member_count,
                        channel: group.is_channel,
                        can_post,
                    },
                );
                if group.is_forum {
                    self.forum_groups.insert(group.id);
                } else {
                    self.forum_groups.remove(&group.id);
                }
            }
            Update::ForumTopicInfo(update) => {
                if let Some(topics) = self.topics.get_mut(&update.info.chat_id)
                    && let Some(topic) = topics
                        .iter_mut()
                        .find(|t| t.info.forum_topic_id == update.info.forum_topic_id)
                {
                    topic.info = update.info;
                }
            }
            Update::User(update) => {
                self.users.insert(update.user.id, update.user);
            }
            Update::File(update) => self.cache_file(update.file, cx),
            Update::ChatFolders(update) => {
                self.folders = update.chat_folders;
            }
            Update::ChatPhoto(update) => {
                if let Some(photo) = &update.photo {
                    self.request_file(&photo.small.clone(), 8, true);
                }
                if let Some(chat) = self.chats.get_mut(&update.chat_id) {
                    chat.info.photo = update.photo;
                }
            }
            Update::ChatReadOutbox(update) => {
                if let Some(chat) = self.chats.get_mut(&update.chat_id) {
                    chat.info.last_read_outbox_message_id = update.last_read_outbox_message_id;
                }
            }
            Update::ChatNotificationSettings(update) => {
                if let Some(chat) = self.chats.get_mut(&update.chat_id) {
                    chat.info.notification_settings = update.notification_settings;
                }
            }
            Update::ChatUnreadMentionCount(update) => {
                if let Some(chat) = self.chats.get_mut(&update.chat_id) {
                    chat.info.unread_mention_count = update.unread_mention_count;
                }
            }
            Update::ChatIsMarkedAsUnread(update) => {
                if let Some(chat) = self.chats.get_mut(&update.chat_id) {
                    chat.info.is_marked_as_unread = update.is_marked_as_unread;
                }
            }
            Update::MessageEdited(update) => {
                if let Some(message) = self
                    .chats
                    .get_mut(&update.chat_id)
                    .and_then(|c| c.messages.get_mut(&update.message_id))
                {
                    message.edit_date = update.edit_date;
                }
            }
            Update::MessageIsPinned(update) => {
                if let Some(message) = self
                    .chats
                    .get_mut(&update.chat_id)
                    .and_then(|c| c.messages.get_mut(&update.message_id))
                {
                    message.is_pinned = update.is_pinned;
                }
                if self.selected == Some(update.chat_id) {
                    self.send(Command::Pinned(update.chat_id));
                }
            }
            Update::MessageInteractionInfo(update) => {
                self.request_custom_emoji(update.interaction_info.as_ref());
                if let Some(message) = self
                    .chats
                    .get_mut(&update.chat_id)
                    .and_then(|c| c.messages.get_mut(&update.message_id))
                {
                    message.interaction_info = update.interaction_info;
                }
            }
            Update::NewChat(update) => {
                if let Some(photo) = &update.chat.photo {
                    self.request_file(&photo.small.clone(), 8, true);
                }
                self.chats.insert(update.chat.id, Chat::new(update.chat));
            }
            Update::ChatTitle(update) => {
                if let Some(chat) = self.chats.get_mut(&update.chat_id) {
                    chat.info.title = update.title;
                }
            }
            Update::ChatPosition(update) => {
                if let Some(chat) = self.chats.get_mut(&update.chat_id) {
                    chat.position(update.position);
                }
            }
            Update::ChatLastMessage(update) => {
                if let Some(chat) = self.chats.get_mut(&update.chat_id) {
                    chat.info.last_message = update.last_message;
                    chat.info.positions = update.positions;
                }
            }
            Update::ChatDraftMessage(update) => {
                if let Some(chat) = self.chats.get_mut(&update.chat_id) {
                    chat.info.positions = update.positions;
                    chat.info.draft_message = update.draft_message;
                }
            }
            // Telegram's own total, which leaves out muted chats like its
            // badge does; shown on the menu bar icon.
            Update::UnreadMessageCount(update)
                if update.chat_list == tdlib_rs::enums::ChatList::Main =>
            {
                self.set_unread(update.unread_unmuted_count, cx);
            }
            Update::ChatReadInbox(update) => {
                if let Some(chat) = self.chats.get_mut(&update.chat_id) {
                    chat.info.unread_count = update.unread_count;
                }
            }
            Update::NewMessage(update) => {
                let message = update.message;
                // A chat with loaded history keeps a contiguous tail, so new
                // messages join it even while the chat isn't open.
                let cached = self
                    .chats
                    .get(&message.chat_id)
                    .is_some_and(|chat| !chat.messages.is_empty());
                let selected = self.selected == Some(message.chat_id);
                if selected || cached {
                    self.prepare_media(&message);
                }
                let sender = Self::sender_id(&message);
                if let Some(typing) = self.typing.get_mut(&message.chat_id) {
                    typing.retain(|(id, _, _)| *id != sender);
                }
                let own = message.is_outgoing && self.selected == Some(message.chat_id);
                let behind = self
                    .chats
                    .get(&message.chat_id)
                    .is_some_and(|chat| chat.has_newer);
                if behind {
                    // Viewing older history: sending returns to the latest,
                    // incoming messages load when scrolled to.
                    if own {
                        self.jump_to_latest(message.chat_id, cx);
                    }
                } else if let Some(chat) = self.chats.get_mut(&message.chat_id)
                    && (selected || cached || message.is_outgoing)
                {
                    let chat_id = message.chat_id;
                    chat.messages.insert(message.id, message);
                    // A busy chat in the background would otherwise grow all session.
                    if !selected && chat.messages.len() > 2 * HISTORY_KEEP {
                        self.trim_history(chat_id, cx);
                    }
                }
                if own {
                    self.scroll_to_end(cx);
                }
            }
            Update::MessageSendSucceeded(update) => {
                if let Some(chat) = self.chats.get_mut(&update.message.chat_id) {
                    chat.messages.remove(&update.old_message_id);
                    chat.messages.insert(update.message.id, update.message);
                }
            }
            Update::MessageSendFailed(update) => {
                self.error = Some(format!("Message failed to send: {}", update.error.message));
                if let Some(chat) = self.chats.get_mut(&update.message.chat_id) {
                    chat.messages.remove(&update.old_message_id);
                    chat.messages.insert(update.message.id, update.message);
                }
            }
            Update::MessageContent(update) => {
                if let Some((_, _, messages)) = &mut self.message_results {
                    for message in messages
                        .iter_mut()
                        .filter(|m| m.chat_id == update.chat_id && m.id == update.message_id)
                    {
                        message.content = update.new_content.clone();
                    }
                }
                if let Some(chat) = self.chats.get_mut(&update.chat_id) {
                    if let Some(message) = chat.messages.get_mut(&update.message_id) {
                        message.content = update.new_content.clone();
                    }
                    if let Some(message) = &mut chat.info.last_message
                        && message.id == update.message_id
                    {
                        message.content = update.new_content;
                    }
                }
            }
            Update::DeleteMessages(update) => {
                if !update.is_permanent {
                    return;
                }
                if let Some((_, _, messages)) = &mut self.message_results {
                    messages.retain(|m| {
                        m.chat_id != update.chat_id || !update.message_ids.contains(&m.id)
                    });
                }
                let removed: Vec<Message> = self
                    .chats
                    .get_mut(&update.chat_id)
                    .map(|chat| {
                        update
                            .message_ids
                            .iter()
                            .filter_map(|id| chat.messages.remove(id))
                            .collect()
                    })
                    .unwrap_or_default();
                for message in &removed {
                    self.forget_media(message, cx);
                }
            }
            _ => {}
        }
    }

    fn set_unread(&mut self, count: i32, cx: &mut App) {
        self.unread_total = count;
        self.sync_tray(cx);
    }

    /// Shows the unread count on the menu bar item, once it exists.
    pub fn sync_tray(&self, cx: &App) {
        if let Some(tray) = cx.try_global::<crate::tray::Tray>() {
            tray.set_unread(self.unread_total);
        }
    }

    /// Keeps only the newest messages of a chat that was left, so memory
    /// doesn't grow with every long history scrolled through. Forum chats
    /// page per topic and keep theirs.
    fn trim_history(&mut self, id: i64, cx: &mut App) {
        const KEEP: usize = HISTORY_KEEP;
        if self.topic_by_chat.contains_key(&id) || self.forward_from.is_some() {
            return;
        }
        let Some(chat) = self.chats.get_mut(&id) else {
            return;
        };
        if chat.messages.len() <= KEEP || chat.history_loading {
            return;
        }
        let Some(&cut) = chat.messages.keys().rev().nth(KEEP - 1) else {
            return;
        };
        let kept = chat.messages.split_off(&cut);
        let dropped = std::mem::replace(&mut chat.messages, kept);
        chat.history_end = false;
        self.initial_history.remove(&id);
        for message in dropped.values() {
            self.forget_media(message, cx);
        }
    }

    /// Frees what was decoded for a message that is no longer kept.
    fn forget_media(&mut self, message: &Message, cx: &mut App) {
        if let Some((_, key)) = minithumbnail(&message.content)
            && let Some(image) = self.minithumbs.remove(&key)
        {
            cx.drop_image(image, None);
        }
    }

    fn load_chats(&mut self, cx: &mut Context<Self>) {
        if !self.chats_loading && !self.chats_end {
            self.chats_loading = self.send(Command::LoadChats(self.list));
        }
        cx.notify();
    }

    fn load_history(&mut self, id: i64, cx: &mut Context<Self>) {
        if let Some(&topic) = self.topic_by_chat.get(&id) {
            if self.topic_loading.contains(&(id, topic)) || self.topic_end.contains(&(id, topic)) {
                return;
            }
            let from = self.topic_from.get(&(id, topic)).copied().unwrap_or(0);
            if self.send(Command::TopicHistory {
                chat_id: id,
                topic,
                from,
            }) {
                self.topic_loading.insert((id, topic));
            }
            cx.notify();
            return;
        }
        let Some(chat) = self.chats.get(&id) else {
            return;
        };
        if chat.history_loading || chat.history_end {
            return;
        }
        let from = chat.messages.first_key_value().map_or(0, |(&id, _)| id);
        let window = chat.window;
        if self.send(Command::History {
            chat_id: id,
            from,
            page: Page::Older,
            window,
        }) && let Some(chat) = self.chats.get_mut(&id)
        {
            chat.history_loading = true;
        }
        cx.notify();
    }

    fn send_message(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.sending.contains(&id) || self.selected != Some(id) {
            return;
        }
        let Some(input) = self.composers.get(&id) else {
            return;
        };
        let text = input.read(cx).value().trim_end().to_string();
        let action = self.compose_actions.get(&id).cloned();
        let attachment = self.attachments.get(&id).cloned();
        if text.trim().is_empty()
            && attachment.is_none()
            && !matches!(action, Some(ComposeAction::Forward { .. }))
        {
            return;
        }
        self.error = None;
        self.draft_tasks.remove(&id);
        let command = match action {
            Some(ComposeAction::Edit { id: message_id, .. }) => Command::Edit {
                chat_id: id,
                message_id,
                text,
                caption: self.find_message(id, message_id).is_some_and(|m| {
                    !matches!(m.content, tdlib_rs::enums::MessageContent::MessageText(_))
                }),
            },
            Some(ComposeAction::Forward {
                from,
                id: message_id,
            }) => Command::Forward {
                chat_id: id,
                topic: self.topic_by_chat.get(&id).copied(),
                from,
                message_id,
            },
            _ => Command::SendMessage {
                chat_id: id,
                topic: self.topic_by_chat.get(&id).copied(),
                text,
                reply_to: match action {
                    Some(ComposeAction::Reply(message_id)) => Some(message_id),
                    _ => None,
                },
                attachment,
            },
        };
        if self.send(command) {
            self.sending.insert(id);
        }
        cx.notify();
    }

    fn composer(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.composers.contains_key(&id) {
            return;
        }
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Write a message…")
                .auto_grow(1, 8)
                .submit_on_enter(true)
        });
        self._subscriptions.push(
            cx.subscribe_in(&input, window, move |this, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    this.send_message(id, cx);
                }
                if matches!(event, InputEvent::Change) {
                    this.save_draft_later(id, cx);
                }
                cx.notify();
            }),
        );
        if let Some(draft) = self
            .chats
            .get(&id)
            .and_then(|c| c.info.draft_message.as_ref())
            && let tdlib_rs::enums::InputMessageContent::InputMessageText(text) =
                &draft.input_message_text
        {
            input.update(cx, |input, cx| {
                input.set_value(text.text.text.clone(), window, cx)
            });
        }
        self.composers.insert(id, input);
    }

    fn select_chat(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        // Clicking the open chat again goes to its latest message.
        if self.selected == Some(id) && self.forward_from.is_none() {
            self.jump_to_latest(id, cx);
            return;
        }
        if let Some(previous) = self.selected {
            self.send(Command::CloseChat(previous));
            self.trim_history(previous, cx);
        }
        self.composer(id, window, cx);
        self.message_results = None;
        self.message_search_open = false;
        if let Some((from, message_id)) = self.forward_from.take() {
            self.compose_actions.insert(
                id,
                ComposeAction::Forward {
                    from,
                    id: message_id,
                },
            );
        }
        self.selected = Some(id);
        self.error = None;
        if self.sticker_open && !self.stickers_loaded {
            self.load_stickers(cx);
        }
        self.send(Command::OpenChat(id));
        if !self.profiles.contains_key(&id)
            && let Some(chat) = self.chats.get(&id)
        {
            self.send(Command::Profile {
                chat_id: id,
                kind: chat.info.r#type.clone(),
            });
        }
        if self.details {
            self.send(Command::SharedMedia(id));
        }
        if let Some(chat) = self.chats.get(&id)
            && let tdlib_rs::enums::ChatType::Supergroup(group) = &chat.info.r#type
            && self.forum_groups.contains(&group.supergroup_id)
            && !self.topics.contains_key(&id)
        {
            self.send(Command::Topics {
                chat_id: id,
                offset: (0, 0, 0),
            });
        }
        self.jump_target = None;
        self.send(Command::Pinned(id));
        // Remember where unread messages start, for the divider.
        let topic = self.topic_by_chat.contains_key(&id);
        let anchor = self
            .chats
            .get(&id)
            .filter(|chat| chat.info.unread_count > 0 && !topic)
            .map(|chat| chat.info.last_read_inbox_message_id);
        match anchor {
            Some(anchor) => {
                self.unread_anchor.insert(id, anchor);
                self.scroll_to_unread = true;
            }
            None => {
                self.unread_anchor.remove(&id);
                self.scroll_to_unread = false;
            }
        }
        // A window left over from an earlier jump starts again from the latest.
        if let Some(chat) = self.chats.get_mut(&id)
            && chat.has_newer
        {
            chat.reset_window();
        }
        let anchor_loaded = anchor.is_some_and(|anchor| {
            self.chats
                .get(&id)
                .is_some_and(|chat| chat.messages.contains_key(&anchor))
        });
        if let Some(anchor) = anchor.filter(|&a| a > 0 && !anchor_loaded) {
            // Load the history around the first unread message.
            self.load_around(id, anchor, cx);
        } else if self
            .chats
            .get(&id)
            .is_some_and(|chat| chat.messages.len() < 20)
        {
            self.initial_history.insert(id);
            self.load_history(id, cx);
        } else if let Some(chat) = self.chats.get_mut(&id)
            && !chat.history_loading
            && !topic
        {
            // Fetch the newest page so anything missed while away is merged in.
            let window = chat.window;
            if self.send(Command::History {
                chat_id: id,
                from: 0,
                page: Page::Older,
                window,
            }) && let Some(chat) = self.chats.get_mut(&id)
            {
                chat.history_loading = true;
            }
        }
        if let Some(input) = self.composers.get(&id) {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    /// Closes the open chat and returns to the empty state.
    fn close_chat(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected.take() {
            self.send(Command::CloseChat(id));
            self.trim_history(id, cx);
        }
        if self.details && self.stickers_before_details {
            self.sticker_open = true;
        }
        self.details = false;
        self.message_search_open = false;
        self.message_results = None;
        cx.notify();
    }

    /// Esc backs out one layer at a time; returns whether it did anything.
    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if window.has_active_dialog(cx) {
            return false;
        }
        let Some(id) = self.selected else {
            // With no chat open, Esc leaves the archive.
            if self.list == List::Archive {
                self.switch_list(List::Main, window, cx);
                return true;
            }
            return false;
        };
        if self.compose_actions.contains_key(&id) || self.attachments.contains_key(&id) {
            self.cancel_compose_action(id, window, cx);
        } else if self.message_search_open {
            self.message_search_open = false;
            self.message_results = None;
        } else {
            self.close_chat(cx);
        }
        cx.notify();
        true
    }

    fn confirm_leave(&self, chat_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::button::{Button, ButtonVariants};
        let Some(chat) = self.chats.get(&chat_id) else {
            return;
        };
        let channel = self.is_channel(chat);
        let title = chat.info.title.clone();
        let this = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let this = this.clone();
            dialog
                .title(if channel {
                    "Leave channel"
                } else {
                    "Leave group"
                })
                .w(px(380.))
                .child(
                    div()
                        .text_color(ui::hsla(ui::SECONDARY))
                        .child(format!("Are you sure you want to leave {title}?")),
                )
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("leave-cancel")
                                .ghost()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("leave-confirm")
                                .danger()
                                .label("Leave")
                                .on_click(move |_, window, cx| {
                                    let _ = this.update(cx, |this, cx| {
                                        this.send(Command::Leave(chat_id));
                                        if this.selected == Some(chat_id) {
                                            this.close_chat(cx);
                                        }
                                    });
                                    window.close_dialog(cx);
                                }),
                        ),
                )
        });
    }

    /// Replaces the loaded window with history around `message_id`.
    fn load_around(&mut self, chat_id: i64, message_id: i64, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get_mut(&chat_id) else {
            return;
        };
        chat.reset_window();
        chat.has_newer = true;
        let window = chat.window;
        if self.send(Command::History {
            chat_id,
            from: message_id,
            page: Page::Around,
            window,
        }) && let Some(chat) = self.chats.get_mut(&chat_id)
        {
            chat.history_loading = true;
        }
        cx.notify();
    }

    /// Scrolls to a message, loading the history around it if needed, and
    /// highlights it briefly.
    fn jump_to(&mut self, chat_id: i64, message_id: i64, cx: &mut Context<Self>) {
        if self.selected != Some(chat_id) {
            return;
        }
        self.message_search_open = false;
        self.message_results = None;
        self.jump_target = Some((chat_id, message_id));
        let loaded = self
            .chats
            .get(&chat_id)
            .is_some_and(|chat| chat.messages.contains_key(&message_id));
        if !loaded && !self.topic_by_chat.contains_key(&chat_id) {
            self.load_around(chat_id, message_id, cx);
        }
        cx.notify();
    }

    /// Loads the next newer page when the window ends before the latest message.
    fn load_newer(&mut self, chat_id: i64, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(&chat_id) else {
            return;
        };
        if !chat.has_newer || chat.history_loading || self.newer_loading.contains(&chat_id) {
            return;
        }
        let Some(&from) = chat.messages.keys().next_back() else {
            return;
        };
        let window = chat.window;
        if self.send(Command::History {
            chat_id,
            from,
            page: Page::Newer,
            window,
        }) {
            self.newer_loading.insert(chat_id);
        }
        cx.notify();
    }

    /// Shows the chat's latest message. When the loaded window ends before it
    /// (after a jump), the window is replaced by the newest page rather than
    /// paged forward one page at a time.
    pub(super) fn jump_to_latest(&mut self, chat_id: i64, cx: &mut Context<Self>) {
        self.jump_target = None;
        self.scroll_to_unread = false;
        if let Some(chat) = self.chats.get_mut(&chat_id)
            && chat.has_newer
        {
            chat.reset_window();
            self.newer_loading.remove(&chat_id);
            self.initial_history.insert(chat_id);
            self.load_history(chat_id, cx);
        }
        self.scroll_to_end(cx);
        cx.notify();
    }

    fn flash(&mut self, message_id: i64, cx: &mut Context<Self>) {
        self.highlight = Some(message_id);
        self.highlight_task = Some(cx.spawn(async move |this, cx| {
            smol::Timer::after(Duration::from_millis(1600)).await;
            let _ = this.update(cx, |this, cx| {
                this.highlight = None;
                cx.notify();
            });
        }));
    }

    fn toggle_details(&mut self, cx: &mut Context<Self>) {
        self.details = !self.details;
        if self.details {
            // The profile and stickers share the right-hand panel.
            self.stickers_before_details = self.sticker_open;
            self.sticker_open = false;
            if let Some(id) = self.selected {
                self.send(Command::SharedMedia(id));
            }
        } else if self.stickers_before_details {
            self.sticker_open = true;
        }
        cx.notify();
    }

    fn toggle_stickers(&mut self, cx: &mut Context<Self>) {
        self.sticker_open = !self.sticker_open;
        if self.sticker_open {
            self.details = false;
            self.load_stickers(cx);
        }
        cx.notify();
    }

    fn load_stickers(&mut self, cx: &mut Context<Self>) {
        self.stickers_loaded = true;
        self.send(Command::Stickers);
        let history = self.sticker_history.clone();
        self.prepare_stickers(&history, cx);
    }

    /// Downloads picker thumbnails and decodes first frames of animated ones.
    fn prepare_stickers(&mut self, stickers: &[tdlib_rs::types::Sticker], cx: &mut Context<Self>) {
        for sticker in stickers {
            if let Some(file) = sticker_file(sticker) {
                let file = file.clone();
                self.request_file(&file, 20, true);
            }
            self.ensure_animation(sticker, false, cx);
        }
    }

    /// Re-decodes picker previews that the animation cache evicted.
    fn ensure_picker_frames(&mut self, cx: &mut Context<Self>) {
        let missing: Vec<_> = self
            .picker_stickers(cx)
            .into_iter()
            // Only animated formats with a file on disk: WebP renders from
            // its path, and downloads trigger decoding when they finish.
            .filter(|s| {
                !matches!(s.format, tdlib_rs::enums::StickerFormat::Webp)
                    && self.file(&s.sticker).local.is_downloading_completed
                    && !self.animations.contains_key(&(s.sticker.id, false))
            })
            .cloned()
            .collect();
        for sticker in missing {
            self.ensure_animation(&sticker, false, cx);
        }
    }

    /// What the picker shows: search results, or up to 50 recents (ours
    /// first, then Telegram's recent and favorite stickers).
    fn picker_stickers(&self, cx: &App) -> Vec<&tdlib_rs::types::Sticker> {
        if !self.sticker_search.read(cx).value().trim().is_empty() {
            return self
                .sticker_results
                .iter()
                .flat_map(|(_, found)| found)
                .collect();
        }
        let mut seen = HashSet::new();
        self.sticker_history
            .iter()
            .chain(&self.stickers)
            .filter(|s| seen.insert(s.id))
            .take(STICKER_HISTORY)
            .collect()
    }

    /// Sends a sticker and moves it to the front of the local history.
    fn send_sticker(
        &mut self,
        chat_id: i64,
        sticker: tdlib_rs::types::Sticker,
        cx: &mut Context<Self>,
    ) {
        self.send(Command::SendSticker {
            chat_id,
            topic: self.topic_by_chat.get(&chat_id).copied(),
            file_id: sticker.sticker.id,
            width: sticker.width,
            height: sticker.height,
            emoji: sticker.emoji.clone(),
        });
        self.sticker_history.retain(|s| s.id != sticker.id);
        self.sticker_history.insert(0, sticker);
        self.sticker_history.truncate(STICKER_HISTORY);
        if !self.demo
            && let Err(error) = save_sticker_history(&self.sticker_history)
        {
            self.error = Some(format!("Could not save recent stickers: {error}"));
        }
        cx.notify();
    }

    fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.scroller
            .update(cx, |state, cx| state.scroll_to_end(cx));
    }

    fn cache_file(&mut self, file: tdlib_rs::types::File, cx: &mut Context<Self>) {
        if file.local.is_downloading_completed || !file.local.is_downloading_active {
            self.downloading.remove(&file.id);
        }
        let completed = file.local.is_downloading_completed;
        let id = file.id;
        if file.local.is_downloading_completed && self.open_when_ready.remove(&file.id) {
            open_file(&file.local.path, cx);
        }
        self.files.insert(file.id, file);
        if completed {
            self.resume_pending_play(id, cx);
        }
        if completed
            && let Some(sticker) = self
                .stickers
                .iter()
                .chain(&self.sticker_history)
                .chain(self.sticker_results.iter().flat_map(|(_, found)| found))
                .find(|s| s.sticker.id == id)
                .cloned()
        {
            self.ensure_animation(&sticker, false, cx);
        }
    }

    /// Starts decoding an animated (TGS) or video (WebM) sticker once its file
    /// is downloaded. `animate` renders every frame; otherwise only the first.
    fn ensure_animation(
        &mut self,
        sticker: &tdlib_rs::types::Sticker,
        animate: bool,
        cx: &mut Context<Self>,
    ) {
        let kind = match sticker.format {
            tdlib_rs::enums::StickerFormat::Tgs => crate::stickers::Kind::Lottie,
            tdlib_rs::enums::StickerFormat::Webm => crate::stickers::Kind::Video,
            tdlib_rs::enums::StickerFormat::Webp => return,
        };
        let key = (sticker.sticker.id, animate);
        if self.animations.contains_key(&key) {
            return;
        }
        let file = self.file(&sticker.sticker).clone();
        if !file.local.is_downloading_completed {
            self.request_file(&file, 16, true);
            return;
        }
        // Chat stickers are 168pt; ~1.4x keeps them crisp without huge frames.
        let longest = if animate { 240. } else { 128. };
        let (w, h) = (sticker.width.max(1) as f32, sticker.height.max(1) as f32);
        let scale = longest / w.max(h);
        let spec = crate::stickers::Spec {
            kind,
            path: PathBuf::from(&file.local.path),
            width: ((w * scale).round() as u32).max(1),
            height: ((h * scale).round() as u32).max(1),
            animate,
        };
        self.animations.insert(
            key,
            CachedAnim {
                anim: Anim::Loading,
                last_used: std::cell::Cell::new(self.animation_clock.get()),
            },
        );
        let decoded = cx
            .background_executor()
            .spawn(async move { crate::stickers::decode(&spec) });
        cx.spawn(async move |this, cx| {
            let result = decoded.await;
            let _ = this.update(cx, |this, cx| {
                let anim = match result {
                    Ok(frames) => Anim::Ready(frames),
                    Err(_) => Anim::Failed,
                };
                if let Some(entry) = this.animations.get_mut(&key) {
                    entry.anim = anim;
                }
                this.evict_animations(animate, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// The decoded animation for a sticker, if ready.
    fn animation(
        &self,
        sticker: &tdlib_rs::types::Sticker,
        animate: bool,
    ) -> Option<Arc<RenderImage>> {
        let entry = self.animations.get(&(sticker.sticker.id, animate))?;
        let Anim::Ready(frames) = &entry.anim else {
            return None;
        };
        let now = self.animation_clock.get() + 1;
        self.animation_clock.set(now);
        entry.last_used.set(now);
        let count = frames.images.len();
        let index = if count > 1 && self.animate_stickers {
            let elapsed = self.started.elapsed().as_millis();
            (elapsed / frames.delay.as_millis().max(1)) as usize % count
        } else {
            0
        };
        frames.images.get(index).cloned()
    }

    /// Called as an animated sticker renders: schedules one redraw for when
    /// its next frame is due. Stickers animate on this clock instead of
    /// GPUI's, which redraws the window at the display's refresh rate for
    /// multi-frame images.
    fn schedule_sticker_frame(
        &mut self,
        sticker: &tdlib_rs::types::Sticker,
        cx: &mut Context<Self>,
    ) {
        if !self.animate_stickers || self.frame_task.is_some() {
            return;
        }
        let Some(CachedAnim {
            anim: Anim::Ready(frames),
            ..
        }) = self.animations.get(&(sticker.sticker.id, true))
        else {
            return;
        };
        if frames.images.len() < 2 {
            return;
        }
        // Wake at the next frame boundary of the shared clock.
        let delay = frames.delay.as_millis().max(1);
        let into = self.started.elapsed().as_millis() % delay;
        let wait = Duration::from_millis((delay - into) as u64);
        self.frame_task = Some(cx.spawn(async move |this, cx| {
            smol::Timer::after(wait).await;
            let _ = this.update(cx, |this, cx| {
                this.frame_task = None;
                this.redraw_conversation(cx);
            });
        }));
    }

    /// Redraws only the conversation, for changes nothing else shows
    /// (sticker frames, playback progress): the other panes stay cached.
    fn redraw_conversation(&mut self, cx: &mut Context<Self>) {
        match &self.panes {
            Some(panes) => panes.conversation.update(cx, |_, cx| cx.notify()),
            None => cx.notify(),
        }
    }

    /// Drops the least recently shown animations of one kind past its limit,
    /// freeing their frames' GPU textures as well.
    fn evict_animations(&mut self, animate: bool, cx: &mut App) {
        let limit = if animate {
            MAX_ANIMATIONS
        } else {
            MAX_FIRST_FRAMES
        };
        loop {
            let ready = self.animations.iter().filter(|((_, kind), entry)| {
                *kind == animate && matches!(entry.anim, Anim::Ready(_))
            });
            if ready.clone().count() <= limit {
                break;
            }
            let Some(oldest) = ready
                .min_by_key(|(_, entry)| entry.last_used.get())
                .map(|(key, _)| *key)
            else {
                break;
            };
            if let Some(CachedAnim {
                anim: Anim::Ready(frames),
                ..
            }) = self.animations.remove(&oldest)
            {
                for image in frames.images {
                    cx.drop_image(image, None);
                }
            }
        }
    }

    fn file<'a>(&'a self, file: &'a tdlib_rs::types::File) -> &'a tdlib_rs::types::File {
        self.files.get(&file.id).unwrap_or(file)
    }

    /// Requests a download. `priority` is TDLib's 1–32, higher first.
    fn request_file(&mut self, file: &tdlib_rs::types::File, priority: i32, automatic: bool) {
        let current = self.files.get(&file.id).unwrap_or(file);
        if current.local.is_downloading_completed {
            self.files.insert(file.id, current.clone());
            return;
        }
        if !current.local.can_be_downloaded || self.downloading.contains(&file.id) {
            return;
        }
        if automatic && file.size.max(file.expected_size) > 10 * 1024 * 1024 {
            return;
        }
        self.files.entry(file.id).or_insert_with(|| file.clone());
        if self.send(Command::Download {
            file: file.id,
            priority,
        }) {
            self.downloading.insert(file.id);
        }
    }

    /// Opens a downloaded file, or downloads it and opens it when ready.
    fn open_media(&mut self, file: &tdlib_rs::types::File, cx: &mut Context<Self>) {
        let current = self.file(file).clone();
        if current.local.is_downloading_completed {
            open_file(&current.local.path, cx);
        } else {
            self.open_when_ready.insert(current.id);
            self.request_file(&current, 32, false);
        }
        cx.notify();
    }

    /// Requests images for custom emoji reactions not seen yet.
    fn request_custom_emoji(&mut self, info: Option<&tdlib_rs::types::MessageInteractionInfo>) {
        let Some(reactions) = info.and_then(|info| info.reactions.as_ref()) else {
            return;
        };
        let missing: Vec<i64> = reactions
            .reactions
            .iter()
            .filter_map(|r| match &r.r#type {
                tdlib_rs::enums::ReactionType::CustomEmoji(custom) => Some(custom.custom_emoji_id),
                _ => None,
            })
            .filter(|id| !self.custom_emoji.contains_key(id))
            .collect();
        if missing.is_empty() {
            return;
        }
        for id in &missing {
            self.custom_emoji.insert(*id, None);
        }
        self.send(Command::CustomEmoji(missing));
    }

    fn prepare_media(&mut self, message: &Message) {
        self.request_custom_emoji(message.interaction_info.as_ref());
        // Sender photos for group avatars.
        if let tdlib_rs::enums::MessageSender::User(sender) = &message.sender_id
            && let Some(photo) = self
                .users
                .get(&sender.user_id)
                .and_then(|u| u.profile_photo.as_ref())
                .map(|p| p.small.clone())
        {
            self.request_file(&photo, 6, true);
        }
        if message.self_destruct_type.is_some() {
            return;
        }
        use tdlib_rs::enums::MessageContent as C;
        if let Some((mini, key)) = minithumbnail(&message.content)
            && !self.minithumbs.contains_key(&key)
            && let Some(image) = decode_minithumbnail(&mini.data)
        {
            self.minithumbs.insert(key, Arc::new(image));
        }
        if let C::MessageSticker(m) = &message.content
            && !matches!(m.sticker.format, tdlib_rs::enums::StickerFormat::Webp)
        {
            let file = m.sticker.sticker.clone();
            self.request_file(&file, 16, true);
        }
        if let Some(thumb) = video_thumbnail(&message.content) {
            let thumb = thumb.clone();
            self.request_file(&thumb, 16, true);
        }
        if let Some((file, photo)) = media(&message.content) {
            self.files.entry(file.id).or_insert_with(|| file.clone());
            if photo {
                let file = file.clone();
                self.request_file(&file, 16, true);
            }
        }
    }

    fn save_draft_later(&mut self, id: i64, cx: &mut Context<Self>) {
        if matches!(
            self.compose_actions.get(&id),
            Some(ComposeAction::Edit { .. } | ComposeAction::Forward { .. })
        ) {
            return;
        }
        self.draft_tasks.insert(
            id,
            cx.spawn(async move |this, cx| {
                smol::Timer::after(Duration::from_millis(600)).await;
                let _ = this.update(cx, |this, cx| {
                    if let Some(input) = this.composers.get(&id) {
                        this.send(Command::Draft {
                            chat_id: id,
                            topic: this.topic_by_chat.get(&id).copied(),
                            text: input.read(cx).value().to_string(),
                        });
                    }
                });
            }),
        );
    }

    fn switch_list(&mut self, list: List, window: &mut Window, cx: &mut Context<Self>) {
        if self.list == list {
            return;
        }
        self.list = list;
        self.chats_loading = false;
        self.chats_end = false;
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.search_results = None;
        self.chat_scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.load_chats(cx);
    }

    fn choose_attachment(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Attach".into()),
        });
        cx.spawn_in(window, async move |this, cx| match paths.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.attachments.insert(id, path);
                        if let Some(input) = this.composers.get(&id) {
                            input.update(cx, |input, cx| input.focus(window, cx));
                        }
                        cx.notify();
                    });
                }
            }
            Ok(Err(error)) => {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.error = Some(error.to_string());
                    cx.notify();
                });
            }
            _ => {}
        })
        .detach();
    }

    fn cancel_compose_action(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ComposeAction::Edit { previous, .. }) = self.compose_actions.remove(&id)
            && let Some(input) = self.composers.get(&id)
        {
            input.update(cx, |input, cx| input.set_value(previous, window, cx));
        }
        self.attachments.remove(&id);
        cx.notify();
    }

    fn start_reply(&mut self, chat_id: i64, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_compose_action(chat_id, window, cx);
        self.compose_actions
            .insert(chat_id, ComposeAction::Reply(id));
        if let Some(input) = self.composers.get(&chat_id) {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn start_edit(&mut self, chat_id: i64, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.find_message(chat_id, id).map(|m| match &m.content {
            tdlib_rs::enums::MessageContent::MessageText(text) => ui::to_markdown(&text.text),
            tdlib_rs::enums::MessageContent::MessagePhoto(m) => ui::to_markdown(&m.caption),
            tdlib_rs::enums::MessageContent::MessageVideo(m) => ui::to_markdown(&m.caption),
            tdlib_rs::enums::MessageContent::MessageDocument(m) => ui::to_markdown(&m.caption),
            tdlib_rs::enums::MessageContent::MessageAnimation(m) => ui::to_markdown(&m.caption),
            tdlib_rs::enums::MessageContent::MessageAudio(m) => ui::to_markdown(&m.caption),
            other => message_text(other),
        });
        self.cancel_compose_action(chat_id, window, cx);
        if let Some(text) = text
            && let Some(input) = self.composers.get(&chat_id)
        {
            let previous = input.read(cx).value().to_string();
            self.compose_actions
                .insert(chat_id, ComposeAction::Edit { id, previous });
            input.update(cx, |input, cx| {
                input.set_value(text, window, cx);
                input.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn find_message(&self, chat_id: i64, id: i64) -> Option<&Message> {
        self.chats
            .get(&chat_id)
            .and_then(|c| c.messages.get(&id))
            .or_else(|| {
                self.message_results.as_ref().and_then(|(_, _, messages)| {
                    messages.iter().find(|m| m.chat_id == chat_id && m.id == id)
                })
            })
    }

    fn confirm_delete(&self, chat_id: i64, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::button::{Button, ButtonVariants};
        let this = cx.entity().downgrade();
        let everyone =
            self.chats
                .get(&chat_id)
                .map_or_else(String::new, |chat| match &chat.info.r#type {
                    tdlib_rs::enums::ChatType::Private(_) if self.is_self_chat(chat) => {
                        String::new()
                    }
                    tdlib_rs::enums::ChatType::Private(_) => {
                        format!(" It will also be deleted for {}.", chat.info.title)
                    }
                    _ => " It will be deleted for everyone in this chat.".into(),
                });
        window.open_dialog(cx, move |dialog, _, _| {
            let this = this.clone();
            dialog
                .title("Delete message")
                .w(px(380.))
                .child(div().text_color(ui::hsla(ui::SECONDARY)).child(format!(
                    "Are you sure you want to delete this message?{everyone}"
                )))
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("delete-cancel")
                                .ghost()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("delete-confirm")
                                .danger()
                                .label("Delete")
                                .on_click(move |_, window, cx| {
                                    let _ = this.update(cx, |this, cx| {
                                        this.send(Command::Delete {
                                            chat_id,
                                            message_id: id,
                                        });
                                        cx.notify();
                                    });
                                    window.close_dialog(cx);
                                }),
                        ),
                )
        });
    }

    fn visible_messages<'a>(&'a self, chat: &'a Chat) -> Vec<&'a Message> {
        if self.message_search_open
            && let Some((id, _, messages)) = &self.message_results
            && *id == chat.info.id
        {
            return messages.iter().rev().collect();
        }
        let topic = self.topic_by_chat.get(&chat.info.id);
        chat.messages
            .values()
            .filter(|m| {
                topic.is_none_or(|topic| {
                    matches!(&m.topic_id, Some(tdlib_rs::enums::MessageTopic::Forum(t)) if t.forum_topic_id == *topic)
                })
            })
            .collect()
    }

    fn select_topic(
        &mut self,
        chat_id: i64,
        topic: Option<i32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sending.contains(&chat_id) {
            return;
        }
        let previous = self.topic_by_chat.get(&chat_id).copied();
        if previous == topic {
            return;
        }
        self.cancel_compose_action(chat_id, window, cx);
        self.draft_tasks.remove(&chat_id);
        if let Some(input) = self.composers.get(&chat_id) {
            let text = input.read(cx).value().to_string();
            self.send(Command::Draft {
                chat_id,
                topic: previous,
                text: text.clone(),
            });
            self.topic_text.insert((chat_id, previous), text);
        }
        if let Some(topic) = topic {
            self.topic_by_chat.insert(chat_id, topic);
        } else {
            self.topic_by_chat.remove(&chat_id);
        }
        let text = self
            .topic_text
            .get(&(chat_id, topic))
            .cloned()
            .unwrap_or_else(|| {
                self.topics
                    .get(&chat_id)
                    .and_then(|topics| topics.iter().find(|t| Some(t.info.forum_topic_id) == topic))
                    .and_then(|t| t.draft_message.as_ref())
                    .and_then(|d| match &d.input_message_text {
                        tdlib_rs::enums::InputMessageContent::InputMessageText(t) => {
                            Some(t.text.text.clone())
                        }
                        _ => None,
                    })
                    .unwrap_or_default()
            });
        if let Some(input) = self.composers.get(&chat_id) {
            input.update(cx, |input, cx| input.set_value(text, window, cx));
        }
        self.message_search_open = false;
        self.message_results = None;
        if let Some(topic) = topic {
            self.topic_end.remove(&(chat_id, topic));
            if self.send(Command::TopicHistory {
                chat_id,
                topic,
                from: 0,
            }) {
                self.topic_loading.insert((chat_id, topic));
            }
        } else {
            self.load_history(chat_id, cx);
        }
        cx.notify();
    }

    fn is_self_chat(&self, chat: &Chat) -> bool {
        matches!(&chat.info.r#type, tdlib_rs::enums::ChatType::Private(user) if user.user_id == self.me && self.me != 0)
    }

    fn chat_title(&self, chat: &Chat) -> String {
        if self.is_self_chat(chat) {
            "Saved Messages".into()
        } else {
            chat.info.title.clone()
        }
    }

    fn group(&self, chat: &Chat) -> Option<&Group> {
        match &chat.info.r#type {
            tdlib_rs::enums::ChatType::Supergroup(group) => self.groups.get(&group.supergroup_id),
            tdlib_rs::enums::ChatType::BasicGroup(group) => self.groups.get(&group.basic_group_id),
            _ => None,
        }
    }

    fn is_channel(&self, chat: &Chat) -> bool {
        matches!(&chat.info.r#type, tdlib_rs::enums::ChatType::Supergroup(group) if group.is_channel)
    }

    /// Active "typing…" text for a chat, if any.
    fn typing_label(&self, chat: &Chat) -> Option<String> {
        let entries: Vec<_> = self
            .typing
            .get(&chat.info.id)?
            .iter()
            .filter(|(_, _, at)| at.elapsed() < Duration::from_secs(6))
            .collect();
        let (sender, action, _) = entries.first()?;
        if matches!(chat.info.r#type, tdlib_rs::enums::ChatType::Private(_)) {
            return Some(format!("{action}…"));
        }
        let name = self
            .users
            .get(sender)
            .map_or_else(|| "Someone".into(), |u| u.first_name.clone());
        Some(if entries.len() > 1 {
            format!("{name} and {} more are typing…", entries.len() - 1)
        } else {
            format!("{name} is {action}…")
        })
    }

    /// Header status: "online", "last seen…", "N members", "typing…".
    fn chat_status(&self, chat: &Chat) -> (String, bool) {
        if let Some(typing) = self.typing_label(chat) {
            return (typing, true);
        }
        if self.is_self_chat(chat) {
            return (String::new(), false);
        }
        match &chat.info.r#type {
            tdlib_rs::enums::ChatType::Private(private) => match self.users.get(&private.user_id) {
                Some(user) if matches!(user.r#type, tdlib_rs::enums::UserType::Bot(_)) => {
                    ("bot".into(), false)
                }
                Some(user) if user.id == 777000 => ("service notifications".into(), false),
                Some(user) => ui::user_status(&user.status),
                None => (String::new(), false),
            },
            _ => match self.group(chat) {
                Some(group) if group.channel => (
                    ui::count_label(group.members, "subscriber", "subscribers"),
                    false,
                ),
                Some(group) => (ui::count_label(group.members, "member", "members"), false),
                None => self
                    .profiles
                    .get(&chat.info.id)
                    .and_then(|p| p.members)
                    .map_or((String::new(), false), |members| {
                        (ui::count_label(members, "member", "members"), false)
                    }),
            },
        }
    }

    fn sender_id(message: &Message) -> i64 {
        match &message.sender_id {
            tdlib_rs::enums::MessageSender::User(sender) => sender.user_id,
            tdlib_rs::enums::MessageSender::Chat(sender) => sender.chat_id,
        }
    }

    fn sender_name(&self, message: &Message) -> String {
        match &message.sender_id {
            tdlib_rs::enums::MessageSender::User(sender) => self.user_name(sender.user_id),
            tdlib_rs::enums::MessageSender::Chat(sender) => self
                .chats
                .get(&sender.chat_id)
                .map_or_else(|| "Channel".into(), |c| c.info.title.clone()),
        }
    }

    fn user_name(&self, id: i64) -> String {
        self.users.get(&id).map_or_else(
            || "Deleted Account".into(),
            |u| {
                format!("{} {}", u.first_name, u.last_name)
                    .trim()
                    .to_owned()
            },
        )
    }

    fn origin_name(&self, origin: &tdlib_rs::enums::MessageOrigin) -> String {
        use tdlib_rs::enums::MessageOrigin;
        match origin {
            MessageOrigin::User(user) => self.user_name(user.sender_user_id),
            MessageOrigin::HiddenUser(user) => user.sender_name.clone(),
            MessageOrigin::Chat(chat) => self
                .chats
                .get(&chat.sender_chat_id)
                .map_or_else(|| "Chat".into(), |c| c.info.title.clone()),
            MessageOrigin::Channel(channel) => self
                .chats
                .get(&channel.chat_id)
                .map_or_else(|| "Channel".into(), |c| c.info.title.clone()),
        }
    }

    /// Marks the newest visible message as read while the user is at the bottom.
    fn mark_read(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !window.is_window_active() || self.message_search_open {
            return;
        }
        let Some(id) = self.selected else {
            return;
        };
        if self.scroller.read(cx).is_scrolled_up() {
            return;
        }
        let last = self.chats.get(&id).and_then(|chat| {
            if let Some(topic_id) = self.topic_by_chat.get(&id) {
                let topic = self
                    .topics
                    .get(&id)?
                    .iter()
                    .find(|t| t.info.forum_topic_id == *topic_id)?;
                let last = self.visible_messages(chat).last().copied()?;
                return (topic.unread_count > 0
                    && topic.last_message.as_ref().is_some_and(|m| m.id == last.id))
                .then_some(last.id);
            }
            chat.messages
                .last_key_value()
                .filter(|(message_id, _)| {
                    chat.info.unread_count > 0
                        && chat
                            .info
                            .last_message
                            .as_ref()
                            .is_some_and(|m| m.id == **message_id)
                })
                .map(|(&message_id, _)| message_id)
        });
        if let Some(message_id) = last
            && self.read_requested.get(&id) != Some(&message_id)
            && self.send(Command::Read {
                chat_id: id,
                message_id,
            })
        {
            self.read_requested.insert(id, message_id);
        }
    }
}

impl Render for Messenger {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Every `img()` below loads through the bounded cache.
        image_cache(self.image_cache.clone())
            .size_full()
            .child(self.root(window, cx))
    }
}

impl Messenger {
    fn root(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        self.animate_stickers = window.is_window_active() && !cx.reduce_motion();
        if let Some(error) = self.error.take() {
            window.push_notification(Notification::error(error), cx);
        }
        let base = div()
            .relative()
            .size_full()
            .bg(ui::hsla(ui::WINDOW))
            .text_color(ui::hsla(ui::TEXT))
            .font_family(".SystemUIFont")
            .text_size(px(14.))
            .child(pointer_exit_watcher());
        if self.login != Login::Ready {
            return base
                .child(self.login_view(cx))
                .child(drag_strip("login-drag"));
        }
        self.mark_read(window, cx);
        self.sync_rows(cx);
        if self.sticker_open && self.selected.is_some() {
            self.ensure_picker_frames(cx);
        }
        let wide = window.bounds().size.width >= px(1180.);
        let palette = self.palette_open.then(|| self.palette(cx));
        let menu = self.render_menu(cx);
        let photo = self.render_viewer(window, cx);
        let entity = cx.entity();
        let panes = self
            .panes
            .get_or_insert_with(|| pane::Panes::new(&entity, cx));
        let (sidebar, conversation, side) = (
            panes.sidebar.clone(),
            panes.conversation.clone(),
            panes.side.clone(),
        );
        // Sidebar sits on the window backdrop; conversation and profile are
        // inset rounded panels beside it.
        base.flex()
            .child(
                sidebar.cached(
                    StyleRefinement::default()
                        .w(sidebar_width(window))
                        .flex_shrink_0()
                        .h_full(),
                ),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .gap(px(ui::INSET))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .py(px(ui::INSET))
                    .pr(px(ui::INSET))
                    .child(
                        panel()
                            .flex_1()
                            .min_w_0()
                            .child(conversation.cached(StyleRefinement::default().size_full())),
                    )
                    .when(
                        (self.details || self.sticker_open) && self.selected.is_some(),
                        |el| {
                            let details = panel()
                                .w(px(340.))
                                .flex_shrink_0()
                                .child(side.cached(StyleRefinement::default().size_full()));
                            if wide {
                                el.child(details)
                            } else {
                                el.child(
                                    div()
                                        .absolute()
                                        .top(px(ui::INSET))
                                        .bottom(px(ui::INSET))
                                        .right(px(ui::INSET))
                                        .child(details.h_full().shadow_2xl()),
                                )
                            }
                        },
                    ),
            )
            .children(palette)
            .children(photo)
            .children(menu)
    }
}

/// A transfer progress report, as opposed to a change other panes show.
fn is_progress(event: &Event) -> bool {
    matches!(event, Event::Update(update) if matches!(&**update, Update::File(file)
        if file.file.local.is_downloading_active || file.file.remote.is_uploading_active))
}

/// GPUI keeps the last pointer position when the pointer leaves the window,
/// so the hover style and tooltip of whatever it left stay on until the
/// pointer returns. On exit, report a move to just outside the window
/// instead, which clears them like any other move would.
fn pointer_exit_watcher() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |_, _, window, _| {
            window.on_mouse_event(|_: &MouseExitEvent, phase, window, cx| {
                if !phase.bubble() {
                    return;
                }
                let handle = window.window_handle();
                let modifiers = window.modifiers();
                // Not from inside this dispatch: queue it.
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        window.dispatch_event(
                            PlatformInput::MouseMove(MouseMoveEvent {
                                position: point(px(-1.), px(-1.)),
                                pressed_button: None,
                                modifiers,
                            }),
                            cx,
                        );
                    });
                });
            });
        },
    )
    .absolute()
    .size_0()
}

/// The sidebar is narrower in small windows.
fn sidebar_width(window: &Window) -> Pixels {
    if window.bounds().size.width >= px(1100.) {
        px(360.)
    } else {
        px(300.)
    }
}

/// A media message's blurry preview and the file id it stands in for.
fn minithumbnail(
    content: &tdlib_rs::enums::MessageContent,
) -> Option<(&tdlib_rs::types::Minithumbnail, i32)> {
    use tdlib_rs::enums::MessageContent as C;
    match content {
        C::MessagePhoto(m) => Some((
            m.photo.minithumbnail.as_ref()?,
            crate::model::photo_size(&m.photo)?.photo.id,
        )),
        C::MessageVideo(m) => Some((m.video.minithumbnail.as_ref()?, m.video.video.id)),
        C::MessageAnimation(m) => Some((
            m.animation.minithumbnail.as_ref()?,
            m.animation.animation.id,
        )),
        C::MessageVideoNote(m) => {
            Some((m.video_note.minithumbnail.as_ref()?, m.video_note.video.id))
        }
        _ => None,
    }
}

/// Decodes a minithumbnail (a tiny JPEG) into an image we own, so it can be
/// freed with its message instead of living in GPUI's global asset cache.
fn decode_minithumbnail(data: &str) -> Option<RenderImage> {
    let mut pixels =
        image::load_from_memory_with_format(&ui::base64(data), image::ImageFormat::Jpeg)
            .ok()?
            .to_rgba8();
    // GPUI wants BGRA.
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(RenderImage::new(vec![image::Frame::new(pixels)]))
}

/// Rounded, bordered panel inset on the window backdrop.
fn panel() -> Div {
    div()
        .relative()
        .h_full()
        .flex()
        .rounded(px(ui::RADIUS_PANEL))
        .bg(ui::hsla(ui::CANVAS))
        .border_1()
        .border_color(ui::hsla(ui::STROKE))
}

fn is_light(appearance: WindowAppearance) -> bool {
    if let Some(light) = crate::demo::theme() {
        return light;
    }
    matches!(
        appearance,
        WindowAppearance::Light | WindowAppearance::VibrantLight
    )
}

/// A transparent strip that lets the window be dragged by its top edge.
fn drag_strip(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(36.))
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count >= 2 {
                window.titlebar_double_click();
            } else {
                window.start_window_move();
            }
        })
}

fn open_file(path: &str, cx: &mut App) {
    if let Ok(url) = url::Url::from_file_path(path) {
        cx.open_url(url.as_str());
    }
}

fn sticker_file(sticker: &tdlib_rs::types::Sticker) -> Option<&tdlib_rs::types::File> {
    crate::model::sticker_file(sticker)
}

/// JPEG preview frame for videos and GIFs.
fn video_thumbnail(content: &tdlib_rs::enums::MessageContent) -> Option<&tdlib_rs::types::File> {
    use tdlib_rs::enums::{MessageContent as C, ThumbnailFormat};
    let thumbnail = match content {
        C::MessageVideo(m) => m.video.thumbnail.as_ref(),
        C::MessageAnimation(m) => m.animation.thumbnail.as_ref(),
        C::MessageVideoNote(m) => m.video_note.thumbnail.as_ref(),
        _ => None,
    }?;
    matches!(
        thumbnail.format,
        ThumbnailFormat::Jpeg | ThumbnailFormat::Png
    )
    .then_some(&thumbnail.file)
}

/// Square-grid preview file for the shared media panel.
fn grid_file(message: &Message) -> Option<&tdlib_rs::types::File> {
    match &message.content {
        tdlib_rs::enums::MessageContent::MessagePhoto(m) => {
            crate::model::photo_thumb(&m.photo).map(|s| &s.photo)
        }
        other => video_thumbnail(other),
    }
}

fn chat_action(action: &tdlib_rs::enums::ChatAction) -> Option<&'static str> {
    use tdlib_rs::enums::ChatAction;
    Some(match action {
        ChatAction::Typing => "typing",
        ChatAction::RecordingVoiceNote => "recording a voice message",
        ChatAction::UploadingVoiceNote(_) => "sending a voice message",
        ChatAction::RecordingVideo | ChatAction::RecordingVideoNote => "recording a video",
        ChatAction::UploadingPhoto(_) => "sending a photo",
        ChatAction::UploadingVideo(_) | ChatAction::UploadingVideoNote(_) => "sending a video",
        ChatAction::UploadingDocument(_) => "sending a file",
        ChatAction::ChoosingSticker => "choosing a sticker",
        _ => return None,
    })
}

fn code_delivery(kind: &AuthenticationCodeType) -> String {
    match kind {
        AuthenticationCodeType::TelegramMessage(_) => "Telegram".into(),
        AuthenticationCodeType::Sms(_) => "SMS".into(),
        AuthenticationCodeType::SmsWord(_) => "a word sent by SMS".into(),
        AuthenticationCodeType::SmsPhrase(_) => "a phrase sent by SMS".into(),
        AuthenticationCodeType::Call(_) => "a voice call".into(),
        AuthenticationCodeType::FlashCall(code) => {
            format!("a flash call matching {}", code.pattern)
        }
        AuthenticationCodeType::MissedCall(code) => format!(
            "a missed call from {} (enter the last {} digits)",
            code.phone_number_prefix, code.length
        ),
        AuthenticationCodeType::Fragment(code) => format!("Fragment at {}", code.url),
        AuthenticationCodeType::FirebaseAndroid(_) | AuthenticationCodeType::FirebaseIos(_) => {
            "mobile-app SMS verification (unsupported in this client)".into()
        }
    }
}
