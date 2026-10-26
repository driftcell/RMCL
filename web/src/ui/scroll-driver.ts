import { CHAPTERS } from "../data/rmcl-spec";

export interface ScrollStateSnapshot {
  rawProgress: number;
  smoothProgress: number;
  velocity: number;
  activeChapterIndex: number;
  chapterLocalProgress: number;
}

export class ScrollDriver {
  private rawProgress = 0;
  private smoothProgress = 0;
  private velocity = 0;
  private maxScroll = 1;

  constructor() {
    this.recalcBounds();
    window.addEventListener("scroll", () => this.onScroll(), { passive: true });
    window.addEventListener("resize", () => this.recalcBounds(), { passive: true });
  }

  public recalcBounds(): void {
    const docHeight = Math.max(
      document.body.scrollHeight,
      document.documentElement.scrollHeight,
    );
    this.maxScroll = Math.max(1, docHeight - window.innerHeight);
    this.onScroll();
  }

  private onScroll(): void {
    const y = Math.max(0, window.scrollY || window.pageYOffset || 0);
    this.rawProgress = Math.max(0, Math.min(1, y / this.maxScroll));
  }

  public scrollToChapter(index: number, reducedMotion: boolean): void {
    const chapter = CHAPTERS[index];
    if (!chapter) return;
    // Target the sweet-spot center of the chapter range (or 0 for chapter 0, 0.96 for chapter 4)
    const targetProgress =
      index === 0
        ? 0
        : index === CHAPTERS.length - 1
          ? 0.94
          : (chapter.range[0] + chapter.range[1]) * 0.5;
    const targetY = targetProgress * this.maxScroll;
    window.scrollTo({
      top: targetY,
      behavior: reducedMotion ? "auto" : "smooth",
    });
  }

  public step(dt: number, reducedMotion: boolean): ScrollStateSnapshot {
    if (reducedMotion) {
      this.velocity = 0;
      this.smoothProgress = this.rawProgress;
    } else {
      const prev = this.smoothProgress;
      const alpha = 1 - Math.exp(-dt * 11.5);
      this.smoothProgress += (this.rawProgress - this.smoothProgress) * alpha;
      const rawVel = (this.smoothProgress - prev) / Math.max(0.001, dt);
      this.velocity = this.velocity * 0.82 + rawVel * 0.18;
    }

    const p = this.smoothProgress;
    let activeChapterIndex = 0;
    for (let i = 0; i < CHAPTERS.length; i++) {
      const [start, end] = CHAPTERS[i].range;
      if (p >= start && p <= end) {
        activeChapterIndex = i;
        break;
      }
      if (p > end) {
        activeChapterIndex = i;
      }
    }

    const [cStart, cEnd] = CHAPTERS[activeChapterIndex].range;
    const span = Math.max(0.001, cEnd - cStart);
    const chapterLocalProgress = Math.max(0, Math.min(1, (p - cStart) / span));

    return {
      rawProgress: this.rawProgress,
      smoothProgress: this.smoothProgress,
      velocity: this.velocity,
      activeChapterIndex,
      chapterLocalProgress,
    };
  }
}
