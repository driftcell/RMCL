import * as THREE from "three";
import { CHAPTERS } from "../data/rmcl-spec";
import { VoxelCoreSystem } from "../render/voxel-core";

interface CalloutDomEntry {
  groupEl: HTMLDivElement;
  codeEl: HTMLSpanElement;
  labelEl: HTMLSpanElement;
  detailEl: HTMLSpanElement;
  lineEl: SVGPolylineElement;
  dotEl: SVGCircleElement;
}

/**
 * Projects 3D world-space voxel coordinates onto a 2D SVG/DOM spatial overlay
 * so technical callout leader lines track the moving 3D subvoxels in real time.
 */
export class HudProjectionOverlay {
  private readonly svgEl: SVGSVGElement;
  private readonly labelsContainer: HTMLElement;
  private readonly pool: CalloutDomEntry[] = [];
  private readonly tempVec = new THREE.Vector3();

  constructor(svgEl: SVGSVGElement, labelsContainer: HTMLElement) {
    this.svgEl = svgEl;
    this.labelsContainer = labelsContainer;

    for (let i = 0; i < 3; i++) {
      const lineEl = document.createElementNS("http://www.w3.org/2000/svg", "polyline");
      lineEl.setAttribute("class", "hud-leader-line");
      this.svgEl.appendChild(lineEl);

      const dotEl = document.createElementNS("http://www.w3.org/2000/svg", "circle");
      dotEl.setAttribute("class", "hud-leader-dot");
      dotEl.setAttribute("r", "3.5");
      this.svgEl.appendChild(dotEl);

      const groupEl = document.createElement("div");
      groupEl.className = "hud-callout-item";
      const codeEl = document.createElement("span");
      codeEl.className = "hud-callout-code";
      const labelEl = document.createElement("span");
      labelEl.className = "hud-callout-label";
      const detailEl = document.createElement("span");
      detailEl.className = "hud-callout-detail";

      groupEl.append(codeEl, labelEl, detailEl);
      this.labelsContainer.appendChild(groupEl);

      this.pool.push({
        groupEl,
        codeEl,
        labelEl,
        detailEl,
        lineEl,
        dotEl,
      });
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
    this.svgEl.setAttribute("viewBox", `0 0 ${width} ${height}`);

    // On narrow mobile screens, keep the 3D viewport clean of floating leader lines
    if (isMobile) {
      for (const item of this.pool) {
        item.groupEl.style.opacity = "0";
        item.lineEl.style.opacity = "0";
        item.dotEl.style.opacity = "0";
      }
      return;
    }

    const chapter = CHAPTERS[activeChapterIndex];
    const callouts = chapter?.callouts ?? [];

    // Fade callouts smoothly at chapter boundaries
    const edgeFade =
      Math.min(1, chapterLocalProgress * 5.0) *
      Math.min(1, (1 - chapterLocalProgress) * 5.0);

    for (let i = 0; i < this.pool.length; i++) {
      const entry = this.pool[i];
      const spec = callouts[i];
      if (!spec || edgeFade <= 0.02) {
        entry.groupEl.style.opacity = "0";
        entry.lineEl.style.opacity = "0";
        entry.dotEl.style.opacity = "0";
        continue;
      }

      voxelCore.getVoxelWorldPosition(spec.voxelIndex, this.tempVec);
      this.tempVec.project(camera);

      // Behind camera check
      if (this.tempVec.z > 1) {
        entry.groupEl.style.opacity = "0";
        entry.lineEl.style.opacity = "0";
        entry.dotEl.style.opacity = "0";
        continue;
      }

      const sx = (this.tempVec.x * 0.5 + 0.5) * width;
      const sy = (-this.tempVec.y * 0.5 + 0.5) * height;

      const dir = spec.side === "right" ? 1 : -1;
      const elbowX = sx + dir * 42;
      const elbowY = sy - 26 + (i - 1) * 18;
      const endX = Math.max(96, Math.min(width - 260, elbowX + dir * 68));
      const endY = Math.max(96, Math.min(height - 110, elbowY));

      entry.dotEl.setAttribute("cx", sx.toFixed(1));
      entry.dotEl.setAttribute("cy", sy.toFixed(1));
      entry.dotEl.style.opacity = edgeFade.toFixed(2);

      entry.lineEl.setAttribute(
        "points",
        `${sx.toFixed(1)},${sy.toFixed(1)} ${elbowX.toFixed(1)},${endY.toFixed(1)} ${endX.toFixed(1)},${endY.toFixed(1)}`,
      );
      entry.lineEl.style.opacity = (edgeFade * 0.72).toFixed(2);

      entry.codeEl.textContent = spec.code;
      entry.labelEl.textContent = spec.label;
      entry.detailEl.textContent = spec.detail;

      entry.groupEl.dataset.side = spec.side;
      entry.groupEl.style.opacity = edgeFade.toFixed(2);
      entry.groupEl.style.transform =
        spec.side === "right"
          ? `translate3d(${(endX + 8).toFixed(1)}px, ${(endY - 14).toFixed(1)}px, 0)`
          : `translate3d(${(endX - 248).toFixed(1)}px, ${(endY - 14).toFixed(1)}px, 0)`;
    }
  }
}
