mod app;
mod services;
mod state;
mod theme;

use std::sync::Arc;

use app::LauncherApp;
use clap::Parser;
use gpui_kit::component::TitleBar;
use gpui_kit::*;
use services::{Bootstrap, LauncherArgs, LauncherCommand, McpArgs, Services};

fn main() {
    let arguments = LauncherArgs::parse();
    if let Some(plan) = arguments.apply_update.as_deref() {
        if let Err(error) = rmcl_app::apply_update(plan) {
            eprintln!("failed to apply RMCL update: {error}");
        }
        return;
    }
    let command = arguments.command.clone();
    let bootstrap = match Bootstrap::from_arguments(arguments) {
        Ok(bootstrap) => bootstrap,
        Err(error) => {
            eprintln!("{error}");
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

    if let Some(command) = command {
        if let Err(error) = run_mcp(services, command) {
            eprintln!("RMCL MCP failed: {error}");
            std::process::exit(1);
        }
        return;
    }

    #[cfg(windows)]
    release_console();

    application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            init(cx);
            #[cfg(target_os = "macos")]
            app::shortcuts::init(cx);
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

fn run_mcp(services: Arc<Services>, command: LauncherCommand) -> Result<(), String> {
    let LauncherCommand::Mcp(arguments) = command;
    let runtime = services.runtime.handle().clone();
    let result = if arguments.http && arguments.port == 0 {
        Err("MCP port must be between 1 and 65535".to_owned())
    } else if arguments.stdio {
        runtime.block_on(async {
            rmcl_mcp::serve_stdio(services.context.clone())
                .await
                .map_err(|error| error.to_string())
        })
    } else {
        match http_config(&arguments) {
            Ok(config) => runtime.block_on(async {
                eprintln!(
                    "RMCL MCP listening on http://127.0.0.1:{}/mcp",
                    arguments.port
                );
                rmcl_mcp::serve_http(
                    services.context.clone(),
                    config,
                    tokio_util::sync::CancellationToken::new(),
                )
                .await
                .map_err(|error| error.to_string())
            }),
            Err(error) => Err(error),
        }
    };
    if let Ok(services) = Arc::try_unwrap(services) {
        let services::Services { context, .. } = services;
        if let Ok(context) = Arc::try_unwrap(context) {
            context.runtime.shutdown_background();
        }
    }
    result
}

fn http_config(arguments: &McpArgs) -> Result<rmcl_mcp::HttpServerConfig, String> {
    let access = match arguments.access.as_str() {
        "read_only" => rmcl_mcp::AccessLevel::ReadOnly,
        "read_write" => rmcl_mcp::AccessLevel::ReadWrite,
        other => return Err(format!("unsupported MCP access level `{other}`")),
    };
    let mut config = rmcl_mcp::HttpServerConfig::new(arguments.port);
    config.access = access;
    config.bearer_token = arguments
        .token
        .clone()
        .or_else(|| std::env::var("RMCL_MCP_TOKEN").ok());
    config.allowed_origins = arguments.allowed_origins.iter().cloned().collect();
    Ok(config)
}

#[cfg(windows)]
fn release_console() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn FreeConsole() -> i32;
    }

    // SAFETY: GUI mode does not use console stdio. MCP mode returns before this
    // call and keeps its console or inherited stdio pipes available.
    unsafe {
        let _ = FreeConsole();
    }
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
