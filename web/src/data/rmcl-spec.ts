export type AccentKey = "emerald" | "ocean" | "violet" | "amber" | "rose";

export interface AccentPalette {
  key: AccentKey;
  name: string;
  enName: string;
  primary: string;
  hover: string;
  active: string;
  lightPrimary: string;
  surfaceTint: string;
  deepSurface: string;
  specular: string;
  ink: string;
}

/**
 * Color tokens grounded directly in `apps/rmcl/src/theme.rs` and `packaging/icons/rmcl.svg`.
 */
export const ACCENT_PALETTES: Record<AccentKey, AccentPalette> = {
  emerald: {
    key: "emerald",
    name: "翡翠",
    enName: "Emerald",
    primary: "#4ADE94",
    hover: "#6BE7A8",
    active: "#34C97F",
    lightPrimary: "#1F8A5B",
    surfaceTint: "#2B5F46",
    deepSurface: "#173B2A",
    specular: "#8AF7BC",
    ink: "#07130C",
  },
  ocean: {
    key: "ocean",
    name: "海蓝",
    enName: "Ocean",
    primary: "#60A5FA",
    hover: "#7DB6FF",
    active: "#3B82F6",
    lightPrimary: "#2563EB",
    surfaceTint: "#1E4273",
    deepSurface: "#102542",
    specular: "#93C5FD",
    ink: "#071120",
  },
  violet: {
    key: "violet",
    name: "紫罗兰",
    enName: "Violet",
    primary: "#A78BFA",
    hover: "#B9A5FF",
    active: "#8B5CF6",
    lightPrimary: "#7C3AED",
    surfaceTint: "#3F276E",
    deepSurface: "#22133F",
    specular: "#DDD6FE",
    ink: "#120A24",
  },
  amber: {
    key: "amber",
    name: "琥珀",
    enName: "Amber",
    primary: "#FBBF24",
    hover: "#FCD34D",
    active: "#F59E0B",
    lightPrimary: "#B45309",
    surfaceTint: "#664011",
    deepSurface: "#3B2206",
    specular: "#FDE68A",
    ink: "#1A1203",
  },
  rose: {
    key: "rose",
    name: "玫瑰",
    enName: "Rose",
    primary: "#FB7185",
    hover: "#FDA4AF",
    active: "#F43F5E",
    lightPrimary: "#BE123C",
    surfaceTint: "#6B1931",
    deepSurface: "#3D0C1B",
    specular: "#FECDD3",
    ink: "#1F0710",
  },
};

export interface VoxelNodeSpec {
  index: number;
  coords: [number, number, number]; // [-1|0|1, -1|0|1, -1|0|1]
  crateName: string;
  layerLabel: string;
  role: string;
  artifactPath: string;
  isRightRMark: boolean;
  isCoreAnchor: boolean;
}

