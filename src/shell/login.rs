use std::time::Instant;

use gpui_kit::{
    component::{
        Disableable, Sizable,
        button::{Button, ButtonVariants},
        input::Input,
        spinner::Spinner,
    },
    prelude::*,
    *,
};

use super::{Login, Messenger, widgets};
use crate::{telegram::Command, ui};

/// Example input for the step's single field.
pub(super) fn placeholder(login: Login) -> &'static str {
    match login {
        Login::Phone => "+1 555 123 4567",
        Login::Code | Login::EmailCode => "Code",
        Login::Password => "Password",
        Login::Email => "you@example.com",
        _ => "",
    }
}

impl Messenger {
    pub(super) fn login_view(&self, cx: &mut Context<Self>) -> Div {
        let title = match self.login {
            Login::Application => "Welcome to Kite",
            Login::Phone => "Your Phone Number",
            Login::Qr => "Log in by QR Code",
            Login::Code | Login::EmailCode => "Enter Code",
            Login::Password => "Enter Password",
            Login::Email => "Your Email",
            Login::Blocked => "Unable to Continue",
            _ => "Kite",
        };
        let form = matches!(
            self.login,
            Login::Application
                | Login::Phone
                | Login::Code
                | Login::Password
                | Login::Email
                | Login::EmailCode
        );
        let field = |input: &Entity<gpui_kit::component::input::InputState>| {
            widgets::field()
                .w_full()
                .h(px(ui::CONTROL_LG))
                .child(Input::new(input).appearance(false).disabled(self.auth_busy))
        };
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                // A floating card, not a window-inset panel.
                super::panel()
                    .rounded(px(ui::RADIUS_LG))
                    .w(px(420.))
                    .h_auto()
                    .px(px(36.))
                    .pt(px(40.))
                    .pb(px(28.))
                    .flex_col()
                    .items_center()
                    .gap_4()
                    .child(match &self.qr {
                        Some(qr) => div()
                            .p(px(10.))
                            .rounded(px(ui::RADIUS_LG))
                            .bg(ui::hsla(ui::WHITE))
                            .child(crate::qr::view(qr.clone()))
                            .into_any_element(),
                        None => widgets::logo(ui::AVATAR_XL).into_any_element(),
                    })
                    .child(
                        div()
                            .mt_2()
                            .text_size(px(ui::TEXT_XL))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_center()
                            .text_size(px(ui::TEXT_MD))
                            .text_color(ui::hsla(ui::MUTED))
                            .line_height(px(20.))
                            .child(self.login_hint.clone()),
                    )
                    .when(self.login == Login::Waiting, |el| {
                        el.child(Spinner::new().color(ui::hsla(ui::MUTED)))
                    })
                    .when(self.login == Login::Application, |el| {
                        el.child(field(&self.api_id)).child(field(&self.api_hash))
                    })
                    .when(form && self.login != Login::Application, |el| {
                        el.child(field(&self.login_input))
                    })
                    .when(self.login == Login::Code, |el| {
                        let Some((deadline, method)) = &self.resend else {
                            return el;
                        };
                        let remaining = deadline
                            .saturating_duration_since(Instant::now())
                            .as_secs_f64()
                            .ceil() as u64;
                        el.child(
                            Button::new("resend-code")
                                .ghost()
                                .small()
                                .label(if remaining > 0 {
                                    format!("Send via {method} in {remaining}s")
                                } else {
                                    format!("Send via {method}")
                                })
                                .disabled(self.auth_busy || remaining > 0)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if !this.auth_busy
                                        && this.resend.as_ref().is_some_and(|(deadline, _)| {
                                            Instant::now() >= *deadline
                                        })
                                    {
                                        this.auth_busy = this.send(Command::ResendCode);
                                        cx.notify();
                                    }
                                })),
                        )
                    })
                    .when(form, |el| {
                        el.child(
                            Button::new("continue-login")
                                .primary()
                                .w_full()
                                .h(px(ui::CONTROL_LG))
                                .rounded(px(ui::RADIUS_MD))
                                .loading(self.auth_busy)
                                .label("Continue")
                                .disabled(self.auth_busy)
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.submit_login(window, cx)
                                    }),
                                ),
                        )
                    })
                    .when(
                        matches!(
                            self.login,
                            Login::Phone
                                | Login::Code
                                | Login::Email
                                | Login::EmailCode
                                | Login::Password
                        ),
                        |el| {
                            el.child(
                                Button::new("qr-login")
                                    .ghost()
                                    .label("Log in by QR Code")
                                    .disabled(self.auth_busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if !this.auth_busy {
                                            this.auth_busy = this.send(Command::QrLogin);
                                            cx.notify();
                                        }
                                    })),
                            )
                        },
                    ),
            )
    }
}
