import downloadsShotUrl from "../../public/screenshots/downloads.webp";
import homeShotUrl from "../../public/screenshots/home.webp";
import instancesShotUrl from "../../public/screenshots/instances.webp";
import serversShotUrl from "../../public/screenshots/servers.webp";

export type AccentKey = "emerald" | "ocean" | "violet" | "amber" | "rose";
export type ThemeMode = "light" | "dark";

export interface AccentPalette {
  key: AccentKey;
  name: string;
  enName: string;
  lightPrimary: string;
  lightHover: string;
  lightActive: string;
  lightTint: string;
  lightBorderTint: string;
  darkPrimary: string;
  darkHover: string;
  darkActive: string;
  darkTint: string;
  darkSpecular: string;
}

export const ACCENT_PALETTES: Record<AccentKey, AccentPalette> = {
  emerald: {
    key: "emerald",
    name: "翡翠",
    enName: "Emerald",
    lightPrimary: "#1F8A5B",
    lightHover: "#18744C",
    lightActive: "#12603E",
    lightTint: "#E6F4ED",
    lightBorderTint: "#BFE3D0",
    darkPrimary: "#4ADE94",
    darkHover: "#6BE7A8",
    darkActive: "#34C97F",
    darkTint: "#142E22",
    darkSpecular: "#8AF7BC",
  },
  ocean: {
    key: "ocean",
    name: "海蓝",
    enName: "Ocean",
    lightPrimary: "#2563EB",
    lightHover: "#1D4ED8",
    lightActive: "#1E40AF",
    lightTint: "#E8F0FE",
    lightBorderTint: "#BFDBFE",
    darkPrimary: "#60A5FA",
    darkHover: "#7DB6FF",
    darkActive: "#3B82F6",
    darkTint: "#132442",
    darkSpecular: "#93C5FD",
  },
  violet: {
    key: "violet",
    name: "紫罗兰",
    enName: "Violet",
    lightPrimary: "#7C3AED",
    lightHover: "#6D28D9",
    lightActive: "#5B21B6",
    lightTint: "#F1EAFE",
    lightBorderTint: "#DDD6FE",
    darkPrimary: "#A78BFA",
    darkHover: "#B9A5FF",
    darkActive: "#8B5CF6",
    darkTint: "#231542",
    darkSpecular: "#DDD6FE",
  },
  amber: {
    key: "amber",
    name: "琥珀",
    enName: "Amber",
    lightPrimary: "#B45309",
    lightHover: "#92400E",
    lightActive: "#78350F",
    lightTint: "#FEF3E2",
    lightBorderTint: "#FDE68A",
    darkPrimary: "#FBBF24",
    darkHover: "#FCD34D",
    darkActive: "#F59E0B",
    darkTint: "#382208",
    darkSpecular: "#FDE68A",
  },
  rose: {
    key: "rose",
    name: "玫瑰",
    enName: "Rose",
    lightPrimary: "#BE123C",
    lightHover: "#9F1239",
    lightActive: "#881337",
    lightTint: "#FCE8EE",
    lightBorderTint: "#FECDD3",
    darkPrimary: "#FB7185",
    darkHover: "#FDA4AF",
    darkActive: "#F43F5E",
    darkTint: "#3B0D1B",
    darkSpecular: "#FECDD3",
  },
};

export interface VoxelNodeSpec {
  index: number;
  coords: [number, number, number];
  featureTitle: string;
  categoryLabel: string;
  description: string;
  isRightRMark: boolean;
}