const CRATE_MODULES: Array<{
  crateName: string;
  role: string;
  artifactPath: string;
}> = [
  {
    crateName: "apps/rmcl",
    role: "GPUI Kit 桌面壳与即时状态投影",
    artifactPath: "apps/rmcl/src/app/mod.rs",
  },
  {
    crateName: "rmcl-app",
    role: "Tokio 多线程任务编排与单任务互斥锁",
    artifactPath: "crates/rmcl-app/src/lib.rs",
  },
  {
    crateName: "rmcl-install",
    role: "Vanilla / Fabric / Quilt / Forge / NeoForge 继承与安装计划",
    artifactPath: "crates/rmcl-install/src/lib.rs",
  },
  {
    crateName: "rmcl-instance",
    role: "rmcl.instance.toml 独立工作区与旧版共享迁移",
    artifactPath: "crates/rmcl-instance/src/lib.rs",
  },
  {
    crateName: "rmcl-content",
    role: "Modrinth 依赖范围预检、本地内容扫描与 options.txt",
    artifactPath: "crates/rmcl-content/src/lib.rs",
  },
  {
    crateName: "rmcl-launch",
    role: "快速启动路径、Quick Play 直连进服与参数脱敏",
    artifactPath: "crates/rmcl-launch/src/lib.rs",
  },
  {
    crateName: "rmcl-auth",
    role: "Microsoft 设备代码认证、系统钥匙串与离线稳定 UUID",
    artifactPath: "crates/rmcl-auth/src/lib.rs",
  },
  {
    crateName: "rmcl-java",
    role: "本机 Runtime 深度探测与 Mojang 官方 JVM 组件复用",
    artifactPath: "crates/rmcl-java/src/lib.rs",
  },
  {
    crateName: "rmcl-net",
    role: "并发分块下载、SHA-1/SHA-256 校验与 Server List Ping",
    artifactPath: "crates/rmcl-net/src/lib.rs",
  },
  {
    crateName: "rmcl-archive",
    role: "临时目录 staging、路径逃逸防护与原子解压",
    artifactPath: "crates/rmcl-archive/src/lib.rs",
  },
  {
    crateName: "rmcl-minecraft",
    role: "Mojang 版本清单解析、OS 规则裁剪与标准仓库布局",
    artifactPath: "crates/rmcl-minecraft/src/lib.rs",
  },
  {
    crateName: "rmcl-store",
    role: "Schema v4 配置文件、同目录 sync 与原子替换",
    artifactPath: "crates/rmcl-store/src/lib.rs",
  },
  {
    crateName: "rmcl-platform",
    role: "macOS Keychain / Windows Credential / Secret Service 适配",
    artifactPath: "crates/rmcl-platform/src/lib.rs",
  },
  {
    crateName: "rmcl-domain",
    role: "跨 crate 领域原语、哈希摘要与不可变契约",
    artifactPath: "crates/rmcl-domain/src/lib.rs",
  },
];

/**
 * Right face (+X = 1) 3x3 grid pattern forming a stylized geometric "R" + play motif
 * in homage to `packaging/icons/rmcl.svg`.
 */
function isRightFaceRCell(y: number, z: number): boolean {
  // On the +X face, z goes from +1 (front-left of right face) to -1 (back-right of right face),
  // y goes from +1 (top) to -1 (bottom).
  if (z === 1) return true; // Left vertical stem of R (y = -1, 0, +1)
  if (y === 1 && z === 0) return true; // Top bar of R
  if (y === 0 && z === -1) return true; // Bowl nose / play apex
  if (y === 0 && z === 0) return true; // Middle crossbar
  if (y === -1 && z === -1) return true; // Diagonal leg of R
  return false;
}

export const VOXEL_NODES: VoxelNodeSpec[] = (() => {
  const nodes: VoxelNodeSpec[] = [];
  let idx = 0;
  for (let y = 1; y >= -1; y--) {
    for (let z = 1; z >= -1; z--) {
      for (let x = -1; x <= 1; x++) {
        const mod = CRATE_MODULES[idx % CRATE_MODULES.length];
        const layerLabel =
          y === 1
            ? "L2 / 加载器与桌面交互层"
            : y === 0
              ? "L1 / 实例隔离与内容编排层"
              : "L0 / 共享存储与系统底座层";
        nodes.push({
          index: idx,
          coords: [x, y, z],
          crateName: mod.crateName,
          layerLabel,
          role: mod.role,
          artifactPath: mod.artifactPath,
          isRightRMark: x === 1 && isRightFaceRCell(y, z),
          isCoreAnchor:
            (x === 1 && y === 1 && z === 1) ||
            (x === -1 && y === 0 && z === 1) ||
            (x === 1 && y === -1 && z === -1) ||
            (x === 0 && y === 0 && z === 0),
        });
        idx++;
      }
    }
  }
  return nodes;
})();

export interface CalloutSpec {
  voxelIndex: number;
  code: string;
  label: string;
  detail: string;
  side: "left" | "right";
}

export interface ChapterSpec {
  id: string;
  index: number;
  range: [number, number];
  stateCode: string;
  shortTitle: string;
  headline: string;
  lead: string;
  metrics: Array<{ value: string; unit: string; caption: string }>;
  terminalSpec: {
    path: string;
    lines: Array<{ key: string; value: string; highlight?: boolean }>;
  };
  callouts: CalloutSpec[];
}

