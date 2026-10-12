use gittree::app_state::{AppState, AppStateHandle, Loaded};
use gittree::views::root::RootView;
use gittree::{actions, logging, menus, theme, window_options};
use gittree_core::store::Store;
use gpui_kit::AppContext;

fn main() {
    logging::init();
    // A path on the command line opens that repository.
    let cli_path = std::env::args_os()
        .nth(1)
        .map(|path| std::path::absolute(&path).unwrap_or(path.into()));
    // Read the app's files while the UI starts up.
    let loaded = std::thread::spawn(|| Loaded::read(Store::new(Store::default_dir())));

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let loaded = loaded.join().expect("reading the app's files panicked");
            let options = window_options(loaded.session.window, cx);
            let app = cx.new(|cx| AppState::new(loaded, cli_path, cx));
            cx.set_global(AppStateHandle(app.clone()));
            theme::apply(cx);
            actions::init(cx);
            menus::init(cx);

            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| RootView::new(app, window, cx))
            })
            .expect("failed to open window");
            // One main window: closing the last window ends the app.
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
}
