import { VOXEL_NODES } from "./data/rmcl-spec";
import { VoxelPicker } from "./render/picking";
import { RmclSceneStage } from "./render/scene";
import { DomController } from "./ui/dom-controller";
import { HudProjectionOverlay } from "./ui/hud-projection";
import { ScrollDriver } from "./ui/scroll-driver";

function bootstrap(): void {
  const stageContainer = document.getElementById("webgl-stage");
  const hudSvg = document.getElementById("hud-svg") as SVGSVGElement | null;
  const hudLabels = document.getElementById("hud-labels");

  if (!stageContainer || !hudSvg || !hudLabels) {
    throw new Error("Missing core stage containers in index.html");
  }

  const reducedMotionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
  let reducedMotion = reducedMotionQuery.matches;
  reducedMotionQuery.addEventListener("change", (ev) => {
    reducedMotion = ev.matches;
  });

  const sceneStage = new RmclSceneStage(stageContainer);
  const picker = new VoxelPicker();
  const scrollDriver = new ScrollDriver();
  const hudOverlay = new HudProjectionOverlay(hudSvg, hudLabels);

  let hoveredVoxelIndex: number | null = null;
  let lockedVoxelIndex: number | null = null;

  const domController = new DomController({
    onSelectChapter: (index) => {
      scrollDriver.scrollToChapter(index, reducedMotion);
    },
    onSelectAccent: (accent) => {
      sceneStage.setAccent(accent);
    },
    onClearVoxelSelection: () => {
      lockedVoxelIndex = null;
      sceneStage.voxelCore.setSelectedVoxel(null);
      const fallback =
        hoveredVoxelIndex !== null ? VOXEL_NODES[hoveredVoxelIndex] : VOXEL_NODES[2];
      domController.updateInspector(fallback, false);
    },
  });

  // Ensure initial scroll bounds are accurate after dynamic chapter injection
  scrollDriver.recalcBounds();

  // Pointer tracking & 3D subvoxel picking
  let targetPointerX = 0;
  let targetPointerY = 0;
  let smoothPointerX = 0;
  let smoothPointerY = 0;

  window.addEventListener(
    "pointermove",
    (ev) => {
      const w = Math.max(1, window.innerWidth);
      const h = Math.max(1, window.innerHeight);
      targetPointerX = (ev.clientX / w) * 2 - 1;
      targetPointerY = -((ev.clientY / h) * 2 - 1);

      const hitIndex = picker.pick(
        ev.clientX,
        ev.clientY,
        sceneStage.camera,
        sceneStage.voxelCore,
      );
      if (hitIndex !== hoveredVoxelIndex) {
        hoveredVoxelIndex = hitIndex;
        sceneStage.voxelCore.setHoveredVoxel(hitIndex);
        document.body.style.cursor = hitIndex !== null ? "pointer" : "";
        if (lockedVoxelIndex === null) {
          domController.updateInspector(
            hitIndex !== null ? VOXEL_NODES[hitIndex] : VOXEL_NODES[2],
            false,
          );
        }
      }
    },
    { passive: true },
  );

  window.addEventListener("click", (ev) => {
    // Ignore clicks on interactive UI elements
    const target = ev.target as HTMLElement | null;
    if (target?.closest("button, a, .chapter-column, .site-bar, .depth-rail, .voxel-inspector")) {
      return;
    }

    const hitIndex = picker.pick(
      ev.clientX,
      ev.clientY,
      sceneStage.camera,
      sceneStage.voxelCore,
    );

    if (hitIndex !== null) {
      lockedVoxelIndex = lockedVoxelIndex === hitIndex ? null : hitIndex;
      sceneStage.voxelCore.setSelectedVoxel(lockedVoxelIndex);
      domController.updateInspector(
        VOXEL_NODES[hitIndex],
        lockedVoxelIndex !== null,
      );
    } else if (lockedVoxelIndex !== null) {
      lockedVoxelIndex = null;
      sceneStage.voxelCore.setSelectedVoxel(null);
      domController.updateInspector(VOXEL_NODES[2], false);
    }
  });

  // Keyboard stage shortcuts (1..5)
  window.addEventListener("keydown", (ev) => {
    if (ev.target instanceof HTMLInputElement || ev.target instanceof HTMLTextAreaElement) {
      return;
    }
    if (ev.key >= "1" && ev.key <= "5") {
      const idx = Number(ev.key) - 1;
      scrollDriver.scrollToChapter(idx, reducedMotion);
    } else if (ev.key === "Escape" && lockedVoxelIndex !== null) {
      lockedVoxelIndex = null;
      sceneStage.voxelCore.setSelectedVoxel(null);
      domController.updateInspector(VOXEL_NODES[2], false);
    }
  });

  window.addEventListener(
    "resize",
    () => {
      sceneStage.resize(window.innerWidth, window.innerHeight);
      scrollDriver.recalcBounds();
    },
    { passive: true },
  );

  sceneStage.resize(window.innerWidth, window.innerHeight);

  let lastTime = performance.now();

  const tick = (now: number) => {
    const dt = Math.min(0.1, Math.max(0.001, (now - lastTime) * 0.001));
    lastTime = now;
    const elapsed = now * 0.001;

    const pLerp = reducedMotion ? 1.0 : 1 - Math.exp(-dt * 8.0);
    smoothPointerX += (targetPointerX - smoothPointerX) * pLerp;
    smoothPointerY += (targetPointerY - smoothPointerY) * pLerp;

    const scrollSnapshot = scrollDriver.step(dt, reducedMotion);
    const isMobile = window.innerWidth < 920;

    sceneStage.updateAndRender(
      dt,
      elapsed,
      scrollSnapshot.smoothProgress,
      scrollSnapshot.velocity,
      smoothPointerX,
      smoothPointerY,
      isMobile,
      reducedMotion,
    );

    hudOverlay.update(
      scrollSnapshot.activeChapterIndex,
      scrollSnapshot.chapterLocalProgress,
      sceneStage.camera,
      sceneStage.voxelCore,
      window.innerWidth,
      window.innerHeight,
      isMobile,
    );

    domController.syncWithScroll(scrollSnapshot, reducedMotion);

    requestAnimationFrame(tick);
  };

  requestAnimationFrame(tick);
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", bootstrap);
} else {
  bootstrap();
}