export const CHAPTERS: ChapterSpec[] = [
  {
    id: "monolith",
    index: 0,
    range: [0.0, 0.2],
    stateCode: "STATE 00 / ASSEMBLED CORE",
    shortTitle: "原生核心",
    headline: "脱离 JVM 自举的原生 Minecraft 启动器",
    lead: "RMCL 使用 Rust 2024 与 GPUI Kit 直接编译为平台原生二进制。启动器本身零虚拟机开销，由单窗口持久化的 Tokio 多线程运行时驱动全部安装、校验与进程生命周期。",
    metrics: [
      { value: "14", unit: "Crates", caption: "严格解耦领域逻辑与 GPUI 视图层" },
      { value: "0", unit: "JVM 预装依赖", caption: "启动器自身原生运行，按需托管游戏 Java" },
      { value: "v0.4.2", unit: "MIT 许可", caption: "独立架构实现，原地兼容标准 .minecraft" },
    ],
    terminalSpec: {
      path: "Workspace Topology / Cargo.toml",
      lines: [
        { key: "shell", value: "apps/rmcl (gpui-kit 0.8.0 + Metal / Vulkan / DirectX)" },
        { key: "runtime", value: "rmcl-app::TaskManager (Tokio multi-thread broadcast)" },
        { key: "storage", value: "~/.config/rmcl/config.toml (Schema v4 atomic sync)", highlight: true },
      ],
    },
    callouts: [
      {
        voxelIndex: 2, // [1, 1, 1] top-front-right
        code: "apps/rmcl",
        label: "GPUI Kit 原生渲染壳",
        detail: "即时导航切换与常驻底栏进度，后台任务脱离页面生命周期",
        side: "right",
      },
      {
        voxelIndex: 13, // [0, 0, 0] center core
        code: "rmcl-app",
        label: "单任务互斥编排核",
        detail: "统一调度安装、修复、内容下载与启动，杜绝并发写爆仓库",
        side: "left",
      },
    ],
  },
  {
    id: "strata",
    index: 1,
    range: [0.2, 0.42],
    stateCode: "STATE 01 / STRATIFIED ISOLATION",
    shortTitle: "分层隔离",
    headline: "实例工作区彻底独立，底层资源只存一份",
    lead: "纵向拆解 3×3×3 核心：上层解析 Vanilla、Fabric、Quilt、Forge 与 NeoForge 继承树；中层通过 rmcl.instance.toml 将每个实例的 Mods、光影、资源包与存档严格隔离；底层共享不可变的 libraries 与 assets 对象池。",
    metrics: [
      { value: "5", unit: "加载器体系", caption: "Vanilla / Fabric / Quilt / Forge / NeoForge" },
      { value: "3", unit: "分层解耦", caption: "加载器 Profile / 独立实例目录 / 共享库与资产" },
      { value: "100%", unit: "安全回退", caption: "旧版共享实例支持非破坏式一键复制迁移" },
    ],
    terminalSpec: {
      path: "versions/1.21.1-fabric/rmcl.instance.toml",
      lines: [
        { key: "L2 profile", value: "inheritsFrom = \"1.21.1\" + fabric-loader-0.16.9", highlight: true },
        { key: "L1 isolated", value: "versions/<id>/{mods, resourcepacks, shaderpacks, saves}" },
        { key: "L0 shared", value: ".minecraft/{libraries, assets/indexes, assets/objects}" },
      ],
    },
    callouts: [
      {
        voxelIndex: 1, // [0, 1, 1] top layer
        code: "L2 / rmcl-install",
        label: "版本继承与 Processor 合成",
        detail: "自动合并父版本参数并执行现代 Forge / NeoForge 安装处理器",
        side: "right",
      },
      {
        voxelIndex: 10, // [0, 0, 1] middle layer
        code: "L1 / rmcl-instance",
        label: "实例级版本隔离",
        detail: "独立的 mods 与 options.txt，互不污染其他版本配置",
        side: "left",
      },
      {
        voxelIndex: 22, // [0, -1, 0] bottom layer
        code: "L0 / rmcl-minecraft",
        label: "共享 Libraries & Assets",
        detail: "删除隔离实例仅移除 versions/<id>，完整保留公共依赖",
        side: "right",
      },
    ],
  },
  {
    id: "conduit",
    index: 2,
    range: [0.42, 0.64],
    stateCode: "STATE 02 / VERIFIED PIPELINE",
    shortTitle: "校验管线",
    headline: "写入磁盘前，先完成依赖预检与哈希闭环",
    lead: "从 Modrinth 获取远程内容时，RMCL 会预读远端 JAR 内置元数据与本地 Mod 的 Maven / SemVer 版本范围，在下载前拦截冲突与重复 ID。所有制品经暂存区校验 SHA-1 / SHA-256 通过后才原子发布到目标路径。",
    metrics: [
      { value: "SHA-1", unit: "全链路校验", caption: "客户端、库文件、资源对象与 Modrinth 远端制品" },
      { value: "SemVer", unit: "前置预检", caption: "自动解析 required 依赖并比对本地已安装版本" },
      { value: "Atomic", unit: "暂存发布", caption: "哈希不匹配或中途取消绝不留残损文件" },
    ],
    terminalSpec: {
      path: "rmcl-content & rmcl-net Execution Trace",
      lines: [
        { key: "01 preflight", value: "Inspect remote JAR fabric.mod.json / mods.toml ranges" },
        { key: "02 resolve", value: "Build DAG of required dependencies, skip satisfied IDs", highlight: true },
        { key: "03 publish", value: "Verify SHA-1 in staging -> atomic rename into instance" },
      ],
    },
    callouts: [
      {
        voxelIndex: 4,
        code: "rmcl-content",
        label: "远程 JAR 元数据预检",
        detail: "安装前给出「将安装 / 已满足 / 版本冲突」明确判定",
        side: "left",
      },
      {
        voxelIndex: 14,
        code: "rmcl-net",
        label: "并发下载与摘要核验",
        detail: "支持随时取消任务，未通过校验的字节流绝不落盘发布",
        side: "right",
      },
      {
        voxelIndex: 24,
        code: "rmcl-archive",
        label: "安全解压边界",
        detail: "拒绝符号链接与路径遍历，确保 Natives 与压缩包纯净",
        side: "left",
      },
    ],
  },
  {
    id: "aperture",
    index: 3,
    range: [0.64, 0.84],
    stateCode: "STATE 03 / DIRECT LAUNCH CONDUIT",
    shortTitle: "直连启动",
    headline: "毫秒级快速路径启动，一键直达目标服务器",
    lead: "已安装实例启动时跳过全量资源重哈希，仅校验核心客户端、库与 Natives 存在性即可拉起游戏进程。联机面板内置实时 Server List Ping，点击进入即可通过 Quick Play 或传统参数直接进服。",
    metrics: [
      { value: "QuickPlay", unit: "1.20+ 直连", caption: "自动适配 --quickPlayMultiplayer 与旧版 --server" },
      { value: "5s", unit: "实时探针", caption: "展示服务器图标、MOTD、在线人数与毫秒延迟" },
      { value: "Keychain", unit: "系统级凭据", caption: "正版刷新令牌存入系统钥匙串，启动前按需静默续期" },
    ],
    terminalSpec: {
      path: "rmcl-launch::start_launch_installed",
      lines: [
        { key: "auth", value: "Microsoft token refreshed via OS Keychain / Offline UUID" },
        { key: "java", value: "Auto-select compatible runtime (Java 8 / 17 / 21) per version" },
        { key: "args", value: "--quickPlayMultiplayer mc.example.net:25565 [REDACTED TOKEN]", highlight: true },
      ],
    },
    callouts: [
      {
        voxelIndex: 5,
        code: "rmcl-launch",
        label: "已安装快速启动路径",
        detail: "不重复扫描数千个 Asset Objects，缺失基础文件时精确提示修复",
        side: "right",
      },
      {
        voxelIndex: 12,
        code: "rmcl-auth",
        label: "正版皮肤与凭据隔离",
        detail: "仅从 Mojang 官方域拉取合成头像，日志输出强制脱敏令牌",
        side: "left",
      },
    ],
  },
  {
    id: "convergence",
    index: 4,
    range: [0.84, 1.0],
    stateCode: "STATE 04 / NATIVE DISTRIBUTION",
    shortTitle: "获取构建",
    headline: "五个主题强调色，三端原生体验",
    lead: "在下方实时切换 RMCL 内置的五种强调色（翡翠、海蓝、紫罗兰、琥珀、玫瑰），观察 3D 核心与界面语义 Token 的同步响应。可直接下载 GitHub Releases 预编译包，或从源码编译。",
    metrics: [
      { value: "macOS", unit: "ARM64 / Intel", caption: "支持 macOS 15+ 原生窗口外观与自动隐藏恢复" },
      { value: "Windows", unit: "x86_64", caption: "Windows 10+ 原生标题栏与系统凭据管理" },
      { value: "Linux", unit: "x86_64", caption: "Wayland / X11 + Vulkan + Secret Service 集成" },
    ],
    terminalSpec: {
      path: "Quick Start / Terminal",
      lines: [
        { key: "source", value: "cargo run -p rmcl", highlight: true },
        { key: "macos", value: "xattr -dr com.apple.quarantine RMCL.app && open RMCL.app" },
        { key: "update", value: "Built-in GitHub Release SHA-256 verified self-updater" },
      ],
    },
    callouts: [
      {
        voxelIndex: 2,
        code: "rmcl-store",
        label: "五色主题实时持久化",
        detail: "翡翠 / 海蓝 / 紫罗兰 / 琥珀 / 玫瑰，立即生效并原子保存",
        side: "right",
      },
    ],
  },
];

