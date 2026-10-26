mod demo;
mod image_cache;
mod media;
mod model;
mod notifications;
mod opus;
mod paths;
mod qr;
mod shell;
mod stickers;
mod telegram;
mod tray;
mod ui;
mod update;
mod webm;

use gpui_kit::{prelude::*, *};

actions!(kite, [Quit, CloseWindow]);

fn main() {
    // The demo renders seed data and never touches the user's folder.
    if !demo::enabled() {
        paths::migrate_legacy();
    }
    let app = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    // Clicking the Dock icon brings a closed (hidden) window back.
    app.on_reopen(|cx| cx.activate(true));
    app.run(|cx| {
        gpui_kit::init(cx);
        set_dock_icon();
        // Start dark; the window switches to the system appearance once open.
        ui::apply_theme(false, cx);
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-w", CloseWindow, None),
        ]);
        // Closing the window keeps Kite running, like Telegram: it hides,
        // and the Dock or menu bar item brings it back.
        cx.on_action(|_: &CloseWindow, cx| cx.hide());
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1280.), px(860.)),
                cx,
            ))),
            window_min_size: Some(size(px(860.), px(600.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Kite".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(16.), px(16.))),
            }),
            ..Default::default()
        };
        if let Err(error) = gpui_kit::open_window(options, cx, |window, cx| {
            let view = cx.new(|cx| shell::Messenger::new(window, cx));
            let weak = view.downgrade();
            cx.on_action(move |_: &Quit, cx| {
                let _ = weak.update(cx, |view, cx| view.quit(cx));
            });
            // The red button closes the same way as Cmd+W.
            window.on_window_should_close(cx, |_, cx| {
                cx.hide();
                false
            });
            start_tray(view.downgrade(), cx);
            view
        }) {
            eprintln!("Could not open Kite: {error:#}");
            cx.quit();
        }
        cx.activate(true);
    });
}

/// Adds the menu bar item and handles its menu.
fn start_tray(view: WeakEntity<shell::Messenger>, cx: &mut App) {
    let (events, received) = async_channel::unbounded();
    let Some(tray) = tray::Tray::new(events) else {
        return;
    };
    cx.set_global(tray);
    // A count may have arrived before the item existed.
    let _ = view.update(cx, |view, cx| view.sync_tray(cx));
    cx.spawn(async move |cx| {
        while let Ok(event) = received.recv().await {
            let quit = matches!(event, tray::TrayEvent::Quit);
            cx.update(|cx| {
                if quit {
                    let _ = view.update(cx, |view, cx| view.quit(cx));
                } else {
                    cx.activate(true);
                }
            });
        }
    })
    .detach();
}

/// The app bundle sets the Dock icon from its Info.plist; a bare binary
/// (cargo run, scripts/dev.sh) would show a generic one, so set it here too.
fn set_dock_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(include_bytes!("../assets/icon/icon.png"));
    if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
        // SAFETY: AppKit call on the main thread with a valid image.
        unsafe {
            NSApplication::sharedApplication(main_thread).setApplicationIconImage(Some(&image));
        }
    }
}
