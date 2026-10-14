use gittree_core::agent::AgentKind;
use gittree_core::mcp;
use gittree_core::store::{DiffView, McpAccess, PullMode, Settings as Stored, ThemeChoice};
use gpui_kit::component::button::Button;
use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::{Sizable, h_flex};
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, ClipboardItem, Context, Global, IntoElement,
    ParentElement, Render, SharedString, Styled, TitlebarOptions, Window, WindowBounds,
    WindowOptions, div, size,
};

use crate::zoom::px;
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
            size(gpui_kit::px(720.), gpui_kit::px(560.)),
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
    number_in((9., 24., 1.), get, set)
}

fn number_in(
    (min, max, step): (f64, f64, f64),
    get: fn(&Stored) -> f32,
    set: fn(&mut Stored, f32),
) -> SettingField<f64> {
    SettingField::number_input(
        NumberFieldOptions { min, max, step },
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(gpui_kit::px(crate::zoom::BASE * crate::zoom::level(cx)));
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
                        (ThemeChoice::HighContrast, "settings.theme_high_contrast"),
                    ],
                    |s| s.theme,
                    |s, v| s.theme = v,
                ),
            ))
            .item(
                SettingItem::new(
                    label("settings.custom_theme"),
                    text(|s| s.custom_theme.clone(), |s, v| s.custom_theme = v),
                )
                .description(label("settings.custom_theme_help")),
            )
            .item(SettingItem::new(
                label("settings.ui_font_size"),
                number(|s| s.ui_font_size, |s, v| s.ui_font_size = v),
            ))
            .item(
                SettingItem::new(
                    label("settings.zoom"),
                    number_in(
                        (50., 300., 10.),
                        |s| (s.zoom * 100.).round(),
                        |s, v| s.zoom = crate::zoom::clamp(v / 100.),
                    ),
                )
                .description(label("settings.zoom_help")),
            )
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
                    label("settings.auto_fetch"),
                    choice(
                        &[
                            (0, "settings.auto_fetch_off"),
                            (5, "settings.auto_fetch_5"),
                            (15, "settings.auto_fetch_15"),
                            (60, "settings.auto_fetch_60"),
                        ],
                        |s| s.auto_fetch_minutes,
                        |s, v| s.auto_fetch_minutes = v,
                    ),
                )
                .description(label("settings.auto_fetch_help")),
            )
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

        let ai = SettingGroup::new()
            .title(label("settings.agents"))
            .item(
                SettingItem::new(
                    label("settings.ai_agent"),
                    choice(
                        &[
                            (AgentKind::Claude, "settings.ai_claude"),
                            (AgentKind::Codex, "settings.ai_codex"),
                            (AgentKind::Gemini, "settings.ai_gemini"),
                            (AgentKind::Other, "settings.ai_other"),
                        ],
                        |s| s.ai_agent,
                        |s, v| s.ai_agent = v,
                    ),
                )
                .description(label("settings.ai_help")),
            )
            .item(
                SettingItem::new(
                    label("settings.ai_command"),
                    text(|s| s.ai_command.clone(), |s, v| s.ai_command = v),
                )
                .description(label("settings.ai_command_help")),
            );

        let mcp_server = SettingGroup::new()
            .title(label("settings.mcp_server"))
            .item(
                SettingItem::new(
                    label("settings.mcp_access"),
                    choice(
                        &[
                            (McpAccess::ReadOnly, "settings.mcp_read_only"),
                            (McpAccess::ReadWrite, "settings.mcp_read_write"),
                        ],
                        |s| s.mcp_access,
                        |s, v| s.mcp_access = v,
                    ),
                )
                .description(label("settings.mcp_help")),
            )
            .item(
                SettingItem::new(
                    label("settings.mcp_ask"),
                    SettingField::switch(
                        |cx| read(cx, |s| s.mcp_ask),
                        |on, cx| write(cx, |s| s.mcp_ask = on),
                    ),
                )
                .description(label("settings.mcp_ask_help")),
            )
            .item(
                SettingItem::new(
                    label("settings.mcp_connect"),
                    SettingField::render(|_, _, cx| {
                        let p = crate::palette::palette(cx);
                        let command = std::env::current_exe()
                            .map(|program| mcp::client_command(&program))
                            .unwrap_or_default();
                        let copied = command.clone();
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .max_w(px(300.))
                                    .truncate()
                                    .px_2()
                                    .py_1()
                                    .rounded(px(6.))
                                    .bg(p.bg_sunken)
                                    .border_1()
                                    .border_color(p.border)
                                    .font_family(crate::views::graph::mono(cx))
                                    .text_size(px(11.))
                                    .child(command),
                            )
                            .child(
                                Button::new("copy-mcp-command")
                                    .small()
                                    .label(t!("ai.copy").into_owned())
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copied.clone(),
                                        ))
                                    }),
                            )
                    }),
                )
                .description(label("settings.mcp_connect_help")),
            );
        // One switch per tool. A tool that is off is not offered to agents.
        let mcp_tools = mcp::TOOLS.iter().fold(
            SettingGroup::new().title(label("settings.mcp_tools")),
            |group, &(name, changes)| {
                group.item(
                    SettingItem::new(
                        name,
                        SettingField::switch(
                            move |cx| read(cx, |s| !s.mcp_tools_off.iter().any(|off| off == name)),
                            move |on, cx| {
                                write(cx, |s| {
                                    s.mcp_tools_off.retain(|off| off != name);
                                    if !on {
                                        s.mcp_tools_off.push(name.to_owned());
                                    }
                                })
                            },
                        ),
                    )
                    .description(label(if changes {
                        "settings.mcp_tool_changes"
                    } else {
                        "settings.mcp_tool_reads"
                    })),
                )
            },
        );

        div().size_full().child(
            Settings::new("settings")
                .page(
                    SettingPage::new(label("settings.title"))
                        .default_open(true)
                        .group(appearance)
                        .group(git)
                        .group(ai),
                )
                .page(
                    SettingPage::new(label("settings.mcp_page"))
                        .group(mcp_server)
                        .group(mcp_tools),
                ),
        )
    }
}
