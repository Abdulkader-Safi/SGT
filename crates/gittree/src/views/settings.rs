use gittree_core::store::{DiffView, PullMode, Settings as Stored, ThemeChoice};
use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Context, Global, IntoElement, ParentElement, Render,
    SharedString, Styled, TitlebarOptions, Window, WindowBounds, WindowOptions, div, px, size,
};
use rust_i18n::t;

use crate::app_state::app_state;

#[derive(Default)]
struct SettingsWindow(Option<AnyWindowHandle>);

impl Global for SettingsWindow {}

/// Opens the settings window, or brings it forward if it is open.
pub fn open(cx: &mut App) {
    if let Some(handle) = cx.default_global::<SettingsWindow>().0
        && cx
            .update_window(handle, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(720.), px(560.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(t!("settings.title").into_owned().into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    if let Ok((handle, _)) = gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| SettingsView)) {
        cx.set_global(SettingsWindow(Some(handle)));
    }
}

fn read<T>(cx: &App, get: impl FnOnce(&Stored) -> T) -> T {
    get(&app_state(cx).read(cx).settings)
}

fn write(cx: &mut App, set: impl FnOnce(&mut Stored)) {
    app_state(cx).update(cx, |app, cx| app.update_settings(set, cx));
}

/// A dropdown over an enum: `(value, label key)` pairs.
fn choice<T: Copy + PartialEq + 'static>(
    options: &'static [(T, &'static str)],
    get: fn(&Stored) -> T,
    set: fn(&mut Stored, T),
) -> SettingField<SharedString> {
    let default = get(&Stored::default());
    let name = move |value: T| -> SharedString {
        let found = options.iter().find(|(option, _)| *option == value);
        found.map_or("", |(_, key)| *key).into()
    };
    SettingField::dropdown(
        options
            .iter()
            .map(|(_, key)| ((*key).into(), t!(*key).into_owned().into()))
            .collect(),
        move |cx| name(read(cx, get)),
        move |picked: SharedString, cx| {
            if let Some((value, _)) = options.iter().find(|(_, key)| *key == picked.as_ref()) {
                write(cx, |settings| set(settings, *value));
            }
        },
    )
    .default_value(name(default))
}

fn number(get: fn(&Stored) -> f32, set: fn(&mut Stored, f32)) -> SettingField<f64> {
    SettingField::number_input(
        NumberFieldOptions {
            min: 9.,
            max: 24.,
            step: 1.,
        },
        move |cx| read(cx, get) as f64,
        move |value, cx| write(cx, |settings| set(settings, value as f32)),
    )
    .default_value(get(&Stored::default()) as f64)
}

fn text(get: fn(&Stored) -> String, set: fn(&mut Stored, String)) -> SettingField<SharedString> {
    SettingField::input(
        move |cx| read(cx, get).into(),
        move |value: SharedString, cx| write(cx, |settings| set(settings, value.to_string())),
    )
    .default_value(get(&Stored::default()))
}

/// One page with a control per setting, each with reset (FR-SET-01).
pub struct SettingsView;

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let label = |key: &str| -> SharedString { t!(key).into_owned().into() };
        let appearance = SettingGroup::new()
            .title(label("settings.appearance"))
            .item(SettingItem::new(
                label("settings.theme"),
                choice(
                    &[
                        (ThemeChoice::System, "settings.theme_system"),
                        (ThemeChoice::Light, "settings.theme_light"),
                        (ThemeChoice::Dark, "settings.theme_dark"),
                    ],
                    |s| s.theme,
                    |s, v| s.theme = v,
                ),
            ))
            .item(SettingItem::new(
                label("settings.ui_font_size"),
                number(|s| s.ui_font_size, |s, v| s.ui_font_size = v),
            ))
            .item(SettingItem::new(
                label("settings.diff_font_family"),
                text(
                    |s| s.diff_font_family.clone(),
                    |s, v| s.diff_font_family = v,
                ),
            ))
            .item(SettingItem::new(
                label("settings.diff_font_size"),
                number(|s| s.diff_font_size, |s, v| s.diff_font_size = v),
            ))
            .item(SettingItem::new(
                label("settings.diff_view"),
                choice(
                    &[
                        (DiffView::Inline, "settings.diff_inline"),
                        (DiffView::Split, "settings.diff_split"),
                    ],
                    |s| s.diff_view,
                    |s, v| s.diff_view = v,
                ),
            ));
        let git = SettingGroup::new()
            .title(label("settings.git"))
            .item(SettingItem::new(
                label("settings.pull_mode"),
                choice(
                    &[
                        (PullMode::FfOnly, "settings.pull_ff_only"),
                        (PullMode::Merge, "settings.pull_merge"),
                        (PullMode::Rebase, "settings.pull_rebase"),
                    ],
                    |s| s.pull_mode,
                    |s, v| s.pull_mode = v,
                ),
            ))
            .item(
                SettingItem::new(
                    label("settings.git_path"),
                    text(
                        |s| {
                            s.git_path
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default()
                        },
                        |s, v| s.git_path = (!v.trim().is_empty()).then(|| v.trim().into()),
                    ),
                )
                .description(label("settings.git_path_help")),
            )
            .item(SettingItem::new(
                label("settings.clone_dir"),
                text(
                    |s| s.clone_dir.display().to_string(),
                    |s, v| s.clone_dir = v.into(),
                ),
            ));

        div().size_full().child(
            Settings::new("settings").page(
                SettingPage::new(label("settings.title"))
                    .default_open(true)
                    .group(appearance)
                    .group(git),
            ),
        )
    }
}