const USER_FEATURES: Array<{
  featureTitle: string;
  description: string;
}> = [
  {
    featureTitle: "一键启动游戏",
    description: "选好实例直接进入世界，游戏运行后启动器可自动最小化待命。",
  },
  {
    featureTitle: "自动配齐 Java",
    description: "无需手动找安装包或配环境变量，按游戏版本自动准备合适的 Java。",
  },
  {
    featureTitle: "微软正版与皮肤",
    description: "支持正版登录并同步官方皮肤头像，离线账号也可随时切换。",
  },
  {
    featureTitle: "默认版本隔离",
    description: "每个实例拥有独立的存档、Mod、材质包与光影文件夹，告别串档。",
  },
  {
    featureTitle: "五大模组加载器",
    description: "点选 Vanilla、Fabric、Quilt、Forge 或 NeoForge 即可自动安装。",
  },
  {
    featureTitle: "精选服务器直连",
    description: "实时查看服务器在线人数与延迟，点击「进入」直接拉起游戏进服。",
  },
  {
    featureTitle: "Modrinth 社区下载",
    description: "内置搜索安装 Mod、资源包与光影，自动识别并补齐所需前置。",
  },
  {
    featureTitle: "全版本官方下载",
    description: "涵盖 Mojang 官方 920+ 个正式版、快照与旧版，搜索即装。",
  },
];

function isRightFaceRCell(y: number, z: number): boolean {
  if (z === 1) return true;
  if (y === 1 && z === 0) return true;
  if (y === 0 && z === -1) return true;
  if (y === 0 && z === 0) return true;
  if (y === -1 && z === -1) return true;
  return false;
}

export const VOXEL_NODES: VoxelNodeSpec[] = (() => {
  const nodes: VoxelNodeSpec[] = [];
  let idx = 0;
  for (let y = 1; y >= -1; y--) {
    for (let z = 1; z >= -1; z--) {
      for (let x = -1; x <= 1; x++) {
        const feat = USER_FEATURES[idx % USER_FEATURES.length];
        const categoryLabel =
          y === 1
            ? "启动与加载器"
            : y === 0
              ? "独立实例与模组"
              : "联机与版本下载";
        nodes.push({
          index: idx,
          coords: [x, y, z],
          featureTitle: feat.featureTitle,
          categoryLabel,
          description: feat.description,
          isRightRMark: x === 1 && isRightFaceRCell(y, z),
        });
        idx++;
      }
    }
  }
  return nodes;
})();

export interface CalloutSpec {
  voxelIndex: number;
  tag: string;
  label: string;
  detail: string;
  side: "left" | "right";
}

export interface ChapterSpec {
  id: string;
  index: number;
  range: [number, number];
  stepLabel: string;
  shortTitle: string;
  /** Strictly single-line title (<= 11 CJK chars) */
  headline: string;
  /** Strictly <= 2 lines concise user-facing description */
  lead: string;
  pills: string[];
  screenshot?: {
    url: string;
    caption: string;
  };
  callouts: CalloutSpec[];
}

