# 内置 AI 助手

状态：Kilo 免费通道、BYOK 服务商配置、系统钥匙串、进程内 MCP、工具权限确认、AI 页面、活动卡、任务进度和 `tasks_wait` 已实现；对话仅保存在内存，重启后清空。

日期：2026-10-29

## 边界

`rmcl-agent` 是不依赖 GPUI 的应用层服务：

- `rig` 0.45 + `rig-rmcp` 负责模型调用和 MCP 工具适配。
- `rig-reqwest` 使用 reqwest 0.13，而 RMCL 其余网络代码仍使用 0.12；第一版接受两个 reqwest 版本并存，后续统一升级时再收敛。
- `AgentService` 由 `apps/rmcl` 的应用层持有，不挂在 AI 页面上；切换页面不会中断当前回合。
- 模型关闭思考过程输出。中间文字不进入聊天；只有不再调用工具的最终回合通过 `TextViewState` 渲染 Markdown。
- 会话历史和长任务都由应用层持有，页面只消费事件和快照。

## 免费通道与 BYOK 服务商

默认服务商是 **Kilo 免费模型**（`kilo`）。它通过 Kilo AI Gateway 的匿名免费池调用 `kilo-auto/free`，不需要 API Key；Rig 使用 `Auth::OptionalBearer`，Key 为空时不发送 `Authorization` 头。

- Kilo Base URL：`https://ai-gateway.kilo.ai/api/v1`；默认模型 `kilo-auto/free` 会自动选择当前可用的免费模型。
- 免费池的模型、限额和隐私策略由 Kilo 控制，可能轮换；免费模型可能记录对话，设置卡片会提示不要发送敏感信息。
- 免费通道遇到 429 时显示“请求过多，请稍后再试或配置自己的 Key”，不暴露原始 HTTP 错误。
- BYOK 仍支持 OpenAI、Anthropic、Google Gemini、DeepSeek、OpenRouter、通义千问、Kimi、智谱 GLM 和自定义 OpenAI-compatible 服务商。
- Provider 配置写入 `config.toml`（schema 8）：`agent_provider`、`agent_model`、`agent_base_url`、`agent_proxy`。
- API Key 只存系统凭据：服务 `com.rmcl.launcher`，键 `agent:<provider>`，值为版本化 JSON `{ "version": 1, "api_key": "..." }`。Kilo 不使用 Keyring。钥匙串读写在同一 `AgentKeyStore` 内串行执行，新读取会等待旧保存/删除完成，避免 UI 状态与实际凭据不一致。
- 已保存的 Key 不回填输入框，只显示“已保存在系统钥匙串”和移除按钮。
- 保存时不自动发起模型请求；首次发送消息时验证服务商与 Key。
- 代理支持 HTTP(S)、SOCKS5 和 SOCKS5H；留空时跟随系统/环境代理。代理只影响 AI 请求，不改变全局网络设置。Base URL 和代理 URL 拒绝内嵌用户名或密码，避免凭据写入 `config.toml` 或进入错误横幅。
- 非法 Provider ID 会显示配置错误并阻止发送，不会静默切换到 Kilo。模型和 Base URL 留空时继续使用 Provider 预设，不会在首次发送后固化默认值。Gemini 的旧 `/v1beta` Base URL 会先归一化，避免升级后重复路径。Provider 错误在显示前会移除原始或 URL 编码后的 API Key。
- 设置卡片明确提示：对话内容和工具结果（实例名、路径、玩家名）会发送给所选服务商。

## MCP 连接

内置 Agent 通过进程内 `rmcp` duplex transport 连接 `McpServer::with_access(ReadWrite)`，与 GUI 共享同一个 `AppContext` 和 `TaskManager`，不启动子进程。该连接独立于 CLI MCP 服务，GUI 不提供额外的 MCP 服务设置。

工具在客户端过滤，不修改 MCP 服务端定义：

- 保留实例、版本、内容、Java、账户读取、服务器读取、皮肤读取和任务工具。
- 移除 `tasks_subscribe`、`tasks_unsubscribe` 和 `content_import`；它们对内置模型没有帮助或需要本地文件路径。
- 不向外部 GUI MCP 客户端暴露 `agent_report_status`；它由 `rmcl-agent` 启动的独立小 MCP 服务提供。
- 长任务使用公开的 `tasks_wait` 等待，默认 120 秒、最长 600 秒；超时返回当前快照并标记 `timed_out`；失败附最后 20 行脱敏日志。
- 工具结果过长时截断后交给模型；任务进度由全局状态栏从 `TaskManager` 读取，AI 页面不解析工具结果中的 `task_id`。

## 权限与确认

权限不手写工具清单，而是读取 MCP `ToolAnnotations`：

