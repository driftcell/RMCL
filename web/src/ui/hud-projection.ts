import * as THREE from "three";
import { CHAPTERS } from "../data/rmcl-spec";
import { VoxelCoreSystem } from "../render/voxel-core";

interface CalloutDomEntry {
  groupEl: HTMLDivElement;
  tagEl: HTMLSpanElement;
  labelEl: HTMLSpanElement;
  detailEl: HTMLSpanElement;
  lineEl: SVGPolylineElement;
  dotEl: SVGCircleElement;
  lastX: number;
  lastY: number;
  lastOpacity: string;
  lastTag: string;
}

export class HudProjectionOverlay {
  private readonly svgEl: SVGSVGElement;
  private readonly labelsContainer: HTMLElement;
  private readonly pool: CalloutDomEntry[] = [];
  private readonly tempVec = new THREE.Vector3();
  private lastWidth = 0;
  private lastHeight = 0;

  constructor(svgEl: SVGSVGElement, labelsContainer: HTMLElement) {
    this.svgEl = svgEl;
    this.labelsContainer = labelsContainer;

    for (let i = 0; i < 2; i++) {
      const lineEl = document.createElementNS("http://www.w3.org/2000/svg", "polyline");
      lineEl.setAttribute("class", "hud-leader-line");
      lineEl.style.opacity = "0";
      this.svgEl.appendChild(lineEl);

      const dotEl = document.createElementNS("http://www.w3.org/2000/svg", "circle");
      dotEl.setAttribute("class", "hud-leader-dot");
      dotEl.setAttribute("r", "3.5");
      dotEl.style.opacity = "0";
      this.svgEl.appendChild(dotEl);

      const groupEl = document.createElement("div");
      groupEl.className = "hud-callout-item";
      groupEl.style.opacity = "0";

      const tagEl = document.createElement("span");
      tagEl.className = "hud-callout-code";
      const labelEl = document.createElement("span");
      labelEl.className = "hud-callout-label";
      const detailEl = document.createElement("span");
      detailEl.className = "hud-callout-detail";

      groupEl.append(tagEl, labelEl, detailEl);
      this.labelsContainer.appendChild(groupEl);

      this.pool.push({
        groupEl,
        tagEl,
        labelEl,
        detailEl,
        lineEl,
        dotEl,
        lastX: -9999,
        lastY: -9999,
        lastOpacity: "0",
        lastTag: "",
      });
    }
  }

  private hideEntry(entry: CalloutDomEntry): void {
    if (entry.lastOpacity !== "0") {
      entry.groupEl.style.opacity = "0";
      entry.lineEl.style.opacity = "0";
      entry.dotEl.style.opacity = "0";
      entry.lastOpacity = "0";
    }
  }

  public update(
    activeChapterIndex: number,
    chapterLocalProgress: number,
    camera: THREE.Camera,
    voxelCore: VoxelCoreSystem,
    width: number,
    height: number,
    isMobile: boolean,
  ): void {
    if (width !== this.lastWidth || height !== this.lastHeight) {
      this.svgEl.setAttribute("viewBox", `0 0 ${width} ${height}`);
      this.lastWidth = width;
      this.lastHeight = height;
    }

    if (isMobile || width < 1080) {
      for (const item of this.pool) {
        this.hideEntry(item);
      }
      return;
    }

    const chapter = CHAPTERS[activeChapterIndex];
    const callouts = chapter?.callouts ?? [];

    // Chapter 0 is visible right at scroll = 0; last chapter stays visible at scroll = 1
    const fadeIn =
      activeChapterIndex === 0 ? 1 : Math.min(1, chapterLocalProgress * 5.0);
    const fadeOut =
      activeChapterIndex === CHAPTERS.length - 1
        ? 1
        : Math.min(1, (1 - chapterLocalProgress) * 5.0);
    const edgeFade = fadeIn * fadeOut;

    for (let i = 0; i < this.pool.length; i++) {
      const entry = this.pool[i];
      const spec = callouts[i];
      if (!spec || edgeFade <= 0.03) {
        this.hideEntry(entry);
        continue;
      }

      const contentChanged = entry.lastTag !== spec.tag;
      if (contentChanged) {
        entry.lastTag = spec.tag;
        entry.tagEl.textContent = spec.tag;
        entry.labelEl.textContent = spec.label;
        entry.detailEl.textContent = spec.detail;
        entry.groupEl.dataset.side = spec.side;
      }

      voxelCore.getVoxelWorldPosition(spec.voxelIndex, this.tempVec);
      this.tempVec.project(camera);

      if (this.tempVec.z > 1) {
        this.hideEntry(entry);
        continue;
      }

      const sx = (this.tempVec.x * 0.5 + 0.5) * width;
      const sy = (-this.tempVec.y * 0.5 + 0.5) * height;
      const opacityStr = edgeFade.toFixed(2);

      if (
        Math.abs(sx - entry.lastX) > 0.35 ||
        Math.abs(sy - entry.lastY) > 0.35 ||
        contentChanged
      ) {
        entry.lastX = sx;
        entry.lastY = sy;

        const dir = spec.side === "right" ? 1 : -1;
        const elbowX = sx + dir * 36;
        const elbowY = Math.max(108, Math.min(height - 140, sy - 38));

        const endX =
          spec.side === "right"
            ? Math.max(width * 0.58, Math.min(width - 236, elbowX + 48))
            : Math.max(248, Math.min(width * 0.42, elbowX - 48));

        entry.dotEl.setAttribute("cx", sx.toFixed(1));
        entry.dotEl.setAttribute("cy", sy.toFixed(1));
        entry.lineEl.setAttribute(
          "points",
          `${sx.toFixed(1)},${sy.toFixed(1)} ${elbowX.toFixed(1)},${elbowY.toFixed(1)} ${endX.toFixed(1)},${elbowY.toFixed(1)}`,
        );

        entry.groupEl.style.transform =
          spec.side === "right"
            ? `translate3d(${(endX + 8).toFixed(1)}px, ${(elbowY - 14).toFixed(1)}px, 0)`
            : `translate3d(${(endX - 220).toFixed(1)}px, ${(elbowY - 14).toFixed(1)}px, 0)`;
      }

      if (opacityStr !== entry.lastOpacity) {
        entry.lastOpacity = opacityStr;
        entry.dotEl.style.opacity = opacityStr;
        entry.lineEl.style.opacity = (edgeFade * 0.72).toFixed(2);
        entry.groupEl.style.opacity = opacityStr;
      }
    }
  }
}
