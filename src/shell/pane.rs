//! The window's three regions as cached views.
//!
//! GPUI re-renders everything under a dirty view on every frame, and only
//! skips subtrees wrapped in a cached view that hasn't changed. All state
//! lives in `Messenger`, so each pane re-renders whenever it notifies; but
//! frequent redraws that only touch the conversation (sticker frames,
//! playback progress) notify the conversation pane alone, and the sidebar
//! and side panel are replayed from cache.

use gpui_kit::{prelude::*, *};

use super::Messenger;

#[derive(Clone, Copy)]
pub(super) enum PaneKind {
    Sidebar,
    Conversation,
    /// Stickers or the profile, beside the conversation.
    Side,
}

pub(super) struct Pane {
    app: WeakEntity<Messenger>,
    kind: PaneKind,
    _changes: Subscription,
}

impl Pane {
    pub(super) fn new(app: &Entity<Messenger>, kind: PaneKind, cx: &mut Context<Self>) -> Self {
        Self {
            app: app.downgrade(),
            kind,
            _changes: cx.observe(app, |_, _, cx| cx.notify()),
        }
    }
}

impl Render for Pane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self.kind;
        // A cached view lays out as its own root: without an explicit size
        // its content would size itself to the widest line.
        let content = self
            .app
            .update(cx, |app, cx| match kind {
                PaneKind::Sidebar => {
                    let width = super::sidebar_width(window);
                    app.sidebar(width, cx).into_any_element()
                }
                PaneKind::Conversation => app.conversation(window, cx),
                PaneKind::Side if app.sticker_open => app.sticker_panel(cx).into_any_element(),
                PaneKind::Side => app.detail_panel(cx),
            })
            .unwrap_or_else(|_| div().into_any_element());
        div().size_full().flex().child(content)
    }
}

/// Handles to the panes, created once the app entity exists.
pub(super) struct Panes {
    pub sidebar: Entity<Pane>,
    pub conversation: Entity<Pane>,
    pub side: Entity<Pane>,
}

impl Panes {
    pub(super) fn new(app: &Entity<Messenger>, cx: &mut App) -> Self {
        Self {
            sidebar: cx.new(|cx| Pane::new(app, PaneKind::Sidebar, cx)),
            conversation: cx.new(|cx| Pane::new(app, PaneKind::Conversation, cx)),
            side: cx.new(|cx| Pane::new(app, PaneKind::Side, cx)),
        }
    }
}