- `read_only_hint = true`：自动执行。
- `destructive_hint = true`：必须确认。
- 其余写操作默认执行，但实际 `content_install` 必须确认；`dry_run=true` 的只读预检自动放行。
- 额外强制确认：启动游戏、`content_update`、`tasks_cancel` 和未知工具；策略层也为账户/皮肤等写入保留强制确认。

确认通过 `AgentHook::on_dispatch` 等待 UI 决定；拒绝时返回 `DispatchAction::skip`，停止时返回 `DispatchAction::stop`。不提供无确认的全自动破坏性模式。

`agent_report_status(message, detail?)` 是内置 Agent 专用的只读空操作，用于更新活动卡文案。系统提示要求它与实际工作工具在同一回合并行调用，避免额外模型轮次。

## 页面与事件

- 侧栏“应用”组新增“AI 助手”；运行中显示 Spinner，等待确认时显示圆点。
- `MessageScroller` 管理对话行；用户消息右对齐 `Filled`，Agent 最终回复为 `Ghost` Bubble + `TextView::new(&state).stream_fade(true)`。
- 运行状态使用一张紧凑的活动卡，而不是独立气泡：主色脉冲圆点 + `ShimmerText` 状态文案组成唯一的活动焦点，系统开启减少动态效果时脉冲与微光都保持静态。
- 工具步骤使用真实 `Spinner` 表示运行中，完成后切换为绿色 `CircleCheck`，失败为红色 `CircleX`，跳过为 `CircleStop`；步骤文字从前景色过渡为弱化色，不叠加多个 shimmer 或旋转图标。
- 同一位置原地切换活动内容，不做反复淡入淡出；并行工具调用可同时列出最近三个步骤。
- 工具步骤摘要只显示实例名、游戏或加载器版本、内容路径、服务器地址等用户可识别的值；`task_id`、revision、幂等键和 Modrinth version ID 不进入界面，无法识别的参数不回退为原始 JSON。
- 已完成步骤折叠为“已完成 N 个步骤 · 时长”，位于该回合最终回复上方；失败时自动展开。
- 展开/收起步骤会 remeasure 当前行；回合结束后先 append 再观察 `TextViewState`，并在下一帧 remeasure 最后一行，避免虚拟列表高度过期导致滚轮停不到底部。消息列表末尾保留一个固定高度的占位行，作为虚拟列表可测量的真实尾项，滚轮可以越过最后一行文字；跳转按钮文案为“回到底部”。
- 任务进度只由全局底部状态栏读取同一 `TaskManager` 快照显示，AI 页面活动卡和输入框不再重复显示进度条；模型仍不需要轮询进度。
- 输入使用 `Textarea`（`auto_grow(1, 8)`、`submit_on_enter`）；输入框与发送按钮默认同为 2.5rem 高，输入框增高时按钮保持底部对齐。运行期间仍可编辑草稿，发送按钮变为停止，提交新消息要等当前回合结束。
- 会话只保存在内存；提供清空和停止。清空同时移除 UI 消息和模型历史。停止立即使当前回合代次失效并取消启动、MCP 连接或模型执行，迟到事件不会进入下一回合；不会自动取消已启动的 RMCL 任务。内部任务异常终止会显示错误，不会静默消失。

## 任务终态副作用

GUI 记录自己发起的 `TaskId`。`on_task_terminal` 只对 GUI 发起的任务执行选中实例、页面跳转和保存；Agent 或外部 MCP 发起的任务只刷新版本/内容/配置数据，避免把用户从 AI 页面拽走或覆盖 Agent 刚写入的配置。

Agent 写工具成功后主动刷新一次版本、当前实例内容和外部配置，避免 GUI 内存中的旧配置覆盖 Agent 的写入。

## 会话限制

- 单会话、仅内存；未来持久化可接 rig-memory 的 `DirStore`。
- `max_turns` 限制模型循环；工具结果超长截断。
- 不提供附件、图片输入、语音或多会话。
- Microsoft 登录仍只在 GUI 完成；Agent 只能使用已有账户。

## 验证

- `rmcl-agent` 覆盖 Provider 预设、非法 Provider、Base URL/代理及 userinfo 校验、Keyring 版本化 JSON、模型构建和系统提示占位符；本地 HTTP mock 验证 Kilo 空 Key 不发送 `Authorization`，Gemini 请求只包含一个 `/v1beta` 前缀；另覆盖 Provider 错误脱敏、并行/超时确认、清空模型历史和启动前取消。
- `rmcl-mcp` 覆盖 `tasks_wait` 的终态/运行游戏边界和真实 Java 任务终态等待；已有任务通知和只读权限测试继续运行。
- `rmcl-store` 覆盖 schema 6 → 7 默认值和往返。
- AI 页面、确认卡、IME 回车、真实服务商工具调用和多平台窗口仍需人工验收。
