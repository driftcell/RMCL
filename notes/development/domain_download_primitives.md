# 共享下载基础类型

日期：2026-10-25

状态：已决定

## 原因

`rmcl-net` 不应依赖 `rmcl-install`，也不应为了 SHA-1 而依赖整个 Minecraft 元数据层。因此把 `Sha1Hash` 移到新的 `rmcl-domain`，并在这里定义传输无关的 `DownloadRequest`。

`rmcl-minecraft` 继续 re-export SHA-1 类型，避免无意义地破坏已有调用方。`PlannedFile` 提供向 `DownloadRequest` 的 owned 和 borrowed 转换，安装计划与网络执行保持独立。

## 边界

`DownloadRequest` 只描述 URL 候选、目标路径和可选 SHA-1/size，不包含 Minecraft 文件类型、重试策略、镜像策略或 UI 状态。

网络实现及验证记录在后续 `rmcl-net` 任务中完成。
