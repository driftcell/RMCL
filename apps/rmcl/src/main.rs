mod app;
mod services;
mod state;
mod theme;

use app::LauncherApp;
use gpui_kit::component::TitleBar;
use gpui_kit::*;
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
    let services = match Services::new(&bootstrap.config, bootstrap.config_store.clone()) {
        Ok(services) => services,
        Err(error) => {
            eprintln!("failed to initialize RMCL: {error}");
            std::process::exit(1);
        }
    };

    application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            init(cx);
            theme::apply(bootstrap.config.appearance, None, cx);

            let mut options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1180.), px(780.)), cx)),
                window_min_size: Some(size(px(980.), px(640.))),
                ..TitleBar::window_options()
            };
            if let Some(titlebar) = &mut options.titlebar {
                titlebar.title = Some("RMCL".into());
            }
            if let Err(error) = open_window(options, cx, |window, cx| {
                theme::apply(bootstrap.config.appearance, Some(window), cx);
                cx.new(|cx| LauncherApp::new(window, cx, services.clone(), bootstrap.clone()))
            }) {
                eprintln!("failed to open RMCL window: {error}");
                cx.quit();
            }
        });
}
