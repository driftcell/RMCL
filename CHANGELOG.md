# Changelog

All notable changes to RMCL are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.7.1](https://github.com/driftcell/RMCL/compare/v0.7.0...v0.7.1) - 2026-10-27

### Fixed

- *(home)* align quick-connect player count and action ([#58](https://github.com/driftcell/RMCL/pull/58))

## [0.7.0](https://github.com/driftcell/RMCL/compare/v0.6.1...v0.7.0) - 2026-10-27

### Added

- *(home)* show live server status in quick connect ([#57](https://github.com/driftcell/RMCL/pull/57))

### Fixed

- *(skin)* stabilize preview layout and interaction ([#55](https://github.com/driftcell/RMCL/pull/55))

## [0.6.1](https://github.com/driftcell/RMCL/compare/v0.6.0...v0.6.1) - 2026-10-27

### Fixed

- *(download)* run mirror probe on the Tokio runtime ([#53](https://github.com/driftcell/RMCL/pull/53))

## [0.6.0](https://github.com/driftcell/RMCL/compare/v0.5.1...v0.6.0) - 2026-10-27

### Added

- *(download)* prefer BMCLAPI with official fallback ([#51](https://github.com/driftcell/RMCL/pull/51))

## [0.5.1](https://github.com/driftcell/RMCL/compare/v0.5.0...v0.5.1) - 2026-10-27

### Changed

- *(ui)* align layout with GPUI guides ([#46](https://github.com/driftcell/RMCL/pull/46))

### Fixed

- *(launch)* materialize missing natives before launch ([#50](https://github.com/driftcell/RMCL/pull/50))

## [0.5.0](https://github.com/driftcell/RMCL/compare/v0.4.5...v0.5.0) - 2026-10-27

### Added

- *(skin)* add official skin page with 3D preview ([#25](https://github.com/driftcell/RMCL/pull/25))
- *(home)* add quick connect for saved servers ([#19](https://github.com/driftcell/RMCL/pull/19))
- *(ui)* use searchable version picker for new instances ([#18](https://github.com/driftcell/RMCL/pull/18))

## [0.4.5](https://github.com/driftcell/RMCL/compare/v0.4.4...v0.4.5) - 2026-10-27

### Fixed

- install legacy assets and Windows Java runtimes ([#22](https://github.com/driftcell/RMCL/pull/22))

## [0.4.4](https://github.com/driftcell/RMCL/compare/v0.4.3...v0.4.4) - 2026-10-26

### Added

- *(ui)* use app icon in title bar ([#15](https://github.com/driftcell/RMCL/pull/15))

## [0.4.3](https://github.com/driftcell/RMCL/compare/v0.4.2...v0.4.3) - 2026-10-26

### Added

- *(instance)* add mod loaders to existing instances ([#11](https://github.com/driftcell/RMCL/pull/11))

### Changed

- *(ui)* streamline account and content search controls ([#9](https://github.com/driftcell/RMCL/pull/9))

## [0.4.2](https://github.com/driftcell/RMCL/compare/v0.4.1...v0.4.2) - 2026-10-26

### Added

- *(servers)* add curated server discovery and direct join ([#6](https://github.com/driftcell/RMCL/pull/6))

### Fixed

- *(java)* run Java discovery on the core runtime ([#8](https://github.com/driftcell/RMCL/pull/8))

## [0.4.1](https://github.com/driftcell/RMCL/compare/v0.4.0...v0.4.1) - 2026-10-26

### Fixed

- *(java)* clean Windows-only compiler warnings ([#2](https://github.com/driftcell/RMCL/pull/2))

## [0.4.0] - 2026-10-26

### Added

- GitHub Release 自动更新：启动检查、手动检查、SHA-256 校验、平台包替换和重启。
- 服务器收藏与 Java Edition Server List Ping：MOTD、在线人数、延迟、图标缓存、自动刷新和 Quick Play/legacy 一键进服。
- 正式版推荐和安装前置预检：本地 Mod 元数据索引、远程主 Mod JAR 元数据、常见 semver/Maven 版本范围、重复/冲突提示和已满足前置跳过。
- 配置 schema 4：新增 `auto_check_updates`；旧 schema 1/2/3 在内存中升级。

### Fixed

- 远程资源包和光影的版本列表不再错误套用实例 Mod 加载器。
- 内容安装任务运行期间禁止本地内容并发编辑。
- 服务器地址解析会尝试全部 DNS 地址，避免首个地址失败后直接放弃。

### Known limitations

- 自动更新目前只有 SHA-256，没有独立签名；首个包含 updater 的版本仍需手动升级一次。
- 真实 Windows/Linux/macOS 自动更新重启、服务器一键进服和真实窗口交互仍需逐平台手工验收。

## [0.3.0] - 2026-10-26

### Added

- 服务器列表、Server List Ping、服务器图标和 60 秒自动刷新。
- 服务器一键进服：1.20+ Quick Play，旧版本 `--server`/`--port`。

## [0.2.0] - 2026-10-26

### Added

- Modrinth 远程内容、必需依赖解析、内容安装任务和本地内容管理。
- 实例隔离、加载器安装、实例级 Java/内存/窗口设置等基础能力。

### Fixed

- 远程资源包/光影版本过滤和内容安装并发编辑问题。
