import {
  ACCENT_PALETTES,
  type AccentKey,
  CHAPTERS,
  PLATFORM_BUILDS,
  type PlatformBuildSpec,
  type ThemeMode,
  VOXEL_NODES,
  type VoxelNodeSpec,
} from "../data/rmcl-spec";
import type { ScrollStateSnapshot } from "./scroll-driver";

export interface DomControllerCallbacks {
  onSelectChapter: (index: number) => void;
  onThemeOrAccentChange: (mode: ThemeMode, accent: AccentKey) => void;
  onClearVoxelSelection: () => void;
}

interface CachedSectionRefs {
  sectionEl: HTMLElement;
  columnEl: HTMLElement;
  scrubFillEl: HTMLElement;
  pillEls: HTMLElement[];
  shotImgEl: HTMLElement | null;
  wasActive: boolean;
}

export class DomController {
  private readonly callbacks: DomControllerCallbacks;
  private readonly sectionRefs: CachedSectionRefs[] = [];
  private readonly topNavButtonEls: HTMLButtonElement[] = [];
  private readonly swatchButtonEls: Map<AccentKey, HTMLButtonElement> = new Map();
  private readonly modeBtnEls: HTMLButtonElement[] = [];

  // Consolidated Appearance Dropdown elements
  private readonly dropdownRootEl: HTMLElement;
  private readonly dropdownTriggerBtn: HTMLButtonElement;
  private readonly dropdownDotEl: HTMLElement;
  private readonly dropdownLabelEl: HTMLElement;
  private readonly dropdownPopoverEl: HTMLElement;

  // Inspector elements
  private readonly inspectorCategoryEl: HTMLElement;
  private readonly inspectorTitleEl: HTMLElement;
  private readonly inspectorDescEl: HTMLElement;
  private readonly inspectorHintEl: HTMLElement;

  // Lightbox elements
  private readonly lightboxEl: HTMLElement;
  private readonly lightboxImgEl: HTMLImageElement;
  private readonly lightboxCaptionEl: HTMLElement;

  private currentMode: ThemeMode = "light";
  private currentAccent: AccentKey = "emerald";
  private activePlatformId: PlatformBuildSpec["id"] = "macos";
  private copyResetTimer: number | null = null;
  private lastSyncedScroll = -1;
  private lastActiveChapter = -1;

  constructor(callbacks: DomControllerCallbacks) {
    this.callbacks = callbacks;

    this.dropdownRootEl = this.mustGet("#appearance-dropdown");
    this.dropdownTriggerBtn = this.mustGet<HTMLButtonElement>("#appearance-trigger");
    this.dropdownDotEl = this.mustGet("#appearance-trigger-dot");
    this.dropdownLabelEl = this.mustGet("#appearance-trigger-label");
    this.dropdownPopoverEl = this.mustGet("#appearance-popover");

    this.inspectorCategoryEl = this.mustGet("#inspector-layer");
    this.inspectorTitleEl = this.mustGet("#inspector-crate");
    this.inspectorDescEl = this.mustGet("#inspector-role");
    this.inspectorHintEl = this.mustGet("#inspector-hint");

    this.lightboxEl = this.mustGet("#shot-lightbox");
    this.lightboxImgEl = this.mustGet<HTMLImageElement>("#lightbox-img");
    this.lightboxCaptionEl = this.mustGet("#lightbox-caption");

    this.buildAppearanceDropdown();
    this.buildTopNav();
    this.buildChapters();
    this.bindPlatformControls();
    this.bindLightbox();

    this.applyThemeTokens(this.currentMode, this.currentAccent);
    this.updateInspector(VOXEL_NODES[0], false);
  }

  private mustGet<T extends HTMLElement = HTMLElement>(selector: string): T {
    const el = document.querySelector<T>(selector);
    if (!el) {
      throw new Error(`Missing required DOM node: ${selector}`);
    }
    return el;
  }