export const CHAPTERS: ChapterSpec[] = [
  {
    id: "monolith",
    index: 0,
    range: [0.0, 0.2],
    stepLabel: "01 · 纯粹顺手",
    shortTitle: "打开即玩",
    headline: "点开即玩，免去繁琐配置",
    lead: "自动检测并配好所需 Java，支持微软正版皮肤头像与离线账号随时切换，让你把时间留给游戏本身。",
    pills: ["自动准备 Java", "正版与离线双模", "最近实例直达"],
    screenshot: {
      url: homeShotUrl,
      caption: "主页：一键启动当前实例，快捷切换账号与最近版本",
    },
    callouts: [
      {
        voxelIndex: 2,
        tag: "开箱即用",
        label: "自动匹配所需 Java",
        detail: "点启动直接进入游戏世界",
        side: "right",
      },
    ],
  },
  {
    id: "strata",
    index: 1,
    range: [0.2, 0.42],
    stepLabel: "02 · 独立实例",
    shortTitle: "版本隔离",
    headline: "独立实例，模组互不串档",
    lead: "点选 Vanilla、Fabric、Quilt、Forge 或 NeoForge 自动安装，每个实例拥有独立的存档、Mod 与光影文件夹。",
    pills: ["5 大主流加载器", "默认版本隔离", "共用底层省空间"],
    screenshot: {
      url: instancesShotUrl,
      caption: "新建实例：选择游戏版本与加载器，稳定版优先推荐",
    },
    callouts: [
      {
        voxelIndex: 9,
        tag: "分层隔离",
        label: "专属 Mods 与存档空间",
        detail: "多版本并存永远互不干扰",
        side: "left",
      },
    ],
  },
  {
    id: "conduit",
    index: 2,
    range: [0.42, 0.64],
    stepLabel: "03 · 精选联机",
    shortTitle: "一键进服",
    headline: "实时测速，点击直接进服",
    lead: "内置精选服务器发现与个人收藏夹，实时查看在线人数与毫秒延迟，点击「进入」直接拉起游戏连入服务器。",
    pills: ["实时人数与延迟", "点击直连进服", "个人收藏自动刷新"],
    screenshot: {
      url: serversShotUrl,
      caption: "联机发现：未进游戏即可查看服务器状态，一键直达",
    },
    callouts: [
      {
        voxelIndex: 5,
        tag: "联机直达",
        label: "看准延迟一键进入",
        detail: "启动后自动连入目标服务器",
        side: "right",
      },
    ],
  },
  {
    id: "aperture",
    index: 3,
    range: [0.64, 0.84],
    stepLabel: "04 · 版本与社区",
    shortTitle: "全量下载",
    headline: "官方全版本，内置社区下载",
    lead: "涵盖 Mojang 官方 920+ 个正式版、快照与旧版；内置 Modrinth 模组、资源包与光影搜索，自动配齐所需前置。",
    pills: ["920+ 官方版本", "Modrinth 在线搜装", "前置 Mod 自动补齐"],
    screenshot: {
      url: downloadsShotUrl,
      caption: "下载游戏：按正式版、快照、旧版清晰筛选，一键安装",
    },
    callouts: [
      {
        voxelIndex: 12,
        tag: "内容中心",
        label: "官方版本与 Modrinth",
        detail: "自动识别并补齐必需前置模组",
        side: "left",
      },
    ],
  },
  {
    id: "convergence",
    index: 4,
    range: [0.84, 1.0],
    stepLabel: "05 · 跨平台原生",
    shortTitle: "立即下载",
    headline: "三端原生，立即免费下载",
    lead: "轻快不卡顿的原生桌面应用，支持 macOS、Windows 与 Linux，内置自动更新检查与五款主题配色。",
    pills: ["macOS ARM64 / Intel", "Windows 10+", "Linux 64 位"],
    callouts: [
      {
        voxelIndex: 2,
        tag: "原生构建",
        label: "轻快省内存，内置自动更新",
        detail: "支持 macOS / Windows / Linux",
        side: "right",
      },
    ],
  },
];

export interface PlatformBuildSpec {
  id: "macos" | "windows" | "linux";
  label: string;
  subtitle: string;
  requirement: string;
  tipLabel: string;
  tipCommand: string;
}

export const PLATFORM_BUILDS: PlatformBuildSpec[] = [
  {
    id: "macos",
    label: "macOS",
    subtitle: "Apple Silicon (M 系列) / Intel",
    requirement: "macOS 15+",
    tipLabel: "首次打开若提示无法验证开发者，可在终端执行：",
    tipCommand: "xattr -dr com.apple.quarantine RMCL.app && open RMCL.app",
  },
  {
    id: "windows",
    label: "Windows",
    subtitle: "Windows x86_64 原生桌面版",
    requirement: "Windows 10+",
    tipLabel: "使用方式：",
    tipCommand: "下载 rmcl-windows-x86_64.zip 解压后直接运行 RMCL.exe",
  },
  {
    id: "linux",
    label: "Linux",
    subtitle: "Linux x86_64 原生桌面版",
    requirement: "Wayland / X11 · Vulkan",
    tipLabel: "赋予执行权限并运行：",
    tipCommand: "chmod +x ./rmcl && ./rmcl",
  },
];
