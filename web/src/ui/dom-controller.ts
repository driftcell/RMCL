import {
  ACCENT_PALETTES,
  type AccentKey,
  CHAPTERS,
  PLATFORM_BUILDS,
  type PlatformBuildSpec,
  VOXEL_NODES,
  type VoxelNodeSpec,
} from "../data/rmcl-spec";
import type { ScrollStateSnapshot } from "./scroll-driver";

export interface DomControllerCallbacks {
  onSelectChapter: (index: number) => void;
  onSelectAccent: (accent: AccentKey) => void;
  onClearVoxelSelection: () => void;
}

export class DomController {
  private readonly callbacks: DomControllerCallbacks;
  private readonly sectionEls: HTMLElement[] = [];
  private readonly railButtonEls: HTMLButtonElement[] = [];
  private readonly swatchButtonEls: Map<AccentKey, HTMLButtonElement> = new Map();

  private readonly stateCodeEl: HTMLElement;
  private readonly scrollCoordEl: HTMLElement;
  private readonly railCursorEl: HTMLElement;
  private readonly railFillEl: HTMLElement;

  // Voxel Inspector DOM elements
  private readonly inspectorCoordEl: HTMLElement;
  private readonly inspectorCrateEl: HTMLElement;
  private readonly inspectorLayerEl: HTMLElement;
  private readonly inspectorRoleEl: HTMLElement;
  private readonly inspectorPathEl: HTMLElement;
  private readonly inspectorHintEl: HTMLElement;

  // Platform switcher DOM elements
  private activePlatformId: PlatformBuildSpec["id"] = "macos";
  private copyResetTimer: number | null = null;

  constructor(callbacks: DomControllerCallbacks) {
    this.callbacks = callbacks;

    this.stateCodeEl = this.mustGet("#bar-state-code");
    this.scrollCoordEl = this.mustGet("#bar-scroll-coord");
    this.railCursorEl = this.mustGet("#rail-cursor");
    this.railFillEl = this.mustGet("#rail-fill");

    this.inspectorCoordEl = this.mustGet("#inspector-coord");
    this.inspectorCrateEl = this.mustGet("#inspector-crate");
    this.inspectorLayerEl = this.mustGet("#inspector-layer");
    this.inspectorRoleEl = this.mustGet("#inspector-role");
    this.inspectorPathEl = this.mustGet("#inspector-path");
    this.inspectorHintEl = this.mustGet("#inspector-hint");

    this.buildTopSwatches();
    this.buildDepthRail();
    this.buildChapters();
    this.bindPlatformControls();
    this.updateInspector(VOXEL_NODES[2], false);
  }

  private mustGet<T extends HTMLElement = HTMLElement>(selector: string): T {
    const el = document.querySelector<T>(selector);
    if (!el) {
      throw new Error(`Missing required DOM node: ${selector}`);
    }
    return el;
  }

  private buildTopSwatches(): void {
    const container = this.mustGet("#accent-swatches");
    const keys: AccentKey[] = ["emerald", "ocean", "violet", "amber", "rose"];

    for (const key of keys) {
      const palette = ACCENT_PALETTES[key];
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "swatch-btn";
      btn.dataset.accent = key;
      btn.dataset.active = key === "emerald" ? "true" : "false";
      btn.title = `${palette.name} (${palette.enName})`;
      btn.setAttribute("aria-label", `切换强调色：${palette.name}`);

      const dot = document.createElement("span");
      dot.className = "swatch-dot";
      dot.style.backgroundColor = palette.primary;

      const label = document.createElement("span");
      label.className = "swatch-label";
      label.textContent = palette.name;

      btn.append(dot, label);
      btn.addEventListener("click", () => {
        this.applyAccentTheme(key);
        this.callbacks.onSelectAccent(key);
      });

      container.appendChild(btn);
      this.swatchButtonEls.set(key, btn);
    }
  }

