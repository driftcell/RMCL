# 启动器配置

状态：schema 7 已实现；schema 1/2/3/4/5/6 可读取并在内存中升级

日期：2026-10-28

## 边界

启动器设置由 `rmcl-store` 持久化。它不依赖 GPUI，也不改写 `.minecraft` 版本 JSON 或 HMCL 配置。游戏文件、Java runtime 和任务日志仍由各自的模块管理。

配置和游戏状态分离：删除或重建 `config.toml` 只会丢失启动器偏好、账户元数据和选中项，不会删除 Minecraft 文件或系统钥匙串中的令牌。

实例级元数据不写入 `config.toml`。每个实例的显示名称、基础版本、隔离模式和可选加载器保存在 `versions/<id>/rmcl.instance.toml`，语义见 [实例与版本隔离](INSTANCES.md)。当前 `selected_version` 仍保存实例目录 ID，而不是显示名称。

## 路径

| 平台 | 默认路径 |
|---|---|
| macOS | `~/Library/Application Support/rmcl/config.toml` |
| Linux | `$XDG_CONFIG_HOME/rmcl/config.toml`；未设置时使用 `~/.config/rmcl/config.toml` |
| Windows | `%APPDATA%\rmcl\config.toml` |

基础目录由 `dirs` 按系统规则解析，RMCL 不再直接读取 `HOME`、`USERPROFILE`、`APPDATA` 或 `XDG_CONFIG_HOME`。系统目录无法解析时启动会返回明确错误，不会静默回退到当前工作目录。

默认 Minecraft 仓库：

| 平台 | 默认路径 |
|---|---|
| macOS | `~/Library/Application Support/minecraft` |
| Linux | `~/.minecraft` |
| Windows | `%APPDATA%\.minecraft` |

RMCL 不迁移或合并其他目录中的仓库。

`--config <PATH>` 可以指定其他文件，用于隔离 smoke 和测试。Java store 默认仍是 `~/.rmcl/java`；日志目录是配置文件所在目录下的 `logs/`，皮肤头像和完整纹理缓存位于同目录的 `cache/skins/`，本地皮肤库位于 `skins/library/`，服务器图标缓存位于 `cache/servers/`，Mod 元数据缓存位于 `cache/mod-metadata/`，更新包和 helper 日志位于 `cache/updates/`。

## Schema 7

```toml
schema_version = 7
minecraft_root = "/Users/example/Library/Application Support/minecraft"
java_store = "/Users/example/.rmcl/java"
selected_version = "1.21.1"
selected_account = "Podwarden"
selected_account_kind = "microsoft"
auto_install_java = true
auto_check_updates = true
download_source = "auto"
launcher_visibility = "keep_visible"
mcp_enabled = false
mcp_port = 39271
agent_provider = "kilo"
agent_model = ""
agent_base_url = ""
agent_proxy = ""

[[accounts]]
username = "Steve"
uuid = "5627dd98-e6be-3c21-b8a8-e92344183641"

[[microsoft_accounts]]
username = "Podwarden"
uuid = "97fe2bde-352d-454d-adf9-a0f568a50a3b"
skin_url = "http://textures.minecraft.net/texture/…"

[[servers]]
id = "0f8b0d0e-…"
name = "生存服"
address = "play.example.com:25565"

[appearance]
theme = "system"
accent = "emerald"
```

字段：

- `minecraft_root`：标准 `.minecraft` 仓库根。修改后重新扫描版本，不移动文件。
- `java_store`：RMCL 管理的 Java runtime store。启动时读取，修改后需重启。
- `selected_version`：下一次启动使用的已安装实例目录 ID；不存在时界面回退到第一个有效实例。
- `selected_account`：当前选中的玩家名。
- `selected_account_kind`：`offline` 或 `microsoft`。同名离线账户与正版账户通过该字段区分。
- `accounts`：离线账户的玩家名和 UUID。UUID 使用离线身份规则生成并持久化；不保存访问令牌。
- `microsoft_accounts`：正版账户的玩家名、Minecraft profile UUID 和可选的官方皮肤 URL；不保存 refresh token 或 access token。
- 本地皮肤库不写入 `config.toml`；`skins/library/index.json` 只保存皮肤 ID、显示名、模型提示、来源 URL、导入时间和文件名，完整 PNG、合成头像与索引均通过同目录临时文件原子发布。
- `servers`：启动器级服务器收藏，包含稳定 `id`、显示名称和原始 `host[:port]` 地址。不保存状态、延迟、图标或账号令牌；不绑定实例，进服时使用当前选中实例和账户。
- `auto_install_java`：缺少兼容 Java 时是否允许下载 Mojang runtime。当前在启动时读取。
- `auto_check_updates`：启动后是否自动检查 GitHub Release；默认开启。手动检查不受该字段影响。
- `download_source`：`auto`、`official` 或 `mirror`。`auto` 探测国内镜像，失败或超时使用官方源；`official` 完全不使用镜像；`mirror` 优先国内镜像并逐请求回退官方源。修改后立即生效，无需重启。见 [下载源与国内镜像](DOWNLOAD_SOURCES.md)。
- `launcher_visibility`：`keep_visible` 或 `hide_while_running`。后者在游戏进程进入运行阶段时隐藏启动器（macOS 隐藏应用，其他平台最小化窗口），任务终态恢复。
- `mcp_enabled`：是否在 GUI 内启动 loopback、只读 MCP HTTP 服务，默认 `false`。启用或修改端口后立即协调启停，绑定失败只显示错误，不阻止 GUI 启动。
- `mcp_port`：内嵌 MCP HTTP 端口，默认 `39271`，必须是 1–65535。配置不保存 Bearer token；GUI 不提供无认证写访问。
- `agent_provider`：内置 AI 助手使用的服务商 ID，当前支持 `kilo`、`openai`、`anthropic`、`gemini`、`deepseek`、`openrouter`、`qwen`、`kimi`、`zhipu` 和 `custom`。`kilo` 使用无需 Key 的免费通道，其余服务商需要 API Key。
- `agent_model`：模型 ID；空字符串表示使用所选服务商的默认模型。模型必须支持工具调用。
- `agent_base_url`：可选的 OpenAI-compatible 或原生服务商 Base URL；空字符串表示使用服务商默认值。`custom` 模式必须填写。
- `agent_proxy`：可选的 AI 请求代理，支持 `http://`、`https://`、`socks5://` 和 `socks5h://`；空字符串表示跟随系统/环境代理。
- AI API Key 不写入 `config.toml`，保存在系统凭据存储，服务名为 `com.rmcl.launcher`，键为 `agent:<provider>`，值为 `{ "version": 1, "api_key": "..." }`。`kilo` 免费通道不使用 Keyring。
- `appearance.theme`：`system`、`light` 或 `dark`。
- `appearance.accent`：`emerald`、`ocean`、`violet`、`amber` 或 `rose`。

