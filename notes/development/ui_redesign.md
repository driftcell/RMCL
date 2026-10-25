# 界面重做与启动器配置记录

日期：2026-10-25

## 调研来源

- HMCL 固定基线 `0c3c7dd62ab073bf384fd0f57914388aae1359c4` 的主窗口、主页、版本列表、下载、设置、账户和日志结构。
- [GPUI Kit Components](https://gpui-kit.com/component.md)、[llms.txt](https://gpui-kit.com/llms.txt)、`gpui-kit 0.8.0`、`gpui-component 0.8.0`、`gpui-base 0.8.0` 本地源码。
- `gpui-kit-assets 0.8.0` 的默认图标清单和完整 Lucide 图标目录。
- RMCL 现有 `rmcl-app` 任务、Java、安装、启动和日志 API。

最终事实与界面约束见 [桌面界面与 GPUI 桥接](../../docs/architecture/USER_INTERFACE.md)；配置字段见 [启动器配置](../../docs/architecture/CONFIGURATION.md)。

## HMCL 行为参考

HMCL 使用常驻左侧导航，侧栏同时承载账户和实例上下文；主页以壁纸和单个启动按钮为中心；版本列表、下载向导、设置树和日志窗口各自独立。任务进度使用模态任务对话框，完成后消失；日志窗口通常是单独 Stage。

可借鉴的是：

- 常驻上下文和明确的主启动动作。
- 版本选择不能让用户手填 ID。
- 下载进度、取消和失败必须可见。
- 设置按用途分组，而不是把所有字段堆在一个页面。
- 日志只在用户需要诊断时出现。

不照搬的是：

- 壁纸 + 半透明表面和自定义 stage 替换。
- 八标签设置树、全局预设继承和主题包格式。
- 模态任务对话框作为唯一进度入口。
- HMCL 的 CSS、SVG 路径、壁纸和图标资源。

## 设计决策

### 桌面壳

- 使用 `TitleBar` 自定义标题栏，macOS 保留原生红绿灯。
- 232px 常驻侧栏：主页、版本、下载游戏、设置；账户卡片固定在底部。
- 日志不作为主导航项。仅在任务失败、游戏异常退出和设置“诊断”中进入。
- 底部 `StatusBar` 持有任务阶段、当前文件、进度和停止入口，页面切换不丢失。
- 页面使用即时切换，不加入 HMCL 式过渡动画；优先保证键盘和滚动行为可预测。

### 版本与账户选择

- 已安装版本使用可搜索的 `Select`，账户使用可搜索的 `Select`；不再出现“版本 ID”文本输入。
- 远程版本使用下载页的搜索 + 类型筛选 + `v_virtual_list`，不把 900 多个版本塞进下拉框。
- 缺少客户端 JAR 的版本不隐藏，标记为“文件不完整”，允许启动或检查修复。
- 离线账户使用稳定 UUID；当前不提供删除确认和 Microsoft 登录。

### 视觉

- 默认跟随系统明暗，强调色默认翡翠绿；提供海蓝、紫罗兰、琥珀、玫瑰。
- 颜色、圆角、间距和字体优先使用 GPUI Kit 语义 token；只在图标/边框/虚拟列表行高等物理边界使用像素值。
- 使用 `AllAssets` 获得完整 Lucide 图标；没有引入第三方壁纸或图标资源。
- 主页启动卡片使用主题色的低透明度背景和简洁排版，不使用装饰性壁纸。

### 启动器可见性

- 新增 `launcher_visibility = keep_visible | hide_while_running`。
- 当 `TaskPhase::RunningGame` 且状态为 `Running` 时，如果选择隐藏，则 `Window::minimize_window()`。
- 任务进入终态（完成、取消、失败、退出）时恢复：macOS 通过 `NSApplication` 的 `deminiaturize`，然后 `App::activate` 和 `Window::activate_window`；其他平台使用 `activate_window`。
- 不在游戏进程创建前隐藏，避免安装或 Java 失败时启动器消失。
- 隐藏行为必须在真实 Vanilla 启动中验证，不能以编译通过代替。

### 配置

- 使用 `rmcl-store` 和 TOML，默认 `~/.config/rmcl/config.toml`；Windows 使用 `%APPDATA%\rmcl\config.toml`。
- schema 1；缺失字段使用默认值，损坏或更高版本文件不静默覆盖。
- 配置目录、Java store 和日志目录分离：配置在 XDG/AppData，Java store 仍在 `~/.rmcl/java`，日志在配置目录的 `logs/`。
- 选择“跟随系统”时使用 `Theme::sync_system_appearance`；显式浅色/深色使用 `Theme::change`，并同步 macOS 原生窗口外观。

## 核心补充

- `rmcl-app::discover_installed_versions`：扫描 `versions/<id>/<id>.json`，过滤不安全名称、无效 JSON 和 ID 不匹配项，按发布时间排序。
- `rmcl-app::VersionCatalog`：缓存 Mojang `version_manifest_v2.json`，支持强制刷新和取消。
- `rmcl-store`：TOML 配置、离线账户、启动器可见性、原子写入和 schema 校验。
- UI 侧使用应用级 `TaskManager` 事件桥接，安装完成或退出时刷新版本列表并发送通知。

## 验证

- `rmcl-store`、`rmcl-app` 和 `rmcl` 的单元/集成测试覆盖配置往返、损坏文件保留、版本扫描、manifest 缓存和任务状态。
- 主页、版本、下载、设置、账户对话框和安装确认框已通过真实窗口截图验证；窗口定位改用 macOS `CGWindowID`，避免依赖屏幕坐标。
- 已确认 920 个 Mojang 版本可加载、虚拟列表可滚动、版本/账户下拉框可搜索、设置即时保存。
- 游戏运行时的最小化/恢复尚未用真实 Vanilla 启动验收；Windows/Linux 的 `activate_window` 是否恢复最小化窗口仍需实际测试。

## 未决问题

- macOS 的 `deminiaturize` 需要 `objc2-app-kit` 依赖；Windows/Linux 是否需要平台特定恢复调用。
- 配置损坏后的备份文件和用户确认流程。
- 原生目录选择器、Java runtime 列表与显式覆盖、多账户删除确认。
- 日志诊断包、崩溃原因分析和日志级别过滤。
- 是否引入真正的实例（实例名与版本目录分离）以及实例级设置。