export interface PlatformBuildSpec {
  id: "macos" | "windows" | "linux" | "source";
  label: string;
  arch: string;
  requirement: string;
  commandLabel: string;
  command: string;
  note: string;
}

export const PLATFORM_BUILDS: PlatformBuildSpec[] = [
  {
    id: "macos",
    label: "macOS",
    arch: "Apple Silicon (ARM64) & Intel (x86_64)",
    requirement: "macOS 15+ · Metal 渲染后端",
    commandLabel: "首次打开未公证构建（移除隔离属性）",
    command: "xattr -dr com.apple.quarantine RMCL.app && open RMCL.app",
    note: "支持原生红绿灯控制、系统钥匙串存储正版令牌，以及游戏运行期间自动最小化与退出恢复。",
  },
  {
    id: "windows",
    label: "Windows",
    arch: "x86_64 Native Binary",
    requirement: "Windows 10+ · DirectX / Vulkan",
    commandLabel: "PowerShell 校验下载包 SHA-256",
    command: "Get-FileHash .\\rmcl-windows-x86_64.zip -Algorithm SHA256",
    note: "单文件/便携压缩包分发，内置 GitHub Release 自动更新与 Credential Manager 凭据集成。",
  },
  {
    id: "linux",
    label: "Linux",
    arch: "x86_64 Native Binary",
    requirement: "Wayland / X11 · Vulkan · Secret Service",
    commandLabel: "解压并赋予可执行权限",
    command: "chmod +x ./rmcl && ./rmcl",
    note: "遵循 XDG 规范（~/.config/rmcl/config.toml），复用已有 ~/.minecraft 仓库结构。",
  },
  {
    id: "source",
    label: "从源码编译",
    arch: "Rust Stable Toolchain (Edition 2024)",
    requirement: "Cargo Workspace · GPUI Kit 0.8.0",
    commandLabel: "克隆并直接启动桌面端",
    command: "git clone https://github.com/driftcell/RMCL.git && cd RMCL && cargo run -p rmcl",
    note: "核心逻辑与 UI 完全解耦，可使用 cargo nextest run --workspace --locked 运行全量单元与契约测试。",
  },
];
