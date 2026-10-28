# 内置 AI 助手

状态：BYOK 服务商配置、系统钥匙串、进程内 MCP、工具权限确认、AI 页面、活动卡、任务进度和 `tasks_wait` 已实现；对话仅保存在内存，重启后清空。

日期：2026-10-29

## 边界

`rmcl-agent` 是不依赖 GPUI 的应用层服务：

- `rig` 0.45 + `rig-rmcp` 负责模型调用和 MCP 工具适配。
- `rig-reqwest` 使用 reqwest 0.13，而 RMCL 其余网络代码仍使用 0.12；第一版接受两个 reqwest 版本并存，后续统一升级时再收敛。
- `AgentService` 由 `apps/rmcl` 的应用层持有，不挂在 AI 页面上；切换页面不会中断当前回合。
- 模型关闭思考过程输出。中间文字不进入聊天；只有不再调用工具的最终回合通过 `TextViewState` 渲染 Markdown。
- 会话历史和长任务都由应用层持有，页面只消费事件和快照。

## BYOK 与服务商

第一版支持 OpenAI、Anthropic、Google Gemini、DeepSeek、OpenRouter、通义千问、Kimi、智谱 GLM 和自定义 OpenAI-compatible 服务商。每个服务商有默认 Base URL 和默认模型，模型 ID 可在设置中编辑；默认模型必须支持工具调用。

- Provider 配置写入 `config.toml`（schema 7）：`agent_provider`、`agent_model`、`agent_base_url`、`agent_proxy`。
- API Key 只存系统凭据：服务 `com.rmcl.launcher`，键 `agent:<provider>`，值为版本化 JSON `{ "version": 1, "api_key": "..." }`。
- 已保存的 Key 不回填输入框，只显示“已保存在系统钥匙串”和移除按钮。
- 保存时不自动发起模型请求；首次发送消息时验证服务商与 Key。
- 代理支持 HTTP(S)、SOCKS5 和 SOCKS5H；留空时跟随系统/环境代理。代理只影响 AI 请求，不改变全局网络设置。
- 设置卡片明确提示：对话内容和工具结果（实例名、路径、玩家名）会发送给所选服务商。

## MCP 连接

内置 Agent 通过进程内 `rmcp` duplex transport 连接 `McpServer::with_access(ReadWrite)`，与 GUI 共享同一个 `AppContext` 和 `TaskManager`，不启动子进程。

工具在客户端过滤，不修改 MCP 服务端定义：

- 保留实例、版本、内容、Java、账户读取、服务器读取、皮肤读取和任务工具。
- 移除 `tasks_subscribe`、`tasks_unsubscribe` 和 `content_import`；它们对内置模型没有帮助或需要本地文件路径。
- 不向外部 GUI MCP 客户端暴露 `agent_report_status`；它由 `rmcl-agent` 启动的独立小 MCP 服务提供。
- 长任务使用公开的 `tasks_wait` 等待，默认 120 秒、最长 600 秒；超时返回当前快照并标记 `timed_out`；失败附最后 20 行脱敏日志。
- 工具结果过长时截断后交给模型；原始结构化结果仍用于 UI 提取 `task_id`。

## 权限与确认

权限不手写工具清单，而是读取 MCP `ToolAnnotations`：

- `read_only_hint = true`：自动执行。
- `destructive_hint = true`：必须确认。
- 其余写操作默认执行。
- 额外强制确认：启动游戏、账户/皮肤写入、`content_update`、`tasks_cancel` 和未知工具。

确认通过 `AgentHook::on_dispatch` 等待 UI 决定；拒绝时返回 `DispatchAction::skip`，停止时返回 `DispatchAction::stop`。不提供无确认的全自动破坏性模式。

`agent_report_status(message, detail?)` 是内置 Agent 专用的只读空操作，用于更新活动卡文案。系统提示要求它与实际工作工具在同一回合并行调用，避免额外模型轮次。

## 页面与事件

- 侧栏“应用”组新增“AI 助手”；运行中显示 Spinner，等待确认时显示圆点。
- `MessageScroller` 管理对话行；用户消息右对齐 `Filled`，Agent 最终回复为 `Ghost` Bubble + `TextView::new(&state).stream_fade(true)`。
- 三点等待动画用 GPUI `with_animation` 自己实现；系统开启减少动态效果时显示静态点。
- 当前活动使用 `Marker`：运行中 `loading(true)` + `MarkerLoadingStyle::Shimmer`，完成显示绿色 `CircleCheck`，失败显示红色 `CircleX`，跳过显示 `CircleStop`。
- 同一位置原地切换活动内容，不做反复淡入淡出；并行工具调用可同时列出多个步骤。
- 已完成步骤折叠为“已完成 N 个步骤 · 时长”，位于该回合最终回复上方；失败时自动展开。
- 有 `task_id` 时直接读取同一 `TaskManager` 快照显示进度条，不让模型轮询进度。
- 输入使用 `Textarea`（`auto_grow(1, 8)`、`submit_on_enter`）；运行时发送按钮变为停止。
- 会话只保存在内存；提供清空和停止。停止只丢弃当前 Agent 回合，不自动取消已启动的 RMCL 任务。

## 任务终态副作用

GUI 记录自己发起的 `TaskId`。`on_task_terminal` 只对 GUI 发起的任务执行选中实例、页面跳转和保存；Agent 或外部 MCP 发起的任务只刷新版本/内容/配置数据，避免把用户从 AI 页面拽走或覆盖 Agent 刚写入的配置。

Agent 写工具成功后主动刷新一次版本、当前实例内容和外部配置，避免 GUI 内存中的旧配置覆盖 Agent 的写入。

## 会话限制

- 单会话、仅内存；未来持久化可接 rig-memory 的 `DirStore`。
- `max_turns` 限制模型循环；工具结果超长截断。
- 不提供附件、图片输入、语音或多会话。
- Microsoft 登录仍只在 GUI 完成；Agent 只能使用已有账户。

## 验证

- `rmcl-agent` 覆盖 Provider 预设、Base URL/代理校验、Keyring 版本化 JSON、模型构建和系统提示占位符。
- `rmcl-mcp` 覆盖 `tasks_wait` 的终态/运行游戏边界；已有任务通知和只读权限测试继续运行。
- `rmcl-store` 覆盖 schema 6 → 7 默认值和往返。
- AI 页面、确认卡、IME 回车、真实服务商工具调用和多平台窗口仍需人工验收。
