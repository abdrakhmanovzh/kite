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

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_app_kit::{
    NSApplication, NSBezierPath, NSColor, NSCompositingOperation, NSEventMask,
    NSEventModifierFlags, NSEventType, NSFont, NSFontAttributeName, NSForegroundColorAttributeName,
    NSImage, NSMenu, NSMenuItem, NSRectFillUsingOperation, NSStatusBar, NSStatusBarButton,
    NSStatusItem, NSStringDrawing, NSVariableStatusItemLength,
};
use objc2_foundation::{NSData, NSObject, NSSize, NSString};
use objc2_foundation::{NSDictionary, NSPoint, NSRect};

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
    button: Option<Retained<NSStatusBarButton>>,
    /// The kite alone, as a template image.
    kite: Option<Retained<NSImage>>,
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
        let data = NSData::with_bytes(include_bytes!("../assets/icon/tray.png"));
        let kite = NSImage::initWithData(NSImage::alloc(), &data);
        if let Some(kite) = &kite {
            // A template image: macOS tints it for light and dark menu bars.
            kite.setSize(NSSize::new(ICON, ICON));
            kite.setTemplate(true);
        }
        let button = item.button(mtm);
        if let Some(button) = &button {
            button.setImage(kite.as_deref());
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
            button,
            kite,
        })
    }

    /// Shows `count` unread messages as a red badge on the kite, like
    /// Telegram; zero shows the plain kite.
    pub fn set_unread(&self, count: i32) {
        let (Some(button), Some(kite)) = (&self.button, &self.kite) else {
            return;
        };
        if count <= 0 {
            button.setImage(Some(kite));
            return;
        }
        let label = if count > 99 {
            "99+".to_owned()
        } else {
            count.to_string()
        };
        button.setImage(Some(&badged(kite, &label)));
    }
}

/// Points per side of the menu bar icon.
const ICON: f64 = 22.;
const BADGE_HEIGHT: f64 = 12.;

/// The kite with a red count badge at its top right. Drawn lazily by AppKit
/// in the menu bar's appearance: the kite takes the menu bar's text color
/// (what a template image would get), the badge stays red.
fn badged(kite: &Retained<NSImage>, label: &str) -> Retained<NSImage> {
    let font = NSFont::boldSystemFontOfSize(8.5);
    let white = NSColor::whiteColor();
    // SAFETY: AppKit's string attribute keys, with matching value types.
    let attrs = unsafe {
        NSDictionary::from_slices(
            &[NSFontAttributeName, NSForegroundColorAttributeName],
            &[&*font as &AnyObject, &*white as &AnyObject],
        )
    };
    let text = NSString::from_str(label);
    // SAFETY: Measuring with the attributes built above.
    let text_size = unsafe { text.sizeWithAttributes(Some(&attrs)) };
    let badge_width = (text_size.width + 6.).max(BADGE_HEIGHT);
    // Wide counts extend the image to the right rather than covering the kite.
    let width = (ICON - 9. + badge_width).max(ICON);
    let kite = kite.clone();
    let draw = RcBlock::new(move |bounds: NSRect| -> Bool {
        // The kite in the menu bar's text color.
        kite.drawInRect_fromRect_operation_fraction(
            NSRect::new(NSPoint::new(0., 0.), NSSize::new(ICON, ICON)),
            NSRect::ZERO,
            NSCompositingOperation::SourceOver,
            1.,
        );
        NSColor::labelColor().setFill();
        NSRectFillUsingOperation(
            NSRect::new(NSPoint::new(0., 0.), NSSize::new(ICON, ICON)),
            NSCompositingOperation::SourceAtop,
        );
        // Badge, top right (y grows up), with a clear ring so it stands off
        // the kite.
        let badge = NSRect::new(
            NSPoint::new(
                bounds.size.width - badge_width,
                bounds.size.height - BADGE_HEIGHT,
            ),
            NSSize::new(badge_width, BADGE_HEIGHT),
        );
        let ring = NSRect::new(
            NSPoint::new(badge.origin.x - 1.5, badge.origin.y - 1.5),
            NSSize::new(badge.size.width + 3., badge.size.height + 3.),
        );
        NSColor::blackColor().setFill();
        let radius = (BADGE_HEIGHT + 3.) / 2.;
        let clear = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(ring, radius, radius);
        if let Some(context) = objc2_app_kit::NSGraphicsContext::currentContext() {
            context.saveGraphicsState();
            context.setCompositingOperation(NSCompositingOperation::Clear);
            clear.fill();
            context.restoreGraphicsState();
        }
        NSColor::systemRedColor().setFill();
        let radius = BADGE_HEIGHT / 2.;
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(badge, radius, radius).fill();
        let at = NSPoint::new(
            badge.origin.x + (badge.size.width - text_size.width) / 2.,
            badge.origin.y + (badge.size.height - text_size.height) / 2.,
        );
        // SAFETY: Drawing with the attributes built above.
        unsafe { text.drawAtPoint_withAttributes(at, Some(&attrs)) };
        Bool::YES
    });
    let image =
        NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(width, ICON), false, &draw);
    image.setTemplate(false);
    image
}
