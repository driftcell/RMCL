mod launcher;
mod services;
mod state;

use gpui_kit::*;
use launcher::LauncherView;
use services::{Bootstrap, Services};

fn main() {
    let bootstrap = match Bootstrap::from_args() {
        Ok(Some(bootstrap)) => bootstrap,
        Ok(None) => {
            println!("{}", Bootstrap::usage());
            return;
        }
        Err(error) => {
            eprintln!("{error}\n\n{}", Bootstrap::usage());
            std::process::exit(2);
        }
    };
    let services = match Services::new(bootstrap.java_store.clone()) {
        Ok(services) => services,
        Err(error) => {
            eprintln!("failed to initialize RMCL: {error}");
            std::process::exit(1);
        }
    };

    application().with_assets(assets::Assets).run(move |cx| {
        init(cx);
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("RMCL".into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::centered(size(px(1100.), px(760.)), cx)),
            window_min_size: Some(size(px(800.), px(600.))),
            ..Default::default()
        };
        if let Err(error) = open_window(options, cx, |window, cx| {
            cx.new(|cx| LauncherView::new(window, cx, services.clone(), bootstrap.clone()))
        }) {
            eprintln!("failed to open RMCL window: {error}");
            cx.quit();
        }
    });
}
