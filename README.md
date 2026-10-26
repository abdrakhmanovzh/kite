# Telegram for macOS

A personal Telegram client in Rust and GPUI, following the dark graphite design in `design/reference.png`.

## Current stage

Implemented: QR/phone/password/email login and saved sessions; main chat list, archive and existing folders; server-backed chat search; history pagination and message search; sender names, timestamps and read/sending/failed indicators; avatars and inline photos; file downloads and opening audio/video in the macOS player; photo/document attachments; text sending, replies, edits, forwarding, emoji reactions and retrying eligible failures; confirmed message deletion; synced text drafts; profile details; pin, mute and archive controls; optional macOS notifications; Saved Messages; installed/recent/favorite sticker browsing and sending; poll creation, voting, multiple answers, retraction and results; forum topic browsing, pagination, history, search, sending and text drafts.

Right-click a message for Reply, Edit, Copy, Forward, React and Delete; double-click a message to reply. The paperclip menu attaches a photo or file, or creates a poll (one answer per line). The sticker button opens recent/favorite stickers and your installed packs; clicking a sticker sends it. Forum groups show their topics as chips under the header. Enter sends, Shift+Enter inserts a newline. Attachments are staged until you press Send. JPEG/PNG files send as photos; other files send as documents. Desktop notifications are enabled from the menu next to search. Click the chat header to open its profile panel with shared media. Downloads use TDLib’s local cache.

This is a core messaging client, not complete Telegram feature parity. Calls and voice recording are intentionally out of scope. GIF picking, stories, account/group administration, scheduled messages, and secret chats are not implemented. Animated (TGS) stickers are rendered with `rasterlottie` and video (WebM) stickers with ffmpeg; sticker pack installation is not implemented. Poll creation supports regular polls; existing quizzes can be answered. Topic administration is not implemented. Message search currently returns the latest 100 matches.

Formatting: type Telegram markdown (`**bold**`, `__italic__`, `` `mono` ``, ```` ``` ```` code blocks, `~~strike~~`, `||spoiler||`, `[text](url)`) or use ⌘B, ⌘I, ⇧⌘M, ⇧⌘X, ⇧⌘P, ⇧⌘C and the formatting menu in the composer. Voice messages, audio files and round video messages play inside the app (voice notes are converted with ffmpeg, which must be installed, e.g. via Homebrew); other videos open in the system player. Photos open in the built-in viewer (←/→ to browse, Esc to close). The theme follows the macOS light/dark setting. Opening a chat with unread messages starts at an "Unread messages" divider, and the pinned-message bar under the header jumps through pins.

Draft text syncs, while staged attachments and edit/forward state last only for the current app session.

## Build and run

Use a current stable Rust toolchain and Apple Command Line Tools. GPUI Kit 0.7.3 enables runtime Metal shader compilation; full Xcode is not required.

```sh
cargo fetch --target aarch64-apple-darwin
cargo run --locked
```

`tdlib-rs` 1.4.0 downloads its matching native TDLib archive for macOS Apple Silicon during its build. Static linking includes TDLib in the app; no Homebrew TDLib installation is required. The first build needs access to crates.io and the wrapper maintainer's GitHub release assets. `cargo fetch` downloads Rust crates, while the native archive is downloaded by the build script.

`cargo run` uses an optimized dev profile (dependencies at `opt-level = 3`, the app at `1`); unoptimized GPUI and image decoding are far too slow to judge. To create a release app bundle:

```sh
sh scripts/bundle.sh
open "target/Telegram GPUI.app"
```

The connected build has been compiled and launched on macOS 14.7 with Rust 1.99.1. Formatting and Clippy pass; the credentials screen and empty-input validation were checked in the native app. QR login succeeded on the user’s account, and real chats and history were observed in the native app. A self-check message was sent successfully to Saved Messages and found with in-chat search. Other mutating actions and notification delivery still require manual validation.

## Demo mode

`TG_DEMO=1 cargo run` renders the interface from built-in seed data (`src/demo.rs`, `assets/demo/`) for design work and screenshots. It never starts TDLib or reads or writes the session directory. `TG_DEMO_SCENE` picks the first screen: `chat` (default), `group`, `channel`, `details`, `nostickers`, `palette`, `search`, `viewer`, `avatar`, `playing`, `archive`, `list` or `login`. `TG_DEMO_THEME=light|dark` overrides the system appearance. Actions that would talk to Telegram do nothing.

`src/demo/defaults.json` holds complete default objects for the TDLib types the seed builds; regenerate it after a `tdlib-rs` upgrade:

```sh
python3 scripts/demo_defaults.py $(find target/debug/build -path "*tdlib-rs*" -name generated.rs | head -1) > src/demo/defaults.json
```

## Sign in

Create your application at https://my.telegram.org under **API development tools**. Enter the resulting API ID and API hash directly into the app, then choose **Log in with QR code** or follow the phone/code/password prompts. For QR login, open Telegram on your phone → Settings → Devices → Link Desktop Device. QR codes are generated locally using macOS Core Image and refresh with TDLib authorization updates. This build only signs in to existing accounts; it doesn't create accounts or make purchases.

The application credentials and TDLib session/database are saved under `~/Library/Application Support/Telegram GPUI`. The directory is created with owner-only access and the credentials file with mode 0600. The TDLib database has no additional encryption key in this build. Verification codes and passwords are not written into application settings. Treat the session directory as private account data.

## Checks

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
```

No automated tests are added. Manual checks: invalid credentials, phone/code/2FA login, restart with the saved session, chat search, history pagination, incoming updates, connection loss, details panel, and resizing. Live-account validation requires signing in yourself; implementation does not imply those checks have passed.

Keep one crate, direct modules, and typed TDLib messages. Resource profiling follows a reliable daily-use app.
