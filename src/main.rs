mod demo;
mod image_cache;
mod media;
mod model;
mod notifications;
mod qr;
mod shell;
mod stickers;
mod telegram;
mod ui;

use gpui_kit::{prelude::*, *};

actions!(telegram, [Quit]);

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx| {
            gpui_kit::init(cx);
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
                    title: Some("Telegram".into()),
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
                eprintln!("Could not open Telegram: {error:#}");
                cx.quit();
            }
            cx.activate(true);
        });
}
