use std::path::PathBuf;

use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Sizable, Size, StyledExt, WindowExt,
        button::{Button, ButtonVariant, ButtonVariants},
        dialog::{DialogClose, DialogFooter},
        h_flex,
        input::Input,
        notification::Notification,
        progress::Progress as ProgressBar,
        scroll::ScrollableElement,
        spinner::Spinner,
        tag::Tag,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use rmcl_app::{
    CopyStrategy, CopyWorldOptions, ImportWorldOptions, InstalledVersion, InstanceLayout, World,
    WorldCompatibility, WorldConflictPolicy, WorldCopyProgress, WorldId, backup_instance_world,
    client_world_version, copy_world_to_instance, delete_instance_world, export_instance_world,
    import_instance_world, inspect_instance_worlds, move_world_to_instance, relative_time,
    restore_instance_world,
};
use rmcl_launch::supports_quick_play;
use tokio_util::sync::CancellationToken;

use super::super::{LauncherApp, Page};
use super::common::{
    card, display_path, empty_state, error_banner, format_size, help, mono, page_header,
    warning_banner,
};
use super::layout;
use crate::services::open_path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransferMode {
    Copy,
    Move,
}

impl TransferMode {
    fn label(self) -> &'static str {
        match self {
            Self::Copy => "复制",
            Self::Move => "移动",
        }
    }
}

struct TransferTarget {
    version: InstalledVersion,
    compatibility: WorldCompatibility,
    same_name: bool,
    loader_mismatch: bool,
    missing_mods: bool,
}

