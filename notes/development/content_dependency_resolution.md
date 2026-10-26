# 内容依赖版本匹配与远程元数据取舍

日期：2026-10-26

## 结论

- Fabric/Quilt 版本谓词使用成熟的 [`semver`](https://crates.io/crates/semver) crate，不自行实现 semver 比较。
- Forge/NeoForge 的 Maven 版本排序使用 [`mvn_version`](https://crates.io/crates/mvn_version) 的 `ComparableVersion`；Maven 区间语法较小，由 RMCL 自己解析并用表驱动测试覆盖。
- 暂不引入 `pubgrub` 等通用求解器。Modrinth 的 required 依赖目前是精确版本/项目，现有“解析 + 范围校验 + 安装批次”已够用；等整合包和 optional 依赖需要多版本选择时再评估。
- 远程主 Mod JAR 在预检阶段按 SHA-1 缓存元数据，上限 16 MiB；缺少 SHA-1 或超限时回退到项目 slug/标题匹配。
- 依赖 JAR 不逐个下载；远程依赖仍使用 Modrinth 项目/版本信息，安装后由本地元数据索引参与后续预检。

## 未采用

- `maven-version-range` 0.1.1：功能匹配，但发布仅一个月且采用量很低，暂不作为长期依赖。
- `pubgrub`：MPL-2.0，且当前没有必须的多版本求解需求；引入会增加适配和测试面。
- 手写 semver 解析：容易在预发布、build metadata、caret/tilde 上出错，使用 `semver` crate。

## 风险

- `mvn_version` 最后发布日期较早，但实现的是 Maven `ComparableVersion`，并有独立测试；升级或替换时需重跑范围表测试。
- Fabric/Quilt 谓词翻译只覆盖常见写法；遇到无法解析的范围时预检回退到“存在性 + 字符串版本”判断，并在结果中保留警告。
- Maven 区间解析覆盖 `[a,b)`、`(,b]`、`[a]`、联合区间和精确版本；未覆盖所有 Maven 扩展语法，遇到未知语法返回错误而不是猜测。
