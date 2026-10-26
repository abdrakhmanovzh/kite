mod demo;
mod image_cache;
mod media;
mod model;
mod notifications;
mod paths;
mod qr;
mod shell;
mod stickers;
mod telegram;
mod ui;

use gpui_kit::{prelude::*, *};

actions!(kite, [Quit]);

fn main() {
    // The demo renders seed data and never touches the user's folder.
    if !demo::enabled() {
        paths::migrate_legacy();
    }
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            set_dock_icon();
            // Start dark; the window switches to the system appearance once open.
            ui::apply_theme(false, cx);
            cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
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
                let weak = view.downgrade();
                window.on_window_should_close(cx, move |_, cx| {
                    let _ = weak.update(cx, |view, cx| view.quit(cx));
                    false
                });
                view
            }) {
                eprintln!("Could not open Kite: {error:#}");
                cx.quit();
            }
            cx.activate(true);
        });
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