  private buildAppearanceDropdown(): void {
    const container = this.mustGet("#accent-swatches");
    const keys: AccentKey[] = ["emerald", "ocean", "violet", "amber", "rose"];

    for (const key of keys) {
      const palette = ACCENT_PALETTES[key];
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "swatch-row-btn";
      btn.dataset.accent = key;
      btn.dataset.active = key === "emerald" ? "true" : "false";

      const dot = document.createElement("span");
      dot.className = "swatch-dot";
      dot.style.backgroundColor = palette.lightPrimary;

      const label = document.createElement("span");
      label.className = "swatch-row-name";
      label.textContent = palette.name;

      const check = document.createElement("span");
      check.className = "swatch-row-check";
      check.textContent = "✓";

      btn.append(dot, label, check);
      btn.addEventListener("click", () => {
        this.currentAccent = key;
        this.applyThemeTokens(this.currentMode, this.currentAccent);
        this.callbacks.onThemeOrAccentChange(this.currentMode, this.currentAccent);
      });

      container.appendChild(btn);
      this.swatchButtonEls.set(key, btn);
    }

    // Mode switch buttons inside popover
    const modeBtns = this.dropdownPopoverEl.querySelectorAll<HTMLButtonElement>(
      ".mode-option-btn",
    );
    modeBtns.forEach((btn) => {
      this.modeBtnEls.push(btn);
      btn.addEventListener("click", () => {
        const mode = btn.dataset.mode as ThemeMode | undefined;
        if (mode && mode !== this.currentMode) {
          this.currentMode = mode;
          this.applyThemeTokens(this.currentMode, this.currentAccent);
          this.callbacks.onThemeOrAccentChange(this.currentMode, this.currentAccent);
        }
      });
    });

    // Toggle popover open/close
    this.dropdownTriggerBtn.addEventListener("click", (ev) => {
      ev.stopPropagation();
      const isOpen = !this.dropdownPopoverEl.hidden;
      this.setDropdownOpen(!isOpen);
    });

    document.addEventListener("click", (ev) => {
      if (
        !this.dropdownPopoverEl.hidden &&
        !this.dropdownRootEl.contains(ev.target as Node)
      ) {
        this.setDropdownOpen(false);
      }
    });

    window.addEventListener("keydown", (ev) => {
      if (ev.key === "Escape") {
        this.setDropdownOpen(false);
        this.lightboxEl.hidden = true;
      }
    });
  }

  private setDropdownOpen(open: boolean): void {
    this.dropdownPopoverEl.hidden = !open;
    this.dropdownTriggerBtn.setAttribute("aria-expanded", open ? "true" : "false");
  }

  public applyThemeTokens(mode: ThemeMode, key: AccentKey): void {
    const p = ACCENT_PALETTES[key];
    const root = document.documentElement;
    root.dataset.theme = mode;

    const activeHex = mode === "light" ? p.lightPrimary : p.darkPrimary;

    if (mode === "light") {
      root.style.setProperty("--bg-void", "#F6F8F7");
      root.style.setProperty("--bg-surface", "#FFFFFF");
      root.style.setProperty("--bg-panel", "rgba(255, 255, 255, 0.94)");
      root.style.setProperty("--text-primary", "#111C16");
      root.style.setProperty("--text-secondary", "#4E6157");
      root.style.setProperty("--text-muted", "#7A8D83");
      root.style.setProperty("--line-subtle", "rgba(17, 28, 22, 0.09)");
      root.style.setProperty("--line-strong", "rgba(17, 28, 22, 0.18)");
      root.style.setProperty("--accent-primary", p.lightPrimary);
      root.style.setProperty("--accent-hover", p.lightHover);
      root.style.setProperty("--accent-active", p.lightActive);
      root.style.setProperty("--accent-tint", p.lightTint);
      root.style.setProperty("--accent-border-tint", p.lightBorderTint);
      root.style.setProperty("--accent-ink", "#FFFFFF");
    } else {
      root.style.setProperty("--bg-void", "#090E0B");
      root.style.setProperty("--bg-surface", "#101914");
      root.style.setProperty("--bg-panel", "rgba(12, 19, 15, 0.94)");
      root.style.setProperty("--text-primary", "#EEF7F2");
      root.style.setProperty("--text-secondary", "#96AEA2");
      root.style.setProperty("--text-muted", "#5C7468");
      root.style.setProperty("--line-subtle", "rgba(238, 247, 242, 0.09)");
      root.style.setProperty("--line-strong", "rgba(238, 247, 242, 0.2)");
      root.style.setProperty("--accent-primary", p.darkPrimary);
      root.style.setProperty("--accent-hover", p.darkHover);
      root.style.setProperty("--accent-active", p.darkActive);
      root.style.setProperty("--accent-tint", p.darkTint);
      root.style.setProperty("--accent-border-tint", p.darkActive);
      root.style.setProperty("--accent-ink", "#07130C");
    }

    // Update consolidated dropdown trigger
    this.dropdownDotEl.style.backgroundColor = activeHex;
    this.dropdownLabelEl.textContent = `${p.name} · ${mode === "light" ? "浅色" : "深色"}`;

    for (const btn of this.modeBtnEls) {
      btn.dataset.active = btn.dataset.mode === mode ? "true" : "false";
    }

    for (const [k, btn] of this.swatchButtonEls.entries()) {
      btn.dataset.active = k === key ? "true" : "false";
      const dot = btn.querySelector<HTMLElement>(".swatch-dot");
      if (dot) {
        dot.style.backgroundColor =
          mode === "light" ? ACCENT_PALETTES[k].lightPrimary : ACCENT_PALETTES[k].darkPrimary;
      }
    }

    this.updateScreenshotThemes(mode);
  }