缺省字段使用默认值；未知字段在读取时被忽略。`schema_version` 必须为 1–7；0 或更高版本会被拒绝。读取 schema 1/2/3/4/5/6 时补齐 `selected_account_kind`、`microsoft_accounts`、空 `servers`、`auto_check_updates`、`download_source`、MCP 设置和 AI 设置，内存中的版本升级为 7，下一次用户修改设置时写回。

正版令牌不在 `config.toml` 中，而是通过 `rmcl-platform` 保存到系统凭据存储：

- 服务名：`com.rmcl.launcher`
- 键：`microsoft:<无连字符 UUID>`
- 值：带版本的 JSON，包含 refresh token、缓存的 Minecraft access token 和过期时间

具体协议和删除语义见 [正版账号、令牌与皮肤](AUTHENTICATION.md)。

## 原子写入

保存流程：

1. 创建配置目录。
2. 在配置目录创建临时文件。
3. 写入 TOML。
4. `sync_all`。
5. 原子替换目标文件。

读取失败不会自动重写文件：

- 不存在：返回默认配置。
- TOML 无效：返回带路径的错误，界面显示“配置读取失败”，原文件保留。
- 更高 schema：返回 `UnsupportedSchema`，原文件保留。
- 后续用户主动修改设置时才可能写入当前 schema。

配置文件只包含离线与正版账户元数据、皮肤 URL、服务器收藏、路径和界面偏好，不包含 Microsoft 令牌、密码或认证服务器凭据。正版令牌只存在于系统凭据存储。

## 迁移

Schema 1 → 2 已在读取时实现：

- `selected_account_kind` 默认为 `offline`。
- `microsoft_accounts` 默认为空。
- 离线账户和选中项原样保留。
- 内存中的版本升级为 2；只有用户主动修改设置时才写回文件，读取本身不重写。

Schema 2 → 3 已在读取时实现：

- `servers` 默认为空。
- 其余字段原样保留。
- 内存中的版本升级为 3；只有用户主动修改设置时才写回文件。

Schema 3 → 4 已在读取时实现：

- `auto_check_updates` 默认为 `true`。
- 其余字段原样保留。
- 内存中的版本升级为 4；只有用户主动修改设置时才写回文件。

Schema 4 → 5 已在读取时实现：

- `download_source` 默认为 `"auto"`。
- 其余字段原样保留。
- 内存中的版本升级为 5；只有用户主动修改设置时才写回文件。

Schema 5 → 6 已在读取时实现：

- `mcp_enabled` 默认为 `false`，`mcp_port` 默认为 `39271`。
- 其余字段原样保留，不写入 token。
- 内存中的版本升级为 6；只有用户主动修改设置时才写回文件。

Schema 6 → 7 已在读取时实现：

- `agent_provider` 默认为 `"kilo"`。
- `agent_model`、`agent_base_url`、`agent_proxy` 默认为空，由所选服务商的预设补全。
- AI API Key 不迁移、不写入配置文件；需要 BYOK 的用户重新在设置中保存。Kilo 免费通道不需要 Key。
- 其余字段原样保留。
- 内存中的版本升级为 7；只有用户主动修改设置时才写回文件。

后续升级 schema 时必须：

- 明确旧版本到新版本的字段映射和默认值。
- 保留未知字段或提供显式丢弃策略。
- 对损坏文件和更高版本继续拒绝写入。
- 在 `rmcl-store` 测试和本文件中同时记录。

## 验证

`rmcl-store` 的单元测试覆盖：

- 默认路径不指向 HMCL 配置。
- 缺失文件返回默认值。
- TOML 往返保留离线账户、正版账户、皮肤 URL、选中账户类型、服务器收藏、自动更新设置、下载源、MCP 开关与端口、外观和启动器可见性。
- Schema 1 文件升级后保留离线账户并补上默认字段。
- Schema 2 文件升级后补上空服务器列表。
- Schema 3 文件升级后补上默认开启的自动更新检查。
- Schema 4 文件升级后补上默认 `auto` 下载源。
- Schema 5 文件升级后补上默认停用的 MCP 和端口 `39271`。
- Schema 6 文件升级后补上默认的 AI 服务商和空的模型/Base URL/代理字段。
- 损坏 TOML 和更高 schema 不被重写。
- 覆盖保存后内容可重新读取。

系统凭据存储、头像缓存和跨平台路径权限仍需在 Windows 和 Linux 上做真实运行验证。
