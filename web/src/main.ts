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

  const sceneStage = new RmclSceneStage(stageContainer);
  const picker = new VoxelPicker();
  const scrollDriver = new ScrollDriver();
  const hudOverlay = new HudProjectionOverlay(hudSvg, hudLabels);

  let hoveredVoxelIndex: number | null = null;
  let lockedVoxelIndex: number | null = null;
  let rafId: number | null = null;
  let lastFrameTime = performance.now();
  let extraFramesRemaining = 4;

  const wakeLoop = (frames = 4) => {
    if (frames > extraFramesRemaining) {
      extraFramesRemaining = frames;
    }
    if (rafId === null) {
      lastFrameTime = performance.now();
      rafId = requestAnimationFrame(tick);
    }
  };

  reducedMotionQuery.addEventListener("change", (ev) => {
    reducedMotion = ev.matches;
    wakeLoop(6);
  });

  const domController = new DomController({
    onSelectChapter: (index) => {
      scrollDriver.scrollToChapter(index, reducedMotion);
      wakeLoop(30);
    },
    onThemeOrAccentChange: (mode, accent) => {
      sceneStage.setThemeAndAccent(mode, accent);
      wakeLoop(45);
    },
    onClearVoxelSelection: () => {
      lockedVoxelIndex = null;
      sceneStage.voxelCore.setSelectedVoxel(null);
      const fallback =
        hoveredVoxelIndex !== null ? VOXEL_NODES[hoveredVoxelIndex] : VOXEL_NODES[0];
      domController.updateInspector(fallback, false);
      wakeLoop(25);
    },
  });

  scrollDriver.recalcBounds();

  let targetPointerX = 0;
  let targetPointerY = 0;
  let smoothPointerX = 0;
  let smoothPointerY = 0;
  let lastClientX = -1;
  let lastClientY = -1;
  let pointerDirty = false;
  let lastPickTime = 0;

  window.addEventListener(
    "scroll",
    () => {
      wakeLoop(8);
    },
    { passive: true },
  );

  window.addEventListener(
    "pointermove",
    (ev) => {
      const dx = ev.clientX - lastClientX;
      const dy = ev.clientY - lastClientY;
      if (dx * dx + dy * dy < 4) return;

      lastClientX = ev.clientX;
      lastClientY = ev.clientY;
      const w = Math.max(1, window.innerWidth);
      const h = Math.max(1, window.innerHeight);
      targetPointerX = (ev.clientX / w) * 2 - 1;
      targetPointerY = -((ev.clientY / h) * 2 - 1);
      pointerDirty = true;
      wakeLoop(8);
    },
    { passive: true },
  );

  window.addEventListener("click", (ev) => {
    const target = ev.target as HTMLElement | null;
    if (
      target?.closest(
        "button, a, .chapter-column, .site-bar, .voxel-inspector",
      )
    ) {
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
      wakeLoop(30);
    } else if (lockedVoxelIndex !== null) {
      lockedVoxelIndex = null;
      sceneStage.voxelCore.setSelectedVoxel(null);
      domController.updateInspector(VOXEL_NODES[0], false);
      wakeLoop(30);
    }
  });

  window.addEventListener("keydown", (ev) => {
    if (
      ev.target instanceof HTMLInputElement ||
      ev.target instanceof HTMLTextAreaElement
    ) {
      return;
    }
    if (ev.key >= "1" && ev.key <= "5") {
      const idx = Number(ev.key) - 1;
      scrollDriver.scrollToChapter(idx, reducedMotion);
      wakeLoop(30);
    } else if (ev.key === "Escape" && lockedVoxelIndex !== null) {
      lockedVoxelIndex = null;
      sceneStage.voxelCore.setSelectedVoxel(null);
      domController.updateInspector(VOXEL_NODES[0], false);
      wakeLoop(25);
    }
  });

  window.addEventListener(
    "resize",
    () => {
      sceneStage.resize(window.innerWidth, window.innerHeight);
      scrollDriver.recalcBounds();
      wakeLoop(10);
    },
    { passive: true },
  );

  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) {
      wakeLoop(6);
    }
  });

  sceneStage.resize(window.innerWidth, window.innerHeight);

  function tick(now: number): void {
    rafId = null;
    if (document.hidden) return;

    const elapsedMs = now - lastFrameTime;
    // Cap to ~60fps on 120Hz displays to keep CPU/GPU cool
    if (elapsedMs < 14 && extraFramesRemaining <= 0) {
      rafId = requestAnimationFrame(tick);
      return;
    }

    const dt = Math.min(0.08, Math.max(0.004, elapsedMs * 0.001));
    lastFrameTime = now;

    // Throttled raycast (max ~15Hz while pointer moves)
    if (pointerDirty && now - lastPickTime > 64) {
      pointerDirty = false;
      lastPickTime = now;
      const hitIndex = picker.pick(
        lastClientX,
        lastClientY,
        sceneStage.camera,
        sceneStage.voxelCore,
      );
      if (hitIndex !== hoveredVoxelIndex) {
        hoveredVoxelIndex = hitIndex;
        sceneStage.voxelCore.setHoveredVoxel(hitIndex);
        document.body.style.cursor = hitIndex !== null ? "pointer" : "";
        if (lockedVoxelIndex === null) {
          domController.updateInspector(
            hitIndex !== null ? VOXEL_NODES[hitIndex] : VOXEL_NODES[0],
            false,
          );
        }
        extraFramesRemaining = Math.max(extraFramesRemaining, 20);
      }
    }

    const pLerp = reducedMotion ? 1.0 : 1 - Math.exp(-dt * 9.0);
    smoothPointerX += (targetPointerX - smoothPointerX) * pLerp;
    smoothPointerY += (targetPointerY - smoothPointerY) * pLerp;

    const pointerDelta =
      Math.abs(targetPointerX - smoothPointerX) +
      Math.abs(targetPointerY - smoothPointerY);

    const scrollSnapshot = scrollDriver.step(dt, reducedMotion);
    const scrollDelta =
      Math.abs(scrollSnapshot.rawProgress - scrollSnapshot.smoothProgress) +
      Math.abs(scrollSnapshot.velocity);

    const isMobile = window.innerWidth < 920;

    const sceneMoving = sceneStage.updateAndRender(
      dt,
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

    if (extraFramesRemaining > 0) {
      extraFramesRemaining--;
    }

    const needsNextFrame =
      extraFramesRemaining > 0 ||
      pointerDirty ||
      pointerDelta > 0.0015 ||
      scrollDelta > 0.0003 ||
      sceneMoving;

    if (needsNextFrame) {
      rafId = requestAnimationFrame(tick);
    }
  }

  wakeLoop(12);
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", bootstrap);
} else {
  bootstrap();
}
