# Design verification

Reference: `design/reference.png`, the selected dark graphite mockup.

The connected app builds and launches on macOS 14.7 with Rust 1.99.1 and Command Line Tools, without full Xcode. Formatting and Clippy with warnings denied pass. No automated tests were added.

Verified in the native app: empty-credential validation, QR rendering and refresh, successful QR login (confirmed by the user), authenticated restart, real chat lists, avatars, existing folder tabs, chat search, message history, sender names, timestamps, and message-action controls. One harmless self-check message was sent to Saved Messages and found with in-chat search. No messages were sent to other people.

Implemented but not yet validated end to end against the live account: replies, edits, forwarding, reactions, deletion, uploads, notification permission/delivery, server permission failures, and connection-loss recovery. These must not be described as verified merely because they compile. Full Telegram feature parity is not implemented; see README for the scope.

Native screenshots sometimes retain a previous composited frame until resizing while accessibility state updates. Visual inspection confirmed the graphite layout and rounded avatars, but final user visual sign-off remains pending.

Sticker/poll/topic increment: offline compilation and Clippy pass. In the native app, the Stickers button loaded the account’s installed packs. The poll creation form and empty-poll validation were visibly checked; the invalid form did not send a poll. Poll creation and voting, sticker transmission, and forum-topic server mutations are implemented but have not been exercised against other people’s chats. No automated tests or dependencies were added for this increment.
