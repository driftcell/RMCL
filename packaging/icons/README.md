# RMCL 图标

原创图标，不使用 Mojang、HMCL 或其他项目的图标、纹理或模型。工作台构图只作为等距方块的抽象参考。

## 源文件

- `rmcl.svg`：主图标。透明背景、3/4 等距方块、顶面 3×3 工作区、右面 R/播放三角标记。
- `rmcl-small.svg`：16–64px 用的高对比 2D 版本。翡翠圆角底 + 深色 R，避免小尺寸下的 3D 细节糊成一团。

## 生成物

- `rmcl.icns`：macOS，小尺寸使用 2D 标记，128px 起使用 3D 方块。
- `rmcl.ico`：Windows，包含 16/24/32/48/64/128/256px PNG 条目。
- `png/rmcl-<size>.png`：16/24/32/48/64 使用 2D 版本，128/256/512/1024 使用透明 3D 版本。

尺寸策略：

| 尺寸 | 源文件 |
|---|---|
| 16 / 24 / 32 / 48 / 64 | `rmcl-small.svg` |
| 128 / 256 / 512 / 1024 | `rmcl.svg` |

## 重新生成

在 macOS 上运行：

```bash
packaging/icons/generate.sh
```

脚本依赖 `sips`、`iconutil` 和 `python3`。生成的 PNG 保留透明通道；`rmcl.ico` 使用标准库直接写入 PNG 条目。

## 平台使用

- macOS：`Info.plist` 的 `CFBundleIconFile` 指向 `rmcl.icns`。直接运行开发二进制时，`apps/rmcl/src/main.rs` 会通过 `NSApplication.setApplicationIconImage` 设置 Dock 图标。
- Windows：把 `rmcl.ico` 作为可执行文件资源嵌入；窗口图标不能代替 exe 资源。
- Linux：安装 PNG 到 `hicolor/<size>x<size>/apps/rmcl.png`，运行时同时通过 `WindowOptions.icon` 设置 X11 窗口图标。

## 设计约束

- 主图标透明背景，悬浮阴影在 1024 画布内。
- 大尺寸保持 R 在方块右半边，顶面网格和右面标记不要压缩到不可读。
- 小尺寸不再保留 3D 细节，只保留清晰的 2D R。
- 所有图标保持深石墨 + 翡翠绿，不跟随用户强调色变化。
