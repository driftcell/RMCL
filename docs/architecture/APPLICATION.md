# 应用层 Vanilla 安装

状态：`rmcl-app` 安装用例已实现；通用任务管理器和启动编排尚未实现

日期：2026-10-25

## 边界

`rmcl-app::VanillaInstaller` 组合现有元数据、安装计划、下载器和安全解压 API，不重复这些 crate 的领域规则，也不依赖 GPUI。

`VanillaInstallRequest` 要求调用方提供 `.minecraft` 根、精确版本 ID、目标平台和 `VersionSource`：

- `Manifest`：从配置的版本 manifest 查找精确 ID；官方构造器使用 Mojang `version_manifest_v2.json`。
- `LocalJson`：读取本地版本 JSON，先验证 JSON 和版本 ID，再保存原始字节。
- `Existing`：读取仓库中已有的版本 JSON，不访问版本 manifest。

`VanillaSources` 允许注入 manifest 和 asset object 来源根，生产默认与本地 mock 使用同一执行路径。自定义 asset base 不降低 SHA-1/size 校验，也不是完整镜像策略。

仓库根在任务开始转为绝对路径。当前调用方必须串行化同一仓库、版本和平台的安装；稳定任务 ID、重复任务拒绝和应用级任务注册表尚未实现。

## 版本元数据

- manifest 通过 `rmcl-net` 下载到临时目录，复用超时、重试、取消和脱敏错误，不新增 HTTP 客户端。
- 远程版本 JSON 直接下载到标准版本目录；manifest summary 中的 SHA-1 进入下载校验，已验证文件可以跳过。
- 本地 JSON 在发布前完成 UTF-8、serde 和请求 ID 校验；内容变化时使用同目录临时文件、sync 和 persist，原样保留未知字段与格式。
- `Existing` 使用独立的读取阶段，不伪装成写入。
- 版本 JSON 可以提前存在，但它不是安装完成标记；后续 client、libraries 和 assets 仍按计划分别校验。

暂不支持 `inheritsFrom`、`virtual: true` 或 `map_to_resources: true`，这些情况返回结构化错误，不按不完整 Vanilla 继续安装。

## 执行顺序与副作用

1. 解析、读取或保存版本 JSON。
2. 生成并下载 base 计划：client、普通 libraries、native libraries 和 asset index。
3. 读取 asset index，生成并下载 asset objects 计划。
4. 在一个 `spawn_blocking` 中按顺序调用 `merge_zip`，合并全部 native JAR。
5. 返回 `InstallOutcome`。

base 失败时不会下载 assets 或提取 natives；assets 失败时不会提取 natives。已经原子发布的文件不会因后续失败或取消而自动撤回。native merge 保留已有文件，不是损坏修复或过期文件清理。

`InstallOutcome` 保留绝对 `MinecraftLayout`、目标平台、`GameVersion`、base/asset 计划和 native 报告，供后续启动复用。`DownloadSummary` 区分下载与跳过的文件数和字节数，只统计 base 与 asset objects，不包含版本 manifest 或版本 JSON。

## 进度、取消与错误

`InstallPhase` 区分版本解析、读取、保存、base/assets 规划与下载、asset index 读取、native 合并和完成。

- `InstallEvent` 通过有界 Tokio mpsc 通道发送；接收方必须持续消费，事件接收端关闭不会使安装失败。
- 下载事件携带阶段、当前目标、当前文件字节进度、可选总量和终态文件数。终态文件数包含完成、跳过、失败和取消，不能解释为成功数量。
- native 事件按“已合并 JAR 数/总数”报告；不伪造需要预扫描 ZIP 才能获得的全局字节百分比。
- `CancellationToken` 贯穿单文件下载、批量下载、阶段边界和 native 解压。请求取消与最终 `AppError::Cancelled` 分开，取消不映射为普通失败。
- 批量错误只保留目标和脱敏消息，不把原始 `DownloadRequest` 或可能带 query 的 URL 放入应用错误。

高频事件的界面合并、任务快照和广播订阅属于后续任务管理器，不由当前安装用例隐式提供。

## 当前不包含

- 稳定 `TaskId`、任务快照、重复安装互斥和取消状态注册表
- Java 检测后自动获取、启动任务和游戏日志环形缓冲
- 实例隔离、配置与账号持久化
- 版本继承、特殊资产名称重建、ModLoader 和镜像策略
- 完整修复、native 清理、内容寻址缓存和断点续传
- GPUI 页面或窗口生命周期

本地测试使用 mock 覆盖远程 manifest、两阶段下载、两个 native JAR、重复安装复用、取消和本地 JSON 发布；这些测试不访问外部服务。

2026-10-25 使用官方 manifest 和已有真实仓库缓存验证了 `VanillaInstaller::official`：`26.3` 的 5229 个 base/asset 文件全部校验并跳过，`1.7.10` 的 708 个文件全部校验并跳过且 3 个 native JAR 完成合并。该结果验证官方来源解析与复用，不把缓存 smoke 描述为全新下载；新文件下载由上述本地 mock 覆盖。跨平台真实游戏启动仍单独验收。
