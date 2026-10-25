#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
            #[cfg(target_os = "macos")]
            apply_dock_icon();
            theme::apply(bootstrap.config.appearance, None, cx);

            let mut options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1180.), px(780.)), cx)),
                window_min_size: Some(size(px(980.), px(640.))),
                icon: image::load_from_memory(include_bytes!(
                    "../../../packaging/icons/png/rmcl-256.png"
                ))
                .ok()
                .map(|image| std::sync::Arc::new(image.to_rgba8())),
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

#[cfg(target_os = "macos")]
fn apply_dock_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(marker) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(include_bytes!("../../../packaging/icons/png/rmcl-256.png"));
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    let application = NSApplication::sharedApplication(marker);
    // SAFETY: called on the main thread with a valid NSImage.
    unsafe { application.setApplicationIconImage(Some(&image)) };
}