  private updateScreenshotThemes(mode: ThemeMode): void {
    const images = document.querySelectorAll<HTMLImageElement>(".shot-img, #lightbox-img");
    for (const image of images) {
      const url = mode === "dark" ? image.dataset.shotDark : image.dataset.shotLight;
      if (url && image.getAttribute("src") !== url) {
        image.src = url;
      }
    }
  }

  private buildTopNav(): void {
    const nav = this.mustGet("#top-stage-nav");
    for (const ch of CHAPTERS) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "top-stage-btn";
      btn.dataset.index = String(ch.index);
      btn.dataset.active = ch.index === 0 ? "true" : "false";
      btn.textContent = ch.shortTitle;
      btn.addEventListener("click", () => {
        this.callbacks.onSelectChapter(ch.index);
      });
      nav.appendChild(btn);
      this.topNavButtonEls.push(btn);
    }
  }

  private buildChapters(): void {
    const track = this.mustGet("#scroll-track");

    for (const ch of CHAPTERS) {
      const section = document.createElement("section");
      section.className = "chapter-section";
      section.id = `chapter-${ch.id}`;
      section.dataset.index = String(ch.index);
      section.dataset.align = ch.index % 2 === 0 ? "left" : "right";

      const isFirst = ch.index === 0;
      const isLast = ch.index === CHAPTERS.length - 1;

      const pillsHtml = ch.pills
        .map(
          (pill, idx) =>
            `<span class="feature-pill" data-pill-idx="${idx}">${pill}</span>`,
        )
        .join("");

      const screenshotHtml = ch.screenshot
        ? `
          <figure class="shot-showcase" data-shot-url-light="${ch.screenshot.lightUrl}" data-shot-url-dark="${ch.screenshot.darkUrl}" data-shot-caption="${ch.screenshot.caption}">
            <div class="shot-img-wrap">
              <img
                src="${ch.screenshot.lightUrl}"
                data-shot-light="${ch.screenshot.lightUrl}"
                data-shot-dark="${ch.screenshot.darkUrl}"
                alt="${ch.screenshot.caption}"
                class="shot-img"
                width="934"
                height="618"
                loading="${isFirst ? "eager" : "lazy"}"
                decoding="async"
              />
            </div>
            <figcaption class="shot-caption">
              <span>${ch.screenshot.caption}</span>
              <span class="shot-zoom-hint">点击放大</span>
            </figcaption>
          </figure>
        `
        : "";

      let actionsHtml = "";
      if (isFirst) {
        actionsHtml = `
          <div class="hero-actions">
            <a class="primary-action" href="https://github.com/driftcell/RMCL/releases" target="_blank" rel="noreferrer">
              免费下载 RMCL
            </a>
            <button type="button" class="secondary-action" data-scroll-next="1">
              向下滚动探索
            </button>
          </div>
        `;
      } else if (isLast) {
        const platformTabs = PLATFORM_BUILDS.map(
          (pb) => `
            <button type="button" class="platform-tab-btn" data-platform="${pb.id}" data-active="${pb.id === "macos" ? "true" : "false"}">
              ${pb.label}
            </button>
          `,
        ).join("");

        const initialPlatform = PLATFORM_BUILDS[0];
        actionsHtml = `
          <div class="convergence-controls">
            <div class="platform-matrix">
              <div class="platform-tabs" role="tablist">${platformTabs}</div>
              <div class="platform-detail">
                <div class="platform-meta-row">
                  <span id="plat-arch" class="platform-arch">${initialPlatform.subtitle}</span>
                  <span id="plat-req" class="platform-req">${initialPlatform.requirement}</span>
                </div>
                <div class="command-box">
                  <div class="command-header">
                    <span id="plat-cmd-label">${initialPlatform.tipLabel}</span>
                    <button type="button" id="copy-cmd-btn" class="copy-cmd-btn">复制</button>
                  </div>
                  <pre class="command-pre"><code id="plat-cmd-code">${initialPlatform.tipCommand}</code></pre>
                </div>
              </div>
            </div>

            <div class="hero-actions">
              <a class="primary-action" href="https://github.com/driftcell/RMCL/releases" target="_blank" rel="noreferrer">
                前往 GitHub 下载安装包
              </a>
              <a class="secondary-action" href="https://github.com/driftcell/RMCL" target="_blank" rel="noreferrer">
                查看开源项目主页
              </a>
            </div>
          </div>
        `;
      }

      section.innerHTML = `
        <div class="chapter-column">
          <div class="chapter-progress-head">
            <span class="chapter-state-id">${ch.stepLabel}</span>
            <div class="chapter-scrub-bar">
              <div class="chapter-scrub-fill"></div>
            </div>
          </div>
          <h2 class="chapter-headline">${ch.headline}</h2>
          <p class="chapter-lead">${ch.lead}</p>
          <div class="pills-row">
            ${pillsHtml}
          </div>
          ${screenshotHtml}
          ${actionsHtml}
        </div>
      `;

      track.appendChild(section);

      this.sectionRefs.push({
        sectionEl: section,
        columnEl: section.querySelector<HTMLElement>(".chapter-column")!,
        scrubFillEl: section.querySelector<HTMLElement>(".chapter-scrub-fill")!,
        pillEls: Array.from(section.querySelectorAll<HTMLElement>(".feature-pill")),
        shotImgEl: section.querySelector<HTMLElement>(".shot-img"),
        wasActive: false,
      });
    }

    const nextBtn = track.querySelector<HTMLButtonElement>("[data-scroll-next]");
    nextBtn?.addEventListener("click", () => {
      this.callbacks.onSelectChapter(1);
    });
  }

  private bindLightbox(): void {
    const figures = document.querySelectorAll<HTMLElement>(".shot-showcase");
    figures.forEach((fig) => {
      fig.addEventListener("click", () => {
        const lightUrl = fig.dataset.shotUrlLight;
        const darkUrl = fig.dataset.shotUrlDark;
        const caption = fig.dataset.shotCaption;
        if (!lightUrl || !darkUrl) return;
        this.lightboxImgEl.dataset.shotLight = lightUrl;
        this.lightboxImgEl.dataset.shotDark = darkUrl;
        this.updateScreenshotThemes(this.currentMode);
        this.lightboxImgEl.alt = caption || "";
        this.lightboxCaptionEl.textContent = caption || "";
        this.lightboxEl.hidden = false;
      });
    });

    const closeBtn = document.getElementById("lightbox-close");
    closeBtn?.addEventListener("click", () => {
      this.lightboxEl.hidden = true;
    });

    this.lightboxEl.addEventListener("click", (ev) => {
      if (ev.target === this.lightboxEl) {
        this.lightboxEl.hidden = true;
      }
    });
  }

  private bindPlatformControls(): void {
    const tabs = document.querySelectorAll<HTMLButtonElement>(".platform-tab-btn");
    const archEl = document.getElementById("plat-arch");
    const reqEl = document.getElementById("plat-req");
    const cmdLabelEl = document.getElementById("plat-cmd-label");
    const cmdCodeEl = document.getElementById("plat-cmd-code");
    const copyBtn = document.getElementById("copy-cmd-btn");

    tabs.forEach((tab) => {
      tab.addEventListener("click", () => {
        const id = tab.dataset.platform as PlatformBuildSpec["id"] | undefined;
        const spec = PLATFORM_BUILDS.find((p) => p.id === id);
        if (!spec) return;
        this.activePlatformId = spec.id;
        tabs.forEach((t) => {
          t.dataset.active = t.dataset.platform === spec.id ? "true" : "false";
        });
        if (archEl) archEl.textContent = spec.subtitle;
        if (reqEl) reqEl.textContent = spec.requirement;
        if (cmdLabelEl) cmdLabelEl.textContent = spec.tipLabel;
        if (cmdCodeEl) cmdCodeEl.textContent = spec.tipCommand;
      });
    });

    copyBtn?.addEventListener("click", async () => {
      const spec = PLATFORM_BUILDS.find((p) => p.id === this.activePlatformId);
      if (!spec) return;
      try {
        await navigator.clipboard.writeText(spec.tipCommand);
        copyBtn.textContent = "已复制";
        if (this.copyResetTimer !== null) {
          window.clearTimeout(this.copyResetTimer);
        }
        this.copyResetTimer = window.setTimeout(() => {
          copyBtn.textContent = "复制";
        }, 1800);
      } catch {
        copyBtn.textContent = "请手动复制";
      }
    });

    const inspectorBox = document.getElementById("voxel-inspector");
    inspectorBox?.addEventListener("click", () => {
      this.callbacks.onClearVoxelSelection();
    });
  }

  public updateInspector(spec: VoxelNodeSpec | null, isLocked: boolean): void {
    const node = spec ?? VOXEL_NODES[0];
    this.inspectorCategoryEl.textContent = node.categoryLabel;
    this.inspectorTitleEl.textContent = node.featureTitle;
    this.inspectorDescEl.textContent = node.description;
    this.inspectorHintEl.textContent = isLocked
      ? "已固定此功能卡片 · 点击解除"
      : "悬停 3D 方块查看亮点";
  }

  public syncWithScroll(snapshot: ScrollStateSnapshot, reducedMotion: boolean): void {
    const { smoothProgress, velocity, activeChapterIndex } = snapshot;

    if (
      Math.abs(smoothProgress - this.lastSyncedScroll) < 0.0003 &&
      Math.abs(velocity) < 0.001 &&
      activeChapterIndex === this.lastActiveChapter
    ) {
      return;
    }
    this.lastSyncedScroll = smoothProgress;

    if (activeChapterIndex !== this.lastActiveChapter) {
      this.lastActiveChapter = activeChapterIndex;

      for (let i = 0; i < this.topNavButtonEls.length; i++) {
        const btn = this.topNavButtonEls[i];
        btn.dataset.active = i === activeChapterIndex ? "true" : "false";
      }
    }

    for (let i = 0; i < this.sectionRefs.length; i++) {
      const ref = this.sectionRefs[i];
      const ch = CHAPTERS[i];
      const [start, end] = ch.range;
      const span = Math.max(0.001, end - start);
      const localP = Math.max(0, Math.min(1, (smoothProgress - start) / span));
      const isCurrent = i === activeChapterIndex;

      if (ref.wasActive !== isCurrent) {
        ref.wasActive = isCurrent;
        ref.sectionEl.dataset.active = isCurrent ? "true" : "false";
      }

      ref.scrubFillEl.style.transform = `scaleX(${localP.toFixed(3)})`;

      for (let pIdx = 0; pIdx < ref.pillEls.length; pIdx++) {
        const lit = localP >= pIdx * 0.22 ? "true" : "false";
        if (ref.pillEls[pIdx].dataset.lit !== lit) {
          ref.pillEls[pIdx].dataset.lit = lit;
        }
      }

      if (reducedMotion) {
        ref.columnEl.style.transform = "none";
      } else {
        const driftY = (0.5 - localP) * 18;
        const velShift = Math.max(-6, Math.min(6, velocity * -8));
        ref.columnEl.style.transform = `translate3d(0, ${(driftY + velShift).toFixed(1)}px, 0)`;
      }
    }
  }
}