impl LauncherApp {
    pub fn render_worlds(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(version) = self.selected_version().cloned() else {
            return v_flex()
                .flex_1()
                .min_h_0()
                .p_6()
                .gap_4()
                .child(page_header(
                    Page::Worlds,
                    Button::new("worlds-go-instances")
                        .primary()
                        .icon(IconName::Boxes)
                        .label("选择实例")
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.navigate(Page::Home, window, cx)
                            }),
                        ),
                    cx,
                ))
                .child(
                    card(cx).child(empty_state(
                        IconName::Map,
                        "先选择一个实例",
                        "先选择一个实例，再管理它的地图。",
                        Button::new("worlds-pick-instance")
                            .primary()
                            .icon(IconName::Boxes)
                            .label("前往主页")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.navigate(Page::Home, window, cx)
                            })),
                        cx,
                    )),
                )
                .into_any_element();
        };

        let worlds = self.filtered_worlds(cx);
        let inspection = self.worlds.as_ref();
        let target_version = inspection.and_then(|inspection| inspection.target_world_version);
        let warning = inspection.and_then(|inspection| inspection.warnings.first());
        let warning_count = inspection.map_or(0, |inspection| inspection.warnings.len());
        let total_size = inspection
            .map(|inspection| inspection.worlds.iter().map(|world| world.size_bytes).sum())
            .unwrap_or(0);
        let total_worlds = inspection.map_or(0, |inspection| inspection.worlds.len());
        let pending = self.world_actions_pending();
        let loading = self.worlds_loading && inspection.is_none();

        v_flex()
            .flex_1()
            .min_h_0()
            .p_6()
            .gap_5()
            .child(page_header(
                Page::Worlds,
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("worlds-open-folder")
                            .ghost()
                            .icon(IconName::FolderOpen)
                            .tooltip("打开存档目录")
                            .accessibility_label("打开存档目录")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_worlds_folder(window, cx)
                            })),
                    )
                    .child(
                        Button::new("worlds-restore")
                            .ghost()
                            .icon(IconName::Archive)
                            .label("恢复备份")
                            .disabled(pending)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.pick_world_restore(window, cx)
                            })),
                    )
                    .child(
                        Button::new("worlds-refresh")
                            .ghost()
                            .icon(IconName::RefreshCw)
                            .tooltip("重新扫描地图")
                            .accessibility_label("重新扫描地图")
                            .disabled(pending)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.load_worlds(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("worlds-import")
                            .primary()
                            .icon(IconName::Plus)
                            .label("导入地图")
                            .disabled(pending)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.pick_world_import(window, cx)
                            })),
                    ),
                cx,
            ))
            .child(self.render_world_summary(
                &version,
                total_worlds,
                total_size,
                target_version,
                cx,
            ))
            .child(
                v_flex()
                    .id("worlds-body")
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .overflow_y_scrollbar()
                    .when_some(self.worlds_error.clone(), |this, error: String| {
                        this.child(error_banner("地图扫描失败", &error, cx))
                    })
                    .when_some(
                        inspection.and_then(|inspection| inspection.client_jar_error.clone()),
                        |this, error: String| {
                            this.child(warning_banner("无法读取目标游戏的数据版本", &error, cx))
                        },
                    )
                    .when_some(warning, |this, warning| {
                        let title = format!("扫描警告（{warning_count}）");
                        this.child(warning_banner(&title, &warning.message, cx))
                    })
                    .when_some(self.world_progress.clone(), |this, progress| {
                        this.child(self.render_world_progress(&progress, cx))
                    })
                    .child(
                        h_flex()
                            .w_full()
                            .gap_3()
                            .items_center()
                            .child(
                                Input::new(&self.world_query)
                                    .prefix(Icon::new(IconName::Search).small())
                                    .cleanable(true)
                                    .w(rems(18.75)),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} 个地图 · {}",
                                        worlds.len(),
                                        format_size(total_size)
                                    )),
                            ),
                    )
                    .child(if loading {
                        card(cx)
                            .child(
                                h_flex()
                                    .w_full()
                                    .justify_center()
                                    .gap_2()
                                    .py_8()
                                    .child(
                                        Spinner::new()
                                            .with_size(Size::Small)
                                            .color(cx.theme().primary),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("正在扫描地图…"),
                                    ),
                            )
                            .into_any_element()
                    } else if worlds.is_empty() {
                        card(cx)
                            .child(empty_state(
                                IconName::Map,
                                if inspection
                                    .is_some_and(|inspection| !inspection.worlds.is_empty())
                                {
                                    "没有匹配的地图"
                                } else {
                                    "还没有地图"
                                },
                                if inspection
                                    .is_some_and(|inspection| !inspection.worlds.is_empty())
                                {
                                    "试试更换搜索词。"
                                } else {
                                    "把旧实例的地图复制过来，或导入一个 ZIP。"
                                },
                                Button::new("worlds-empty-import")
                                    .primary()
                                    .icon(IconName::Plus)
                                    .label("导入地图")
                                    .disabled(pending)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.pick_world_import(window, cx)
                                    })),
                                cx,
                            ))
                            .into_any_element()
                    } else {
                        div()
                            .w_full()
                            .grid()
                            .grid_cols(2)
                            .gap_4()
                            .children(worlds.iter().map(|world| {
                                self.render_world_card(
                                    world,
                                    target_version,
                                    version.loader.is_some(),
                                    pending,
                                    cx,
                                )
                                .into_any_element()
                            }))
                            .into_any_element()
                    }),
            )
            .into_any_element()
    }

    fn render_world_summary(
        &self,
        version: &InstalledVersion,
        world_count: usize,
        total_size: u64,
        target_version: Option<i32>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let instance_path = display_path(version.game_dir.clone());
        let target_label = target_version
            .map(|version| format!("DataVersion {version}"))
            .unwrap_or_else(|| "DataVersion 未知".to_owned());
        card(cx)
            .child(
                h_flex()
                    .w_full()
                    .items_start()
                    .gap_6()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_lg()
                                            .font_semibold()
                                            .truncate()
                                            .child(version.display_name().to_owned()),
                                    )
                                    .when(!version.is_isolated(), |this| {
                                        this.child(Tag::new().outline().child("共享").small())
                                    }),
                            )
                            .child(mono(instance_path, cx)),
                    )
                    .child(world_stat("地图", world_count.to_string(), cx))
                    .child(world_stat("占用", format_size(total_size), cx))
                    .child(world_stat("目标", target_label, cx))
                    .child(
                        Button::new("worlds-switch-instance")
                            .ghost()
                            .icon(IconName::Boxes)
                            .tooltip("在主页切换实例")
                            .accessibility_label("在主页切换实例")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.navigate(Page::Home, window, cx)
                            })),
                    ),
            )
            .when(self.world_action_pending, |this| {
                this.child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_3()
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().primary.opacity(0.06))
                        .px_3()
                        .py_2()
                        .child(
                            Spinner::new()
                                .with_size(Size::XSmall)
                                .color(cx.theme().primary),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("正在处理地图…"),
                        )
                        .when(self.world_cancel.is_some(), |this| {
                            this.child(
                                Button::new("worlds-cancel")
                                    .ghost()
                                    .xsmall()
                                    .label("取消")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.cancel_world_action(window, cx)
                                    })),
                            )
                        }),
                )
            })
    }

    fn render_world_progress(
        &self,
        progress: &WorldCopyProgress,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let percent = if progress.total_bytes == 0 {
            0.0
        } else {
            (progress.copied_bytes as f32 / progress.total_bytes as f32 * 100.0).min(100.0)
        };
        h_flex()
            .w_full()
            .gap_3()
            .items_center()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .px_3()
            .py_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .truncate()
                    .child(format!(
                        "{} / {} 个文件 · {} / {}",
                        progress.completed_files,
                        progress.total_files,
                        format_size(progress.copied_bytes),
                        format_size(progress.total_bytes)
                    )),
            )
            .child(
                ProgressBar::new("world-progress")
                    .value(percent)
                    .w(rems(12.5)),
            )
            .child(
                Button::new("world-progress-cancel")
                    .ghost()
                    .xsmall()
                    .label("取消")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.cancel_world_action(window, cx)),
                    ),
            )
    }

    fn render_world_card(
        &self,
        world: &World,
        target_version: Option<i32>,
        target_has_loader: bool,
        pending: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let compatibility = world.compatibility(target_version);
        let icon = self.world_icons.get(&world.id).cloned();
        let has_icon = icon.is_some();
        let world_id = world.id.clone();
        let folder_name = world.folder_name().to_owned();
        let world_version = world
            .game_version
            .clone()
            .or_else(|| {
                self.selected_version()
                    .map(|version| version.minecraft_version.clone())
            })
            .unwrap_or_default();
        let quick_play = supports_quick_play(&world_version).unwrap_or(false);
        let can_enter =
            !world.locked && compatibility != WorldCompatibility::Downgrade && quick_play;
        let enter_tip = if world.locked {
            "地图正在使用中"
        } else if compatibility == WorldCompatibility::Downgrade {
            "需要更新的游戏版本"
        } else if !quick_play {
            "当前版本不支持快速进入"
        } else {
            "启动实例并进入地图"
        };
        let meta = format!(
            "Minecraft {} · {} · {} · {}",
            world.game_version.as_deref().unwrap_or("版本未知"),
            world
                .data_version
                .map(|version| format!("DataVersion {version}"))
                .unwrap_or_else(|| "DataVersion 未知".to_owned()),
            relative_time(world.last_played).unwrap_or_else(|| "未记录".to_owned()),
            format_size(world.size_bytes)
        );

        v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(if world.locked {
                cx.theme().warning.opacity(0.45)
            } else {
                cx.theme().border
            })
            .bg(cx.theme().group_box)
            .p_3()
            .child(
                div()
                    .w_full()
                    .h_40()
                    .rounded(cx.theme().radius)
                    .overflow_hidden()
                    .bg(cx.theme().primary.opacity(0.07))
                    .when_some(icon, |this, icon| {
                        this.child(img(icon).size_full().object_fit(ObjectFit::Contain))
                    })
                    .when(!has_icon, |this| {
                        this.flex().items_center().justify_center().child(
                            Icon::new(IconName::Map)
                                .with_size(Size::Large)
                                .text_color(cx.theme().primary),
                        )
                    }),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_semibold()
                                    .child(world.name.clone()),
                            )
                            .child(compatibility_tag(compatibility)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .line_clamp(2)
                            .child(meta),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .when(world.locked, |this| {
                                this.child(Tag::warning().child("使用中").small())
                            })
                            .when(world.was_modded, |this| {
                                this.child(if target_has_loader {
                                    Tag::new().outline().child("模组").small()
                                } else {
                                    Tag::warning().child("需要模组").small()
                                })
                            })
                            .when(
                                world.enabled_datapacks.iter().any(|pack| pack != "vanilla"),
                                |this| this.child(Tag::new().outline().child("数据包").small()),
                            )
                            .when(!world.enabled_features.is_empty(), |this| {
                                this.child(Tag::warning().child("实验性").small())
                            }),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new(format!("world-enter-{folder_name}"))
                            .outline()
                            .small()
                            .icon(IconName::Play)
                            .label("进入")
                            .disabled(pending || !can_enter)
                            .tooltip(enter_tip)
                            .accessibility_label(enter_tip)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.start_world_launch(world_id.as_str().to_owned(), window, cx)
                            })),
                    )
                    .child(
                        Button::new(format!("world-copy-{folder_name}"))
                            .ghost()
                            .small()
                            .icon(IconName::Copy)
                            .label("复制")
                            .tooltip("复制到其他实例，源地图保留")
                            .disabled(pending || world.locked)
                            .on_click(cx.listener({
                                let world = world.clone();
                                move |this, _, window, cx| {
                                    this.open_world_transfer_dialog(
                                        world.clone(),
                                        TransferMode::Copy,
                                        window,
                                        cx,
                                    )
                                }
                            })),
                    )
                    .child(
                        Button::new(format!("world-move-{folder_name}"))
                            .ghost()
                            .small()
                            .label("移动")
                            .tooltip("移动到其他实例，源地图删除")
                            .disabled(pending || world.locked)
                            .on_click(cx.listener({
                                let world = world.clone();
                                move |this, _, window, cx| {
                                    this.open_world_transfer_dialog(
                                        world.clone(),
                                        TransferMode::Move,
                                        window,
                                        cx,
                                    )
                                }
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new(format!("world-export-{folder_name}"))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Download)
                            .tooltip("导出 ZIP")
                            .accessibility_label("导出 ZIP")
                            .disabled(pending || world.locked)
                            .on_click(cx.listener({
                                let world = world.clone();
                                move |this, _, window, cx| {
                                    this.pick_world_export(world.clone(), window, cx)
                                }
                            })),
                    )
                    .child(
                        Button::new(format!("world-backup-{folder_name}"))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Archive)
                            .tooltip("备份")
                            .accessibility_label("备份")
                            .disabled(pending || world.locked)
                            .on_click(cx.listener({
                                let world = world.clone();
                                move |this, _, window, cx| {
                                    this.backup_world_action(world.clone(), window, cx)
                                }
                            })),
                    )
                    .child(
                        Button::new(format!("world-delete-{folder_name}"))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Trash)
                            .tooltip("删除")
                            .accessibility_label("删除")
                            .disabled(pending || world.locked)
                            .on_click(cx.listener({
                                let world = world.clone();
                                move |this, _, window, cx| {
                                    this.confirm_delete_world(world.clone(), window, cx)
                                }
                            })),
                    ),
            )
    }

    fn open_world_transfer_dialog(
        &mut self,
        world: World,
        mode: TransferMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.selected_version().cloned() else {
            return;
        };
        let targets: Vec<TransferTarget> = self
            .installed_versions
            .iter()
            .filter(|version| version.id != source.id)
            .cloned()
            .map(|version| {
                let target_version = client_world_version(&version.client_jar_path)
                    .ok()
                    .flatten();
                let same_name = version
                    .instance_layout(self.config.minecraft_root.clone())
                    .saves_dir()
                    .join(world.folder_name())
                    .exists();
                TransferTarget {
                    compatibility: world.compatibility(target_version),
                    same_name,
                    loader_mismatch: version.loader != source.loader,
                    missing_mods: world.was_modded && version.loader.is_none(),
                    version,
                }
            })
            .collect();
        let source_id = source.id.as_str().to_owned();
        let world_id = world.id.clone();
        let view = cx.entity().downgrade();
        let mode_label = mode.label();
        let has_conflict = targets.iter().any(|target| target.same_name);
        window.open_dialog(cx, move |dialog, _, cx| {
            dialog
                .title(format!("{mode_label}地图"))
                .width(layout::rem_px(cx, layout::DIALOG_LARGE))
                .child(
                    v_flex()
                        .gap_3()
                        .child(help(
                            if mode == TransferMode::Move {
                                "移动后当前实例不再保留这张地图；升级前自动备份。"
                            } else {
                                "目标实例会得到一份独立地图；升级前自动备份。"
                            },
                            cx,
                        ))
                        .when(has_conflict, |this| {
                            this.child(help("覆盖同名地图前会先备份。", cx))
                        })
                        .when(targets.is_empty(), |this| {
                            this.child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("没有其他实例。先创建一个实例。"),
                            )
                        })
                        .children(targets.iter().map(|target| {
                            let target_id = target.version.id.as_str().to_owned();
                            let view = view.clone();
                            let source_id = source_id.clone();
                            let world_id = world_id.clone();
                            let same_name = target.same_name;
                            let compatibility = target.compatibility;
                            let unsafe_move = mode == TransferMode::Move
                                && compatibility == WorldCompatibility::Downgrade;
                            let target_button = Button::new(format!("world-target-{target_id}"))
                                .primary()
                                .small()
                                .label(if same_name { "覆盖" } else { mode.label() })
                                .disabled(unsafe_move);
                            let target_button = if unsafe_move {
                                target_button.tooltip("目标版本更低，不能移动")
                            } else {
                                target_button
                            };
                            h_flex()
                                .w_full()
                                .items_center()
                                .gap_3()
                                .rounded(cx.theme().radius)
                                .border_1()
                                .border_color(cx.theme().border)
                                .p_3()
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_0p5()
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_medium()
                                                .truncate()
                                                .child(target.version.display_name().to_owned()),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(format!(
                                                    "Minecraft {} · {}",
                                                    target.version.minecraft_version,
                                                    target.version.id
                                                )),
                                        ),
                                )
                                .child(compatibility_tag(compatibility))
                                .when(target.loader_mismatch, |this| {
                                    this.child(Tag::warning().child("加载器不同").small())
                                })
                                .when(target.missing_mods, |this| {
                                    this.child(Tag::warning().child("缺少模组").small())
                                })
                                .when(same_name, |this| {
                                    this.child(
                                        Button::new(format!("world-target-rename-{target_id}"))
                                            .outline()
                                            .small()
                                            .label(format!("{}为副本", mode.label()))
                                            .on_click({
                                                let view = view.clone();
                                                let source_id = source_id.clone();
                                                let world_id = world_id.clone();
                                                let target_id = target_id.clone();
                                                move |_, window, cx| {
                                                    let Some(view) = view.upgrade() else {
                                                        return;
                                                    };
                                                    view.update(cx, |this, cx| {
                                                        this.start_world_transfer(
                                                            source_id.clone(),
                                                            target_id.clone(),
                                                            world_id.clone(),
                                                            mode,
                                                            WorldConflictPolicy::Rename,
                                                            window,
                                                            cx,
                                                        );
                                                    });
                                                    window.close_dialog(cx);
                                                }
                                            }),
                                    )
                                })
                                .child(target_button.on_click({
                                    let view = view.clone();
                                    let source_id = source_id.clone();
                                    let world_id = world_id.clone();
                                    let target_id = target_id.clone();
                                    let conflict = if same_name {
                                        WorldConflictPolicy::Replace
                                    } else {
                                        WorldConflictPolicy::Rename
                                    };
                                    move |_, window, cx| {
                                        let Some(view) = view.upgrade() else {
                                            return;
                                        };
                                        view.update(cx, |this, cx| {
                                            this.start_world_transfer(
                                                source_id.clone(),
                                                target_id.clone(),
                                                world_id.clone(),
                                                mode,
                                                conflict,
                                                window,
                                                cx,
                                            );
                                        });
                                        window.close_dialog(cx);
                                    }
                                }))
                        })),
                )
                .footer(
                    DialogFooter::new()
                        .child(DialogClose::new().trigger(|button| button.label("取消").outline())),
                )
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn start_world_transfer(
        &mut self,
        source_id: String,
        target_id: String,
        world_id: WorldId,
        mode: TransferMode,
        conflict: WorldConflictPolicy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.world_actions_pending() {
            return;
        }
        let Some(source) = self
            .installed_versions
            .iter()
            .find(|version| version.id.as_str() == source_id)
            .cloned()
        else {
            return;
        };
        let Some(target) = self
            .installed_versions
            .iter()
            .find(|version| version.id.as_str() == target_id)
            .cloned()
        else {
            return;
        };
        let source_instance = source.instance_layout(self.config.minecraft_root.clone());
        let target_instance = target.instance_layout(self.config.minecraft_root.clone());
        let target_name = target.display_name().to_owned();
        let target_world_version = client_world_version(&target.client_jar_path).ok().flatten();
        let options = CopyWorldOptions {
            target_name: None,
            conflict,
            strategy: CopyStrategy::Auto,
        };
        let cancellation = CancellationToken::new();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        self.world_action_pending = true;
        self.world_progress = None;
        self.world_cancel = Some(cancellation.clone());
        self._world_progress_task = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = receiver.recv().await {
                let _ = this.update_in(cx, |view, _, cx| {
                    view.world_progress = Some(event);
                    cx.notify();
                });
            }
        }));
        let services = self.services.clone();
        self._world_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = services
                .runtime
                .spawn(async move {
                    if mode == TransferMode::Copy {
                        copy_world_to_instance(
                            source_instance,
                            target_instance,
                            world_id,
                            target_world_version,
                            options,
                            cancellation,
                            Some(sender),
                        )
                        .await
                    } else {
                        move_world_to_instance(
                            source_instance,
                            target_instance,
                            world_id,
                            target_world_version,
                            options,
                            cancellation,
                            Some(sender),
                        )
                        .await
                    }
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.world_action_pending = false;
                view.world_cancel = None;
                view.world_progress = None;
                view._world_progress_task = None;
                match result {
                    Ok(Ok(outcome)) => {
                        window.push_notification(
                            Notification::success(format!(
                                "已{}到 {}（{}）。",
                                mode.label(),
                                target_name,
                                outcome.world.name
                            ))
                            .title("地图已处理"),
                            cx,
                        );
                        view.load_worlds(window, cx);
                    }
                    Ok(Err(error)) => {
                        window.push_notification(
                            Notification::error(error.to_string())
                                .title(format!("{}地图失败", mode.label())),
                            cx,
                        );
                        view.load_worlds(window, cx);
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(format!("后台任务失败: {error}"))
                                .title(format!("{}地图失败", mode.label())),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn start_world_launch(&mut self, world: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(version) = self.selected_version().cloned() else {
            return;
        };
        let Some(entry) = self.worlds.as_ref().and_then(|inspection| {
            inspection
                .worlds
                .iter()
                .find(|entry| entry.folder_name() == world)
        }) else {
            return;
        };
        let compatibility = entry.compatibility(
            self.worlds
                .as_ref()
                .and_then(|inspection| inspection.target_world_version),
        );
        let world_version = entry
            .game_version
            .clone()
            .unwrap_or_else(|| version.minecraft_version.clone());
        if entry.locked {
            window.push_notification(
                Notification::warning("该地图正在使用中。").title("无法进入地图"),
                cx,
            );
            return;
        }
        if compatibility == WorldCompatibility::Downgrade {
            window.push_notification(
                Notification::warning("目标游戏版本低于地图数据版本。").title("无法进入地图"),
                cx,
            );
            return;
        }
        match supports_quick_play(&world_version) {
            Ok(true) => {}
            Ok(false) => {
                window.push_notification(
                    Notification::warning("当前游戏版本不支持快速进入地图。").title("无法快速进入"),
                    cx,
                );
                return;
            }
            Err(error) => {
                window.push_notification(
                    Notification::warning(error.to_string()).title("无法快速进入"),
                    cx,
                );
                return;
            }
        }
        self.start_launch_with_world(version.id.as_str().to_owned(), world, window, cx);
    }

    pub(crate) fn load_worlds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.world_request = self.world_request.wrapping_add(1);
        let request = self.world_request;
        let Some(version) = self.selected_version().cloned() else {
            self.worlds = None;
            self.worlds_instance = None;
            self.world_icons.clear();
            self.worlds_loading = false;
            self.worlds_error = None;
            cx.notify();
            return;
        };
        let instance_id = version.id.as_str().to_owned();
        if self.worlds_instance.as_deref() != Some(instance_id.as_str()) {
            self.worlds = None;
            self.world_icons.clear();
            self.worlds_instance = Some(instance_id.clone());
        }
        let instance = version.instance_layout(self.config.minecraft_root.clone());
        let client_jar = version.client_jar_path.clone();
        let services = self.services.clone();
        self.worlds_loading = true;
        self.worlds_error = None;
        self._world_load_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = services
                .runtime
                .spawn(async move {
                    tokio::task::spawn_blocking(move || {
                        inspect_instance_worlds(&instance, &client_jar)
                    })
                    .await
                    .map_err(|error| rmcl_app::AppError::BlockingTask(error.to_string()))?
                })
                .await;
            let _ = this.update_in(cx, |view, _, cx| {
                if view.world_request != request {
                    return;
                }
                view.worlds_loading = false;
                match result {
                    Ok(Ok(inspection)) => {
                        view.world_icons = inspection
                            .worlds
                            .iter()
                            .filter_map(|world| {
                                let path = world.icon_path.as_ref()?;
                                let image =
                                    image::load_from_memory(&std::fs::read(path).ok()?).ok()?;
                                Some((
                                    world.id.clone(),
                                    super::common::render_rgba_image(image.to_rgba8()),
                                ))
                            })
                            .collect();
                        view.worlds = Some(inspection);
                        view.worlds_error = None;
                    }
                    Ok(Err(error)) => view.worlds_error = Some(error.to_string()),
                    Err(error) => {
                        view.worlds_error = Some(format!("后台任务失败: {error}"));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn filtered_worlds(&self, cx: &App) -> Vec<World> {
        let query = self.world_query.read(cx).value().to_lowercase();
        self.worlds
            .as_ref()
            .map(|inspection| {
                inspection
                    .worlds
                    .iter()
                    .filter(|world| {
                        query.is_empty()
                            || world.name.to_lowercase().contains(&query)
                            || world.folder_name().to_lowercase().contains(&query)
                            || world
                                .game_version
                                .as_deref()
                                .is_some_and(|version| version.to_lowercase().contains(&query))
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn world_actions_pending(&self) -> bool {
        self.world_action_pending || self.worlds_loading || self.content_operations_pending()
    }

    fn cancel_world_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(cancellation) = self.world_cancel.take() {
            cancellation.cancel();
            window.push_notification(Notification::info("正在取消地图操作。").title("地图"), cx);
        }
        cx.notify();
    }

    fn open_worlds_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(version) = self.selected_version().cloned() else {
            return;
        };
        let directory = version
            .instance_layout(self.config.minecraft_root.clone())
            .saves_dir();
        let _ = std::fs::create_dir_all(&directory);
        if let Err(error) = open_path(&directory) {
            window.push_notification(
                Notification::error(format!("无法打开存档目录: {error}")).title("打开失败"),
                cx,
            );
        }
    }

    fn pick_world_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(version) = self.selected_version().cloned() else {
            return;
        };
        let instance = version.instance_layout(self.config.minecraft_root.clone());
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择地图 ZIP".into()),
        });
        self._prompt_task = Some(
            cx.spawn_in(window, async move |this, cx| match prompt.await {
                Ok(Ok(Some(paths))) => {
                    let Some(path) = paths.into_iter().next() else {
                        return;
                    };
                    let _ = this.update_in(cx, |view, window, cx| {
                        view.start_world_import(path, false, instance.clone(), window, cx)
                    });
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    let _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            Notification::error(format!("无法打开文件选择器: {error}"))
                                .title("选择失败"),
                            cx,
                        );
                    });
                }
                Err(_) => {}
            }),
        );
    }

    fn pick_world_restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(version) = self.selected_version().cloned() else {
            return;
        };
        let instance = version.instance_layout(self.config.minecraft_root.clone());
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择备份 ZIP".into()),
        });
        self._prompt_task = Some(
            cx.spawn_in(window, async move |this, cx| match prompt.await {
                Ok(Ok(Some(paths))) => {
                    let Some(path) = paths.into_iter().next() else {
                        return;
                    };
                    let _ = this.update_in(cx, |view, window, cx| {
                        view.start_world_import(path, true, instance.clone(), window, cx)
                    });
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    let _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            Notification::error(format!("无法打开文件选择器: {error}"))
                                .title("选择失败"),
                            cx,
                        );
                    });
                }
                Err(_) => {}
            }),
        );
    }

    fn start_world_import(
        &mut self,
        path: PathBuf,
        restore: bool,
        instance: InstanceLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.world_actions_pending() {
            return;
        }
        let cancellation = CancellationToken::new();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        self.world_action_pending = true;
        self.world_progress = None;
        self.world_cancel = Some(cancellation.clone());
        self._world_progress_task = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = receiver.recv().await {
                let _ = this.update_in(cx, |view, _, cx| {
                    view.world_progress = Some(event);
                    cx.notify();
                });
            }
        }));
        let services = self.services.clone();
        let title = if restore {
            "恢复备份失败"
        } else {
            "导入地图失败"
        };
        self._world_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = services
                .runtime
                .spawn(async move {
                    if restore {
                        restore_instance_world(
                            instance,
                            path,
                            ImportWorldOptions::default(),
                            cancellation,
                            Some(sender),
                        )
                        .await
                    } else {
                        import_instance_world(
                            instance,
                            path,
                            ImportWorldOptions::default(),
                            cancellation,
                            Some(sender),
                        )
                        .await
                    }
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.world_action_pending = false;
                view.world_cancel = None;
                view.world_progress = None;
                view._world_progress_task = None;
                match result {
                    Ok(Ok(world)) => {
                        window.push_notification(
                            Notification::success(format!("已导入「{}」。", world.name)).title(
                                if restore {
                                    "备份已恢复"
                                } else {
                                    "地图已导入"
                                },
                            ),
                            cx,
                        );
                        view.load_worlds(window, cx);
                    }
                    Ok(Err(error)) => {
                        window.push_notification(
                            Notification::error(error.to_string()).title(title),
                            cx,
                        );
                        view.load_worlds(window, cx);
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(format!("后台任务失败: {error}")).title(title),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn pick_world_export(&mut self, world: World, window: &mut Window, cx: &mut Context<Self>) {
        let Some(version) = self.selected_version().cloned() else {
            return;
        };
        let instance = version.instance_layout(self.config.minecraft_root.clone());
        let suggested = format!("{}.zip", world.folder_name());
        let prompt = cx.prompt_for_new_path(&version.game_dir, Some(&suggested));
        self._prompt_task = Some(
            cx.spawn_in(window, async move |this, cx| match prompt.await {
                Ok(Ok(Some(path))) => {
                    let _ = this.update_in(cx, |view, window, cx| {
                        view.start_world_export(world.clone(), path, instance.clone(), window, cx)
                    });
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    let _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            Notification::error(format!("无法打开保存对话框: {error}"))
                                .title("选择失败"),
                            cx,
                        );
                    });
                }
                Err(_) => {}
            }),
        );
    }

    fn start_world_export(
        &mut self,
        world: World,
        destination: PathBuf,
        instance: InstanceLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.world_actions_pending() {
            return;
        }
        let cancellation = CancellationToken::new();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        self.world_action_pending = true;
        self.world_progress = None;
        self.world_cancel = Some(cancellation.clone());
        self._world_progress_task = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = receiver.recv().await {
                let _ = this.update_in(cx, |view, _, cx| {
                    view.world_progress = Some(event);
                    cx.notify();
                });
            }
        }));
        let services = self.services.clone();
        let world_id = world.id.clone();
        let destination_label = display_path(&destination);
        self._world_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = services
                .runtime
                .spawn(async move {
                    export_instance_world(
                        instance,
                        world_id,
                        destination,
                        cancellation,
                        Some(sender),
                    )
                    .await
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.world_action_pending = false;
                view.world_cancel = None;
                view.world_progress = None;
                view._world_progress_task = None;
                match result {
                    Ok(Ok(_)) => {
                        window.push_notification(
                            Notification::success(format!("已导出到 {destination_label}。"))
                                .title("地图已导出"),
                            cx,
                        );
                    }
                    Ok(Err(error)) => {
                        window.push_notification(
                            Notification::error(error.to_string()).title("导出地图失败"),
                            cx,
                        );
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(format!("后台任务失败: {error}"))
                                .title("导出地图失败"),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn backup_world_action(&mut self, world: World, window: &mut Window, cx: &mut Context<Self>) {
        if self.world_actions_pending() {
            return;
        }
        let Some(version) = self.selected_version().cloned() else {
            return;
        };
        let instance = version.instance_layout(self.config.minecraft_root.clone());
        let cancellation = CancellationToken::new();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        self.world_action_pending = true;
        self.world_progress = None;
        self.world_cancel = Some(cancellation.clone());
        self._world_progress_task = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = receiver.recv().await {
                let _ = this.update_in(cx, |view, _, cx| {
                    view.world_progress = Some(event);
                    cx.notify();
                });
            }
        }));
        let services = self.services.clone();
        let world_id = world.id.clone();
        let world_name = world.name.clone();
        self._world_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = services
                .runtime
                .spawn(async move {
                    backup_instance_world(instance, world_id, cancellation, Some(sender)).await
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.world_action_pending = false;
                view.world_cancel = None;
                view.world_progress = None;
                view._world_progress_task = None;
                match result {
                    Ok(Ok(backup)) => {
                        window.push_notification(
                            Notification::success(format!(
                                "「{}」已备份到 {}。",
                                world_name,
                                display_path(&backup.path)
                            ))
                            .title("备份完成"),
                            cx,
                        );
                    }
                    Ok(Err(error)) => {
                        window.push_notification(
                            Notification::error(error.to_string()).title("备份地图失败"),
                            cx,
                        );
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(format!("后台任务失败: {error}"))
                                .title("备份地图失败"),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn confirm_delete_world(&mut self, world: World, window: &mut Window, cx: &mut Context<Self>) {
        let name = world.name.clone();
        let world_id = world.id.clone();
        let view = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .confirm()
                .title(format!("删除地图「{name}」？"))
                .description("只删除该世界目录，无法撤销。")
                .ok_text("删除")
                .ok_variant(ButtonVariant::Danger)
                .on_ok({
                    let view = view.clone();
                    let world_id = world_id.clone();
                    move |_, window, cx| {
                        let Some(view) = view.upgrade() else {
                            return true;
                        };
                        view.update(cx, |this, cx| {
                            this.start_world_delete(world_id.clone(), window, cx)
                        });
                        true
                    }
                })
        });
    }

    fn start_world_delete(
        &mut self,
        world_id: WorldId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.world_actions_pending() {
            return;
        }
        let Some(version) = self.selected_version().cloned() else {
            return;
        };
        let instance = version.instance_layout(self.config.minecraft_root.clone());
        let services = self.services.clone();
        self.world_action_pending = true;
        self._world_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = services
                .runtime
                .spawn(async move { delete_instance_world(instance, world_id).await })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.world_action_pending = false;
                match result {
                    Ok(Ok(())) => {
                        window.push_notification(
                            Notification::success("地图已删除。").title("地图"),
                            cx,
                        );
                        view.load_worlds(window, cx);
                    }
                    Ok(Err(error)) => {
                        window.push_notification(
                            Notification::error(error.to_string()).title("删除地图失败"),
                            cx,
                        );
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(format!("后台任务失败: {error}"))
                                .title("删除地图失败"),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

fn compatibility_tag(compatibility: WorldCompatibility) -> Tag {
    match compatibility {
        WorldCompatibility::Same => Tag::success().child(compatibility.label()).small(),
        WorldCompatibility::Upgrade => Tag::warning().child(compatibility.label()).small(),
        WorldCompatibility::Downgrade => Tag::danger().child(compatibility.label()).small(),
        WorldCompatibility::Unknown => Tag::new().outline().child(compatibility.label()).small(),
    }
}

fn world_stat(label: &str, value: String, cx: &App) -> impl IntoElement {
    v_flex()
        .w(rems(7.))
        .flex_shrink_0()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label.to_owned()),
        )
        .child(div().text_sm().font_semibold().truncate().child(value))
}
