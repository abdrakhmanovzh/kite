use async_channel::Receiver;
use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::{NSBundle, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
    UNNotificationRequest, UNNotificationSound, UNUserNotificationCenter,
};

pub fn authorization(request: bool) -> Receiver<Result<bool, String>> {
    let (sender, receiver) = async_channel::bounded(1);
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        let _ = sender.try_send(Err(
            "Open the app bundle to enable macOS notifications.".into()
        ));
        return receiver;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    if request {
        let callback = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            // SAFETY: Apple's completion handler supplies a valid NSError for this call or null.
            let result = unsafe { error.as_ref() }.map_or_else(
                || Ok(granted.as_bool()),
                |e| Err(e.localizedDescription().to_string()),
            );
            let _ = sender.try_send(result);
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &callback,
        );
    } else {
        let callback = RcBlock::new(
            move |settings: std::ptr::NonNull<objc2_user_notifications::UNNotificationSettings>| {
                // SAFETY: Settings is valid for the lifetime of this callback.
                let allowed = unsafe { settings.as_ref() }.authorizationStatus()
                    == UNAuthorizationStatus::Authorized;
                let _ = sender.try_send(Ok(allowed));
            },
        );
        center.getNotificationSettingsWithCompletionHandler(&callback);
    }
    receiver
}

pub fn show(id: i32, title: &str, body: &str, silent: bool) -> Receiver<Result<(), String>> {
    let (sender, receiver) = async_channel::bounded(1);
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    if !silent {
        content.setSound(Some(&UNNotificationSound::defaultSound()));
    }
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&format!("telegram-{id}")),
        &content,
        None,
    );
    let callback = RcBlock::new(move |error: *mut NSError| {
        // SAFETY: Apple's completion handler supplies a valid NSError for this call or null.
        let result =
            unsafe { error.as_ref() }.map_or(Ok(()), |e| Err(e.localizedDescription().to_string()));
        let _ = sender.try_send(result);
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&callback));
    receiver
}
