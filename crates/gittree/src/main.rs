use gittree::views::graph::GraphView;
use gittree::window_options;
use gpui_kit::AppContext;

fn main() {
    let Some(repo) = std::env::args_os().nth(1) else {
        eprintln!("usage: gittree <path to a repository>");
        std::process::exit(2);
    };
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(window_options(cx), cx, |_, cx| {
            cx.new(|cx| GraphView::new(repo.into(), cx))
        })
        .expect("failed to open window");
    });
}
