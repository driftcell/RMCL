# 远程内容与 Modrinth 取舍

日期：2026-10-25

状态：已形成实现决策，最终规范见 [远程内容与 Modrinth](../../docs/architecture/REMOTE_CONTENT.md)

## 背景

本地内容管理已经提供稳定的内容目录、冲突策略和 `options.txt` 语义。下一步要让 UI 能搜索并安装远程 Mod、资源包和光影。

## 参考行为

HMCL：

- 内容源为 CurseForge 和 Modrinth；CurseForge 需要 API key，Modrinth 不需要。
- 搜索和版本列表分开；每个文件可选 primary file。
- 依赖默认只展示，不自动安装。
- 下载通常直接写入目标目录，部分路径没有远端哈希校验。
- 新实例/内容安装属于长任务，有进度和取消。

## 决策

- 先接 Modrinth：公共 API、无需 key、文档稳定，适合打通搜索、版本、下载、校验和任务链路。
- 元数据客户端放在 `rmcl-content`，只负责 API 和模型；文件下载留在 `rmcl-app`，复用 `rmcl-net`。
- 远端文件先下载到临时目录，验证 SHA-1/size 后再通过本地 `import_file` 发布；不复用远端文件名直接写目标目录。
- 缺少 SHA-1 的远端文件拒绝安装；URL 只接受 HTTP(S)，文件名必须是单一安全路径组件。
- 默认使用 `Rename` 冲突策略，和本地导入一致；不静默覆盖。
- 新增 `TaskKind::ContentInstall`、独立的内容阶段/进度和 `TaskSubject`，不把内容安装伪装成 Vanilla 安装。
- required 依赖在任务内解析并先于主文件安装；optional、incompatible 和 embedded 不自动处理。
- 依赖解析支持精确 `version_id` 和按游戏版本/加载器选择 `project_id`，折叠循环和重复版本，限制 32 个不同版本。
- 已安装判定按目标目录文件名和 SHA-1；`.disabled` Mod 也视为已安装，避免自动重新启用。
- 当前实例没有加载器时仍允许下载 Mod，但显示警告，避免把“文件已安装”误认为“游戏会加载”。

## 2026-10-30 资源包与光影浏览补充

- 内容源继续使用 Modrinth：搜索响应直接提供 `gallery` 和 `featured_gallery`，无需为搜索页逐项目补发详情请求；真实只读请求已确认光影项目返回这两类字段。
- 初稿曾考虑所有内容统一为大图卡，审视后改为仅资源包和光影使用作品画廊，Mod 保留高密度列表，避免视觉装饰压过依赖和加载器信息。
- 图库直接采用 GPUI Kit `Carousel`、`CarouselState`、分页项和前后控件，不实现自定义点击区域、焦点或动画。详情页最多加载 6 张图，精选图优先并去重。
- 小图标缓存不能满足作品预览，新增独立 gallery 缓存目录和尺寸配置；保留 HTTPS/私网校验、内容类型和原子缓存语义，缩略时不裁切宽高比。
- 原生窗口视觉与键盘验收由用户后续执行，本次不以编译或截图缺失宣称完成该部分验证。最终事实见 [远程内容与 Modrinth](../../docs/architecture/REMOTE_CONTENT.md)。

## 暂缓

- CurseForge API key、分发条款和 provider 切换。
- optional/incompatible 依赖处理、依赖冲突求解和用户选择界面。
- 全局已安装文件哈希索引、更新检查和整合包。
- 项目图标、网页跳转、速率限制队列。
- 整合包和 `.mrpack` 的 SHA-512 校验。
