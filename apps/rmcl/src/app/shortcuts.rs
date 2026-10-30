use gpui_kit::{
    Action, AnyView, App, Context, KeyBinding, Menu, MenuItem, SystemMenuType, Window, actions,
};

use super::{LauncherApp, Page};

actions!(
    rmcl,
    [
        Quit,
        CloseWindow,
        Hide,
        HideOthers,
        ShowAll,
        MinimizeWindow,
        ZoomWindow,
        ToggleFullscreen,
    ]
);

#[derive(Action, Clone, Debug, PartialEq)]
#[action(namespace = rmcl, no_json)]
pub struct SelectPage {
    pub(super) page: Page,
}

const PAGE_KEYS: [&str; 9] = [
    "cmd-1", "cmd-2", "cmd-3", "cmd-4", "cmd-5", "cmd-6", "cmd-7", "cmd-8", "cmd-9",
];

/// Installs the macOS keymap before building native menus, which capture the
/// current bindings when they are created.
pub fn init(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &CloseWindow, cx| {
        with_active_window(cx, |_, window, _| window.remove_window());
    });
    cx.on_action(|_: &MinimizeWindow, cx| {
        with_active_window(cx, |_, window, _| window.minimize_window());
    });
    cx.on_action(|_: &ZoomWindow, cx| {
        with_active_window(cx, |_, window, _| window.zoom_window());
    });
    cx.on_action(|_: &ToggleFullscreen, cx| {
        with_active_window(cx, |_, window, _| window.toggle_fullscreen());
    });
    cx.on_action(|action: &SelectPage, cx| {
        let page = action.page;
        with_active_launcher(cx, move |launcher, window, cx| {
            launcher.navigate(page, window, cx);
        });
    });

    cx.bind_keys(key_bindings());
    cx.set_menus(menus());
}

fn key_bindings() -> Vec<KeyBinding> {
    let mut bindings = vec![
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("cmd-alt-h", HideOthers, None),
        KeyBinding::new("cmd-m", MinimizeWindow, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullscreen, None),
        KeyBinding::new(
            "cmd-,",
            SelectPage {
                page: Page::Settings,
            },
            None,
        ),
    ];
    bindings.extend(
        Page::NAVIGATION
            .into_iter()
            .zip(PAGE_KEYS)
            .map(|(page, key)| KeyBinding::new(key, SelectPage { page }, None)),
    );
    bindings
}

fn menus() -> Vec<Menu> {
    vec![
        Menu::new("RMCL").items([
            MenuItem::action(
                "设置…",
                SelectPage {
                    page: Page::Settings,
                },
            ),
            MenuItem::separator(),
            MenuItem::os_submenu("服务", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("隐藏 RMCL", Hide),
            MenuItem::action("隐藏其他程序", HideOthers),
            MenuItem::action("全部显示", ShowAll),
            MenuItem::separator(),
            MenuItem::action("退出 RMCL", Quit),
        ]),
        Menu::new("文件").items([MenuItem::action("关闭窗口", CloseWindow)]),
        Menu::new("前往").items(
            Page::NAVIGATION
                .into_iter()
                .map(|page| MenuItem::action(page.title(), SelectPage { page })),
        ),
        Menu::new("窗口").items([
            MenuItem::action("最小化", MinimizeWindow),
            MenuItem::action("缩放", ZoomWindow),
            MenuItem::separator(),
            MenuItem::action("切换全屏", ToggleFullscreen),
        ]),
    ]
}

fn with_active_window(cx: &mut App, update: impl FnOnce(AnyView, &mut Window, &mut App) + 'static) {
    if let Some(handle) = cx.active_window() {
        // NOTE: Global actions run while their dispatching window is borrowed.
        // Update that window after dispatch instead of failing a nested update.
        cx.defer(move |cx| {
            let _ = handle.update(cx, |root, window, cx| update(root, window, cx));
        });
    }
}

fn with_active_launcher(
    cx: &mut App,
    update: impl FnOnce(&mut LauncherApp, &mut Window, &mut Context<LauncherApp>) + 'static,
) {
    with_active_window(cx, |root, window, cx| {
        let Ok(root) = root.downcast::<gpui_kit::base::Root>() else {
            return;
        };
        let launcher = root.read(cx).view().clone().downcast::<LauncherApp>().ok();
        if let Some(launcher) = launcher {
            launcher.update(cx, |launcher, cx| update(launcher, window, cx));
        }
    });
}
