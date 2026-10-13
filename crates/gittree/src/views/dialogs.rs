use std::rc::Rc;

use gpui_kit::component::WindowExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::{App, ParentElement, Window};
use rust_i18n::t;

/// Asks before an action that loses work. Enter and Escape both cancel:
/// only a click on the action button goes ahead (FR-SAFE-01).
pub fn confirm_destructive(
    title: String,
    text: String,
    action: String,
    on_confirm: impl Fn(&mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let on_confirm = Rc::new(on_confirm);
    window.open_alert_dialog(cx, move |alert, _, _| {
        let on_confirm = on_confirm.clone();
        alert
            .title(title.clone())
            .description(text.clone())
            // Enter is the dialog's OK. Here it only closes the dialog.
            .on_ok(|_, _, _| true)
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("cancel")
                            .primary()
                            .label(t!("dialog.cancel").into_owned())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("confirm")
                            .danger()
                            .label(action.clone())
                            .on_click(move |_, window, cx| {
                                on_confirm(cx);
                                window.close_dialog(cx);
                            }),
                    ),
            )
    });
}
