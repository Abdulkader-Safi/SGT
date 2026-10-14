use gittree::app_state::{AppState, AppStateHandle, Loaded};
use gittree::views::root::RootView;
use gittree::{actions, logging, menus, theme, window_options};
use gittree_core::store::Store;
use gpui_kit::AppContext;

/// `gittree mcp [folder]`: serves MCP over stdio for one repository, with
/// no window. The folder defaults to the current one.
fn serve_mcp(folder: Option<std::ffi::OsString>) -> Result<(), String> {
    use gittree_core::ops::setup;
    use gittree_core::store::{Settings, TrustList};

    let mut store = Store::new(Store::default_dir());
    let settings: Settings = store.load();
    let trust: TrustList = store.load();
    let folder = std::path::absolute(folder.unwrap_or_else(|| ".".into()))
        .map_err(|error| error.to_string())?;
    let custom = std::env::var_os("GITTREE_GIT")
        .map(Into::into)
        .or(settings.git_path.clone());
    let git = setup::find_git(custom.as_deref()).map_err(|error| error.to_string())?;
    let repository = setup::open(&git.path, &folder, &trust).map_err(|error| error.to_string())?;
    let runner = gittree_core::GitRunner::new(&git.path, &repository.path, repository.trusted);
    let mut server =
        gittree_core::mcp::Server::new(runner, &repository.common_dir, settings.mcp_access)
            .without(settings.mcp_tools_off.clone());
    if settings.mcp_ask {
        server = server.asking(gittree_core::mcp::ASK_WAIT);
    }
    server
        .serve(std::io::stdin().lock(), std::io::stdout().lock())
        .map_err(|error| error.to_string())
}

/// Askpass helper mode: Git or ssh started this binary to ask a question.
/// It is passed on to the running app, and the answer goes to stdout.
fn askpass(sock: String) -> ! {
    use gittree_core::askpass;
    let token = std::env::var(askpass::TOKEN_VAR).unwrap_or_default();
    let question = std::env::args().nth(1).unwrap_or_default();
    match askpass::ask(&sock, &token, &question) {
        Some(answer) => {
            println!("{answer}");
            std::process::exit(0)
        }
        None => std::process::exit(1),
    }
}

fn main() {
    if let Ok(sock) = std::env::var(gittree_core::askpass::SOCK_VAR) {
        askpass(sock);
    }
    logging::init();
    let mut args = std::env::args_os().skip(1);
    if args.next().is_some_and(|arg| arg == "mcp") {
        if let Err(error) = serve_mcp(args.next()) {
            eprintln!("gittree mcp: {error}");
            std::process::exit(1);
        }
        return;
    }
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
            let mut loaded = loaded.join().expect("reading the app's files panicked");
            let keymap = std::mem::take(&mut loaded.keymap);
            let options = window_options(loaded.session.window, cx);
            let app = cx.new(|cx| AppState::new(loaded, cli_path, cx));
            cx.set_global(AppStateHandle(app.clone()));
            theme::apply(cx);
            actions::init(cx);
            actions::load_keymap(&keymap, cx);
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
