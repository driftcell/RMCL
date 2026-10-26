import * as THREE from "three";
import { ACCENT_PALETTES, type AccentKey, type ThemeMode } from "../data/rmcl-spec";
import { VoxelCoreSystem } from "./voxel-core";

export class RmclSceneStage {
  public readonly scene = new THREE.Scene();
  public readonly camera: THREE.PerspectiveCamera;
  public readonly renderer: THREE.WebGLRenderer | null = null;
  public readonly voxelCore: VoxelCoreSystem;

  private rimLight: THREE.PointLight;
  private pointerLight: THREE.PointLight;
  private hemiLight: THREE.HemisphereLight;
  private keyLight: THREE.DirectionalLight;

  private targetBgColor = new THREE.Color("#F6F8F7");
  private liveBgColor = new THREE.Color("#F6F8F7");
  private targetAccentColor = new THREE.Color(ACCENT_PALETTES.emerald.darkPrimary);
  private fog: THREE.FogExp2;

  constructor(container: HTMLElement) {
    this.scene.background = this.liveBgColor;
    this.fog = new THREE.FogExp2("#F6F8F7", 0.045);
    this.scene.fog = this.fog;

    this.camera = new THREE.PerspectiveCamera(
      38,
      window.innerWidth / Math.max(1, window.innerHeight),
      0.1,
      65,
    );
    this.camera.position.set(0, 0.25, 7.5);

    this.hemiLight = new THREE.HemisphereLight("#FFFFFF", "#D8E5DF", 1.25);
    this.scene.add(this.hemiLight);

    this.keyLight = new THREE.DirectionalLight("#FFFFFF", 2.2);
    this.keyLight.position.set(5.5, 8.2, 6.4);
    this.scene.add(this.keyLight);

    const fillLight = new THREE.DirectionalLight("#BFE3D0", 0.95);
    fillLight.position.set(-6.0, -3.5, 4.0);
    this.scene.add(fillLight);

    this.rimLight = new THREE.PointLight(ACCENT_PALETTES.emerald.darkPrimary, 5.2, 16, 1.6);
    this.rimLight.position.set(3.2, 2.4, -3.5);
    this.scene.add(this.rimLight);

    this.pointerLight = new THREE.PointLight(
      ACCENT_PALETTES.emerald.darkSpecular,
      3.2,
      11,
      1.8,
    );
    this.pointerLight.position.set(0, 0, 4.2);
    this.scene.add(this.pointerLight);

    this.voxelCore = new VoxelCoreSystem();
    this.scene.add(this.voxelCore.root);

    try {
      const renderer = new THREE.WebGLRenderer({
        antialias: true,
        powerPreference: "high-performance",
        alpha: false,
      });
      // Cap pixel ratio at 1.5 to prevent 4K/Retina fill-rate bottlenecks
      renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.5));
      renderer.setSize(window.innerWidth, window.innerHeight);
      renderer.toneMapping = THREE.ACESFilmicToneMapping;
      renderer.toneMappingExposure = 1.08;
      container.appendChild(renderer.domElement);
      this.renderer = renderer;
    } catch {
      container.setAttribute("data-webgl-fallback", "true");
    }
  }

  public setThemeAndAccent(mode: ThemeMode, key: AccentKey): void {
    const palette = ACCENT_PALETTES[key];
    this.targetAccentColor.set(palette.darkPrimary);
    if (mode === "light") {
      this.targetBgColor.set("#F6F8F7");
      this.hemiLight.intensity = 1.3;
      this.keyLight.intensity = 2.2;
      this.fog.density = 0.042;
    } else {
      this.targetBgColor.set("#090E0B");
      this.hemiLight.intensity = 0.75;
      this.keyLight.intensity = 2.0;
      this.fog.density = 0.065;
    }
    this.voxelCore.setThemeAndAccent(mode, key);
  }

  public resize(width: number, height: number): void {
    const safeH = Math.max(1, height);
    this.camera.aspect = width / safeH;
    this.camera.fov = width < 860 ? 48 : 38;
    this.camera.updateProjectionMatrix();
    this.renderer?.setSize(width, safeH);
  }

  public updateAndRender(
    dt: number,
    scroll: number,
    scrollVelocity: number,
    pointerX: number,
    pointerY: number,
    isMobile: boolean,
    reducedMotion: boolean,
  ): boolean {
    const colorLerp = reducedMotion ? 1.0 : 1 - Math.exp(-dt * 8);
    this.liveBgColor.lerp(this.targetBgColor, colorLerp);
    this.fog.color.copy(this.liveBgColor);
    this.rimLight.color.lerp(this.targetAccentColor, colorLerp);
    this.pointerLight.color.lerp(this.targetAccentColor, colorLerp);

    this.pointerLight.position.set(pointerX * 4.2, pointerY * 3.0, 4.2);

    const stageZoom =
      scroll < 0.2
        ? 7.35
        : scroll < 0.42
          ? 8.15
          : scroll < 0.64
            ? 8.35
            : scroll < 0.84
              ? 6.65
              : 7.45;
    const mobileExtraZ = isMobile ? 1.85 : 0;
    const targetCamZ = stageZoom + mobileExtraZ;
    const targetCamX = pointerX * 0.16;
    const targetCamY = 0.18 + pointerY * 0.12;

    const lerp = reducedMotion ? 1.0 : 1 - Math.exp(-dt * 9.0);
    const camDelta =
      Math.abs(this.camera.position.z - targetCamZ) +
      Math.abs(this.camera.position.x - targetCamX) +
      Math.abs(this.camera.position.y - targetCamY);

    this.camera.position.z = THREE.MathUtils.lerp(this.camera.position.z, targetCamZ, lerp);
    this.camera.position.x = THREE.MathUtils.lerp(this.camera.position.x, targetCamX, lerp);
    this.camera.position.y = THREE.MathUtils.lerp(this.camera.position.y, targetCamY, lerp);
    this.camera.lookAt(0, isMobile ? -0.35 : 0, 0);

    const coreMoving = this.voxelCore.update(
      dt,
      scroll,
      scrollVelocity,
      pointerX,
      pointerY,
      isMobile,
      reducedMotion,
    );

    if (this.renderer) {
      this.renderer.render(this.scene, this.camera);
    }

    return coreMoving || camDelta > 0.0003;
  }
}