  public applyAccentTheme(key: AccentKey): void {
    const palette = ACCENT_PALETTES[key];
    const root = document.documentElement;
    root.style.setProperty("--accent-primary", palette.primary);
    root.style.setProperty("--accent-hover", palette.hover);
    root.style.setProperty("--accent-active", palette.active);
    root.style.setProperty("--accent-specular", palette.specular);
    root.style.setProperty("--accent-surface", palette.surfaceTint);
    root.style.setProperty("--accent-deep", palette.deepSurface);
    root.style.setProperty("--accent-ink", palette.ink);

    for (const [k, btn] of this.swatchButtonEls.entries()) {
      btn.dataset.active = k === key ? "true" : "false";
    }

    // Also sync any accent buttons inside Chapter 4
    const inlineBtns = document.querySelectorAll<HTMLButtonElement>(".theme-inline-btn");
    inlineBtns.forEach((b) => {
      b.dataset.active = b.dataset.accent === key ? "true" : "false";
    });
  }

  private buildDepthRail(): void {
    const nav = this.mustGet("#rail-stages");
    for (const ch of CHAPTERS) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "rail-stage-btn";
      btn.dataset.index = String(ch.index);
      btn.innerHTML = `
        <span class="rail-stage-num">0${ch.index}</span>
        <span class="rail-stage-name">${ch.shortTitle}</span>
      `;
      btn.addEventListener("click", () => {
        this.callbacks.onSelectChapter(ch.index);
      });
      nav.appendChild(btn);
      this.railButtonEls.push(btn);
    }
  }

  private buildChapters(): void {
    const track = this.mustGet("#scroll-track");

    for (const ch of CHAPTERS) {
      const section = document.createElement("section");
      section.className = "chapter-section";
      section.id = `chapter-${ch.id}`;
      section.dataset.index = String(ch.index);
      // Alternate left/right alignment so the 3D core and typography counter-balance each other
      section.dataset.align = ch.index % 2 === 0 ? "left" : "right";

      const isFirst = ch.index === 0;
      const isLast = ch.index === CHAPTERS.length - 1;

      const metricsHtml = ch.metrics
        .map(
          (m, idx) => `
          <div class="metric-cell" data-metric-idx="${idx}">
            <div class="metric-head">
              <span class="metric-value">${m.value}</span>
              <span class="metric-unit">${m.unit}</span>
            </div>
            <p class="metric-caption">${m.caption}</p>
          </div>
        `,
        )
        .join("");

      const terminalLinesHtml = ch.terminalSpec.lines
        .map(
          (line, lIdx) => `
          <div class="trace-line" data-line-idx="${lIdx}" data-highlight="${line.highlight ? "true" : "false"}">
            <span class="trace-key">${line.key}</span>
            <span class="trace-val">${line.value}</span>
          </div>
        `,
        )
        .join("");

      let extraInteractiveHtml = "";
      if (isFirst) {
        extraInteractiveHtml = `
          <div class="hero-actions">
            <a class="primary-action" href="https://github.com/driftcell/RMCL/releases" target="_blank" rel="noreferrer">
              下载 Release 构建
            </a>
            <button type="button" class="secondary-action" data-scroll-next="1">
              向下滚动拆解核心
            </button>
          </div>
        `;
      } else if (isLast) {
        const accentPills = (["emerald", "ocean", "violet", "amber", "rose"] as AccentKey[])
          .map((k) => {
            const p = ACCENT_PALETTES[k];
            return `
              <button type="button" class="theme-inline-btn" data-accent="${k}" data-active="${k === "emerald" ? "true" : "false"}">
                <span class="swatch-dot" style="background:${p.primary}"></span>
                <span>${p.name}</span>
              </button>
            `;
          })
          .join("");

        const platformTabs = PLATFORM_BUILDS.map(
          (pb) => `
            <button type="button" class="platform-tab-btn" data-platform="${pb.id}" data-active="${pb.id === "macos" ? "true" : "false"}">
              ${pb.label}
            </button>
          `,
        ).join("");

        const initialPlatform = PLATFORM_BUILDS[0];
        extraInteractiveHtml = `
          <div class="convergence-controls">
            <div class="theme-strip">
              <span class="strip-title">外观强调色 (同步 apps/rmcl/src/theme.rs)</span>
              <div class="theme-inline-list">${accentPills}</div>
            </div>

            <div class="platform-matrix">
              <div class="platform-tabs" role="tablist">${platformTabs}</div>
              <div class="platform-detail">
                <div class="platform-meta-row">
                  <span id="plat-arch" class="platform-arch">${initialPlatform.arch}</span>
                  <span id="plat-req" class="platform-req">${initialPlatform.requirement}</span>
                </div>
                <p id="plat-note" class="platform-note">${initialPlatform.note}</p>
                <div class="command-box">
                  <div class="command-header">
                    <span id="plat-cmd-label">${initialPlatform.commandLabel}</span>
                    <button type="button" id="copy-cmd-btn" class="copy-cmd-btn">复制命令</button>
                  </div>
                  <pre class="command-pre"><code id="plat-cmd-code">${initialPlatform.command}</code></pre>
                </div>
              </div>
            </div>

            <div class="hero-actions">
              <a class="primary-action" href="https://github.com/driftcell/RMCL/releases" target="_blank" rel="noreferrer">
                前往 GitHub Releases
              </a>
              <a class="secondary-action" href="https://github.com/driftcell/RMCL" target="_blank" rel="noreferrer">
                浏览 Rust 源码与架构文档
              </a>
            </div>
          </div>
        `;
      }

      section.innerHTML = `
        <div class="chapter-column">
          <div class="chapter-progress-head">
            <span class="chapter-state-id">${ch.stateCode}</span>
            <div class="chapter-scrub-bar">
              <div class="chapter-scrub-fill"></div>
            </div>
          </div>
          <h1 class="chapter-headline">${ch.headline}</h1>
          <p class="chapter-lead">${ch.lead}</p>
          <div class="metrics-strip">
            ${metricsHtml}
          </div>
          <div class="trace-block">
            <div class="trace-header">
              <span class="trace-path">${ch.terminalSpec.path}</span>
              <span class="trace-phase-pct">0%</span>
            </div>
            <div class="trace-body">
              ${terminalLinesHtml}
            </div>
          </div>
          ${extraInteractiveHtml}
        </div>
      `;

      track.appendChild(section);
      this.sectionEls.push(section);
    }

    const nextBtn = track.querySelector<HTMLButtonElement>("[data-scroll-next]");
    nextBtn?.addEventListener("click", () => {
      this.callbacks.onSelectChapter(1);
    });

    const inlineThemeBtns = track.querySelectorAll<HTMLButtonElement>(".theme-inline-btn");
    inlineThemeBtns.forEach((btn) => {
      btn.addEventListener("click", () => {
        const accent = btn.dataset.accent as AccentKey | undefined;
        if (accent) {
          this.applyAccentTheme(accent);
          this.callbacks.onSelectAccent(accent);
        }
      });
    });
  }

  private bindPlatformControls(): void {
    const tabs = document.querySelectorAll<HTMLButtonElement>(".platform-tab-btn");
    const archEl = document.getElementById("plat-arch");
    const reqEl = document.getElementById("plat-req");
    const noteEl = document.getElementById("plat-note");
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
        if (archEl) archEl.textContent = spec.arch;
        if (reqEl) reqEl.textContent = spec.requirement;
        if (noteEl) noteEl.textContent = spec.note;
        if (cmdLabelEl) cmdLabelEl.textContent = spec.commandLabel;
        if (cmdCodeEl) cmdCodeEl.textContent = spec.command;
      });
    });

    copyBtn?.addEventListener("click", async () => {
      const spec = PLATFORM_BUILDS.find((p) => p.id === this.activePlatformId);
      if (!spec) return;
      try {
        await navigator.clipboard.writeText(spec.command);
        copyBtn.textContent = "已复制命令";
        copyBtn.dataset.copied = "true";
        if (this.copyResetTimer !== null) {
          window.clearTimeout(this.copyResetTimer);
        }
        this.copyResetTimer = window.setTimeout(() => {
          copyBtn.textContent = "复制命令";
          copyBtn.dataset.copied = "false";
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
    const node = spec ?? VOXEL_NODES[2];
    const [x, y, z] = node.coords;
    const fmt = (n: number) => (n >= 0 ? `+${n}` : `${n}`);
    this.inspectorCoordEl.textContent = `[${fmt(x)}, ${fmt(y)}, ${fmt(z)}]`;
    this.inspectorCrateEl.textContent = node.crateName;
    this.inspectorLayerEl.textContent = node.layerLabel;
    this.inspectorRoleEl.textContent = node.role;
    this.inspectorPathEl.textContent = node.artifactPath;
    this.inspectorHintEl.textContent = isLocked
      ? "已锁定单元 · 点击此处或空白区解除"
      : "悬停或点击 3D 核心单元检视对应 Crate";
  }

  /**
   * Synchronizes all scroll-driven DOM elements with the current scroll progress & velocity.
   */
  public syncWithScroll(snapshot: ScrollStateSnapshot, reducedMotion: boolean): void {
    const { smoothProgress, velocity, activeChapterIndex, chapterLocalProgress } = snapshot;

    const activeChapter = CHAPTERS[activeChapterIndex];
    this.stateCodeEl.textContent = activeChapter.stateCode;
    this.scrollCoordEl.textContent = smoothProgress.toFixed(3);

    // Update left-edge depth rail cursor & active stage button
    const railPct = (smoothProgress * 100).toFixed(2);
    this.railCursorEl.style.transform = `translate3d(0, ${railPct}%, 0)`;
    this.railFillEl.style.transform = `scaleY(${smoothProgress.toFixed(4)})`;

    for (let i = 0; i < this.railButtonEls.length; i++) {
      const btn = this.railButtonEls[i];
      btn.dataset.active = i === activeChapterIndex ? "true" : "false";
      btn.dataset.passed = i < activeChapterIndex ? "true" : "false";
    }

    // Drive per-section kinetic transforms, scrub bars, metric activation, and trace cursors
    for (let i = 0; i < this.sectionEls.length; i++) {
      const section = this.sectionEls[i];
      const ch = CHAPTERS[i];
      const [start, end] = ch.range;
      const span = Math.max(0.001, end - start);
      const localP = Math.max(0, Math.min(1, (smoothProgress - start) / span));
      const isCurrent = i === activeChapterIndex;

      section.dataset.active = isCurrent ? "true" : "false";

      const fillEl = section.querySelector<HTMLElement>(".chapter-scrub-fill");
      if (fillEl) {
        fillEl.style.transform = `scaleX(${localP.toFixed(3)})`;
      }

      const pctEl = section.querySelector<HTMLElement>(".trace-phase-pct");
      if (pctEl) {
        pctEl.textContent = `${Math.round(localP * 100)}%`;
      }

      // Activate metrics sequentially as the reader scrolls through the chapter
      const metrics = section.querySelectorAll<HTMLElement>(".metric-cell");
      metrics.forEach((mEl, mIdx) => {
        const threshold = mIdx * 0.25;
        mEl.dataset.lit = localP >= threshold ? "true" : "false";
      });

      // Scrub the active line inside the execution trace terminal
      const traceLines = section.querySelectorAll<HTMLElement>(".trace-line");
      const activeLineIdx = Math.min(
        traceLines.length - 1,
        Math.floor(localP * traceLines.length),
      );
      traceLines.forEach((lEl, lIdx) => {
        lEl.dataset.activeLine = isCurrent && lIdx === activeLineIdx ? "true" : "false";
      });

      // Subtle scroll-linked kinetic parallax on the chapter column
      const colEl = section.querySelector<HTMLElement>(".chapter-column");
      if (colEl) {
        if (reducedMotion) {
          colEl.style.transform = "none";
        } else {
          const driftY = (0.5 - localP) * 28;
          const velShift = Math.max(-10, Math.min(10, velocity * -14));
          colEl.style.transform = `translate3d(0, ${(driftY + velShift).toFixed(1)}px, 0)`;
        }
      }
    }

    // Sync chapterLocalProgress CSS variable for any subtle stage indicators
    document.documentElement.style.setProperty(
      "--chapter-progress",
      chapterLocalProgress.toFixed(3),
    );
  }
}
