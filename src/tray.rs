//! The menu bar item, like Telegram's: Kite keeps running (and receiving
//! messages and notifications) after its window is closed. Clicking the
//! item brings the window back; a secondary click (right click, two-finger
//! click or control-click) shows its menu.

use async_channel::Sender;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained, runtime::AnyObject, sel,
};
use std::cell::OnceCell;

use objc2_app_kit::{
    NSApplication, NSEventMask, NSEventModifierFlags, NSEventType, NSImage, NSMenu, NSMenuItem,
    NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{NSData, NSObject, NSSize, NSString};

/// What the menu bar item asks the app to do.
pub enum TrayEvent {
    Open,
    Quit,
}

struct Ivars {
    events: Sender<TrayEvent>,
    /// Set once built; the item's button targets this object weakly, so
    /// holding the item here makes no cycle.
    item: OnceCell<Retained<NSStatusItem>>,
    menu: OnceCell<Retained<NSMenu>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the class
    // doesn't implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "KiteTrayTarget"]
    #[ivars = Ivars]
    struct Target;

    impl Target {
        /// The item's button: open on a primary click, menu on a secondary.
        #[unsafe(method(clicked:))]
        fn clicked(&self, _sender: Option<&AnyObject>) {
            let secondary = NSApplication::sharedApplication(self.mtm())
                .currentEvent()
                .is_some_and(|event| {
                    event.r#type() == NSEventType::RightMouseUp
                        || event.modifierFlags().contains(NSEventModifierFlags::Control)
                });
            if !secondary {
                let _ = self.ivars().events.try_send(TrayEvent::Open);
                return;
            }
            let (Some(item), Some(menu)) = (self.ivars().item.get(), self.ivars().menu.get())
            else {
                return;
            };
            // Attach the menu just for this click: with a menu set, the
            // button shows it instead of sending its action.
            item.setMenu(Some(menu));
            if let Some(button) = item.button(self.mtm()) {
                // SAFETY: Simulates a click on our own status bar button.
                unsafe { button.performClick(None) };
            }
            item.setMenu(None);
        }

        #[unsafe(method(open:))]
        fn open(&self, _sender: Option<&AnyObject>) {
            let _ = self.ivars().events.try_send(TrayEvent::Open);
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            let _ = self.ivars().events.try_send(TrayEvent::Quit);
        }
    }
);

/// Keeps the menu bar item and its menu target alive; dropping it removes
/// the item.
pub struct Tray {
    _item: Retained<NSStatusItem>,
    _target: Retained<Target>,
}

impl gpui_kit::Global for Tray {}

impl Tray {
    /// Adds the item to the menu bar. Returns None off the main thread.
    pub fn new(events: Sender<TrayEvent>) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let target = mtm.alloc::<Target>().set_ivars(Ivars {
            events,
            item: OnceCell::new(),
            menu: OnceCell::new(),
        });
        // SAFETY: `init` on a freshly allocated NSObject subclass.
        let target: Retained<Target> = unsafe { msg_send![super(target), init] };

        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        if let Some(button) = item.button(mtm) {
            let data = NSData::with_bytes(include_bytes!("../assets/icon/tray.png"));
            if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
                // A template image: macOS tints it for light and dark menu bars.
                image.setSize(NSSize::new(22., 22.));
                image.setTemplate(true);
                button.setImage(Some(&image));
            }
            button.setToolTip(Some(&NSString::from_str("Kite")));
            // SAFETY: The target responds to `clicked:` and outlives the
            // button (both are held by `Tray`).
            unsafe {
                button.setTarget(Some(&target));
                button.setAction(Some(sel!(clicked:)));
            }
            button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
        }

        let menu = NSMenu::new(mtm);
        let entry = |title: &str, action, key: &str| {
            // SAFETY: Standard menu item initializer; the target outlives it
            // (both are held by `Tray`).
            let entry = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    Some(action),
                    &NSString::from_str(key),
                )
            };
            // SAFETY: The target responds to the item's action.
            unsafe { entry.setTarget(Some(&target)) };
            entry
        };
        menu.addItem(&entry("Open Kite", sel!(open:), ""));
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        menu.addItem(&entry("Quit Kite", sel!(quit:), "q"));
        let _ = target.ivars().menu.set(menu);
        let _ = target.ivars().item.set(item.clone());

        Some(Self {
            _item: item,
            _target: target,
        })
    }
}
