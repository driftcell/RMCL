import * as THREE from "three";
import { ACCENT_PALETTES, type AccentKey } from "../data/rmcl-spec";
import { VoxelCoreSystem } from "./voxel-core";

export class RmclSceneStage {
  public readonly scene = new THREE.Scene();
  public readonly camera: THREE.PerspectiveCamera;
  public readonly renderer: THREE.WebGLRenderer | null = null;
  public readonly voxelCore: VoxelCoreSystem;

  private rimLight: THREE.PointLight;
  private pointerLight: THREE.PointLight;
  private keyLight: THREE.DirectionalLight;
  private targetAccentColor = new THREE.Color(ACCENT_PALETTES.emerald.primary);

  constructor(container: HTMLElement) {
    this.scene.background = new THREE.Color("#090E0B");
    this.scene.fog = new THREE.FogExp2("#090E0B", 0.068);

    this.camera = new THREE.PerspectiveCamera(
      40,
      window.innerWidth / Math.max(1, window.innerHeight),
      0.1,
      80,
    );
    this.camera.position.set(0, 0.25, 7.6);

    // Ambient & Key Lighting
    const hemi = new THREE.HemisphereLight("#D8F3E5", "#070D0A", 0.72);
    this.scene.add(hemi);

    this.keyLight = new THREE.DirectionalLight("#F4FFF9", 2.1);
    this.keyLight.position.set(5.5, 8.2, 6.4);
    this.scene.add(this.keyLight);

    const fillLight = new THREE.DirectionalLight("#2B5F46", 0.85);
    fillLight.position.set(-6.0, -3.5, 4.0);
    this.scene.add(fillLight);

    this.rimLight = new THREE.PointLight(ACCENT_PALETTES.emerald.primary, 6.5, 16, 1.6);
    this.rimLight.position.set(3.2, 2.4, -3.5);
    this.scene.add(this.rimLight);

    this.pointerLight = new THREE.PointLight(ACCENT_PALETTES.emerald.specular, 3.8, 11, 1.8);
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
      renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
      renderer.setSize(window.innerWidth, window.innerHeight);
      renderer.toneMapping = THREE.ACESFilmicToneMapping;
      renderer.toneMappingExposure = 1.12;
      container.appendChild(renderer.domElement);
      this.renderer = renderer;
    } catch {
      container.setAttribute("data-webgl-fallback", "true");
    }
  }

  public setAccent(key: AccentKey): void {
    const palette = ACCENT_PALETTES[key];
    this.targetAccentColor.set(palette.primary);
    this.voxelCore.setAccent(key);
  }

  public resize(width: number, height: number): void {
    const safeH = Math.max(1, height);
    this.camera.aspect = width / safeH;
    // Slightly wider FOV on portrait viewports so the 3x3x3 assembly never clips
    this.camera.fov = width < 860 ? 48 : 38;
    this.camera.updateProjectionMatrix();
    this.renderer?.setSize(width, safeH);
  }

  public updateAndRender(
    dt: number,
    elapsed: number,
    scroll: number,
    scrollVelocity: number,
    pointerX: number,
    pointerY: number,
    isMobile: boolean,
    reducedMotion: boolean,
  ): void {
    const colorLerp = reducedMotion ? 1.0 : 1 - Math.exp(-dt * 6);
    this.rimLight.color.lerp(this.targetAccentColor, colorLerp);
    this.pointerLight.color.lerp(this.targetAccentColor, colorLerp);

    // Move pointer light in screen space to sweep specular reflections across voxel bevels
    this.pointerLight.position.set(pointerX * 4.5, pointerY * 3.2, 4.2);

    // Camera distance & vertical framing modulated by scroll stage
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

    const lerp = reducedMotion ? 1.0 : 1 - Math.exp(-dt * 7.5);
    this.camera.position.z = THREE.MathUtils.lerp(
      this.camera.position.z,
      stageZoom + mobileExtraZ,
      lerp,
    );
    this.camera.position.x = THREE.MathUtils.lerp(
      this.camera.position.x,
      pointerX * 0.18,
      lerp,
    );
    this.camera.position.y = THREE.MathUtils.lerp(
      this.camera.position.y,
      0.18 + pointerY * 0.14,
      lerp,
    );
    this.camera.lookAt(0, isMobile ? -0.35 : 0, 0);

    this.voxelCore.update(
      dt,
      elapsed,
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
  }
}
