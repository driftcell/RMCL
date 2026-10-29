# Kilo 免费网关调研

日期：2026-10-29

状态：已实测并接入为默认的无 Key 免费通道。

## 结论

Kilo AI Gateway 提供匿名免费模型池，不需要 API Key，OpenAI-compatible，支持工具调用和流式输出。RMCL 使用它作为普通玩家的默认 AI 后端，BYOK 服务商继续保留。

- Base URL：`https://ai-gateway.kilo.ai/api/v1`
- 默认模型：`kilo-auto/free`
- 认证：匿名请求不带 `Authorization`；有 Key 时按正常 Bearer 处理
- 限额：官方文档写明匿名请求 200 次/小时/IP，按 IP 识别
- 文档：<https://kilo.ai/docs/gateway/authentication>（2026-10-29 读取）

## 实测记录

以下请求均未带 API Key：

- `kilo-auto/free` 普通对话返回 200。
- `kilo-auto/free` 中文工具调用返回 `tool_calls`，参数为 `{"city":"上海"}`。
- 工具结果回传后，模型用中文给出最终答案。
- 流式请求返回 SSE，包含 reasoning 和 content 增量。
- 固定免费模型 `nvidia/nemotron-3-ultra-550b-a55b:free`、`poolside/laguna-s-2.1:free`、`nvidia/nemotron-3.5-lightning:free` 都能返回工具调用。
- 带 `Authorization: Bearer anonymous` 仍返回 200；带 `Bearer public` 返回 401。因此实现里使用空 Key，让 Rig 省略 Authorization 头。

## 限制与风险

1. **免费池会轮换**：模型 ID、可用性和路由目标由 Kilo 控制，`kilo-auto/free` 可能在不同时间落到不同模型。
2. **隐私**：Kilo 文档提示免费模型可能路由到记录 prompt/output 的提供商；设置页必须提示不要发送敏感信息。
3. **限额**：200 次/小时/IP 是匿名上限，多人共享出口 IP 时会互相影响。429 时转为可操作提示，不展示原始 HTTP 错误。
4. **稳定性**：免费通道没有 SLA；需要稳定性的用户应配置自己的 Key。
5. **国内网络**：`ai-gateway.kilo.ai` 走 Vercel，部分网络可能需要代理；RMCL 已有 AI 代理设置。

## 实现选择

- 新增 `AgentProvider::Kilo`，默认 Base URL 和模型如上，`requires_key()` 返回 `false`。
- `AgentService::start_turn` 对无 Key 服务商传空字符串，OpenAI 配置显式使用 `Auth::OptionalBearer`，因此省略 `Authorization` 头；本地 HTTP mock 覆盖此行为。
- `AgentKeyState::Free` 表示当前服务商不需要 Key；设置页隐藏 Key 输入，AI 页不禁用发送按钮。
- 429 / 401 通过 `friendly_error` 转成中文提示。
