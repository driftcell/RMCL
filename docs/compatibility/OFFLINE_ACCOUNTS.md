# 离线账号行为

状态：当前核心行为已实现

日期：2026-10-25

## 身份模型

`rmcl-auth` 提供不依赖 UI 或存储格式的 `OfflineAccount`，包含：

- 原始用户名
- Minecraft profile UUID
- 启动参数使用的无连字符 UUID 形式

账号记录 ID、选中状态、排序、皮肤、Microsoft 会话和具体持久化格式不属于这一层。

## 用户名

- 只拒绝空白用户名。
- 不 trim、不改变大小写、不进行 Unicode 归一化。
- `Steve`、`Steve ` 和 `steve` 是不同身份。
- ASCII 字母数字或下划线组成且长度不超过 16 的名称被标记为常规名称。
- 非常规名称仍可创建，`has_conventional_username` 供 UI 显示警告，不作为核心层拒绝条件。

这保留了 HMCL 的核心宽容行为，同时把界面警告与账号身份分离。

## UUID

默认 UUID 按以下方式独立计算：

1. 将 `OfflinePlayer:<原样用户名>` 编码为 UTF-8。
2. 计算 MD5。
3. 设置 UUID v3 版本位和 RFC 4122 variant 位。
4. 按标准 UUID 字节顺序表示。

`with_uuid` 支持显式保留存储或用户提供的 UUID。持久化实现必须保存最终 UUID，不能在加载时无条件重新按用户名推导。

最小核心不包含离线皮肤、本地 Yggdrasil 服务、authlib-injector 或额外 JVM agent 参数。

行为依据：[HMCL 固定基线](HMCL_BASELINE.md)。实现使用独立编写的测试值，没有复制 HMCL 测试数据。
