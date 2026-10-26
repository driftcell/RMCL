import * as THREE from "three";
import {
  ACCENT_PALETTES,
  type AccentKey,
   type AccentPalette,
  VOXEL_NODES,
  type VoxelNodeSpec,
} from "../data/rmcl-spec";

export interface VoxelUnit {
  spec: VoxelNodeSpec;
  group: THREE.Group;
  bodyMesh: THREE.Mesh<THREE.BufferGeometry, THREE.MeshStandardMaterial>;
  corePlateMesh: THREE.Mesh<THREE.BufferGeometry, THREE.MeshStandardMaterial>;
  edgeLines: THREE.LineSegments<THREE.EdgesGeometry, THREE.LineBasicMaterial>;
  currentPos: THREE.Vector3;
  targetPos: THREE.Vector3;
  currentQuat: THREE.Quaternion;
  targetQuat: THREE.Quaternion;
  hoverAmount: number;
  selectedAmount: number;
}

function createMicroGridTexture(): THREE.CanvasTexture {
  const canvas = document.createElement("canvas");
  canvas.width = 256;
  canvas.height = 256;
  const ctx = canvas.getContext("2d");
  if (ctx) {
    ctx.fillStyle = "#16211C";
    ctx.fillRect(0, 0, 256, 256);

    // Subtle brushed diagonal grain
    ctx.strokeStyle = "rgba(255, 255, 255, 0.035)";
    ctx.lineWidth = 1;
    for (let i = -256; i < 512; i += 8) {
      ctx.beginPath();
      ctx.moveTo(i, 0);
      ctx.lineTo(i + 256, 256);
      ctx.stroke();
    }

    // Precision inset border
    ctx.strokeStyle = "rgba(255, 255, 255, 0.14)";
    ctx.lineWidth = 4;
    ctx.strokeRect(10, 10, 236, 236);

    // Corner fiducial ticks
    ctx.fillStyle = "rgba(255, 255, 255, 0.28)";
    const s = 18;
    ctx.fillRect(8, 8, s, 3);
    ctx.fillRect(8, 8, 3, s);
    ctx.fillRect(248 - s, 8, s, 3);
    ctx.fillRect(245, 8, 3, s);
    ctx.fillRect(8, 245, s, 3);
    ctx.fillRect(8, 248 - s, 3, s);
    ctx.fillRect(248 - s, 245, s, 3);
    ctx.fillRect(245, 248 - s, 3, s);
  }
  const tex = new THREE.CanvasTexture(canvas);
  tex.anisotropy = 4;
  tex.needsUpdate = true;
  return tex;
}

/**
 * Builds a chamfered cube geometry so every subvoxel catches directional specular highlights
 * along its 12 edges instead of looking like a flat default BoxGeometry.
 */
function createChamferedBoxGeometry(size = 0.68, bevel = 0.045): THREE.BufferGeometry {
  const geo = new THREE.BoxGeometry(size, size, size, 4, 4, 4);
  const pos = geo.attributes.position;
  const half = size * 0.5;
  const inner = half - bevel;
  const v = new THREE.Vector3();
  const clamped = new THREE.Vector3();

  for (let i = 0; i < pos.count; i++) {
    v.fromBufferAttribute(pos, i);
    clamped.set(
      Math.max(-inner, Math.min(inner, v.x)),
      Math.max(-inner, Math.min(inner, v.y)),
      Math.max(-inner, Math.min(inner, v.z)),
    );
    v.sub(clamped);
    if (v.lengthSq() > 1e-6) {
      v.normalize().multiplyScalar(bevel);
    }
    v.add(clamped);
    pos.setXYZ(i, v.x, v.y, v.z);
  }
  geo.computeVertexNormals();
  return geo;
}

function smoothstep(edge0: number, edge1: number, x: number): number {
  const t = Math.max(0, Math.min(1, (x - edge0) / (edge1 - edge0)));
  return t * t * (3 - 2 * t);
}

export class VoxelCoreSystem {
  public readonly root = new THREE.Group();
  public readonly rigGroup = new THREE.Group();
  public readonly units: VoxelUnit[] = [];
  public readonly pickableMeshes: THREE.Object3D[] = [];

  private conduitLines: THREE.LineSegments<THREE.BufferGeometry, THREE.LineBasicMaterial>;
  private conduitPositions: Float32Array;
  private pulsePoints: THREE.Points<THREE.BufferGeometry, THREE.PointsMaterial>;
  private pulseCoords: Float32Array;
  private pulseRings: Array<THREE.Mesh<THREE.RingGeometry, THREE.MeshBasicMaterial>> = [];
  private floorGrid: THREE.Group;

  private currentPalette: AccentPalette = ACCENT_PALETTES.emerald;
  private targetPrimaryColor = new THREE.Color(ACCENT_PALETTES.emerald.primary);
  private targetSpecularColor = new THREE.Color(ACCENT_PALETTES.emerald.specular);
  private targetSurfaceTintColor = new THREE.Color(ACCENT_PALETTES.emerald.surfaceTint);
  private livePrimaryColor = new THREE.Color(ACCENT_PALETTES.emerald.primary);
  private liveSpecularColor = new THREE.Color(ACCENT_PALETTES.emerald.specular);
  private liveSurfaceTintColor = new THREE.Color(ACCENT_PALETTES.emerald.surfaceTint);

  private hoveredIndex: number | null = null;
  private selectedIndex: number | null = null;

  // Connections between voxel indices for Strata & Pipeline conduits
  private readonly conduitPairs: Array<[number, number]> = [
    // Vertical stratum buses (L0 -> L1 -> L2)
    [0, 9], [9, 18],
    [2, 11], [11, 20],
    [4, 13], [13, 22],
    [6, 15], [15, 24],
    [8, 17], [17, 26],
    // Cross-stratum dependency links
    [1, 10], [10, 13],
    [3, 12], [12, 13],
    [5, 14], [14, 13],
    [7, 16], [16, 13],
    // Sequential pipeline chain
    [0, 1], [1, 2], [3, 4], [4, 5], [6, 7], [7, 8],
    [9, 10], [10, 11], [12, 13], [13, 14], [15, 16], [16, 17],
    [18, 19], [19, 20], [21, 22], [22, 23], [24, 25], [25, 26],
  ];

  constructor() {
    this.root.add(this.rigGroup);

    const sharedBoxGeo = createChamferedBoxGeometry(0.68, 0.042);
    const sharedEdgesGeo = new THREE.EdgesGeometry(new THREE.BoxGeometry(0.685, 0.685, 0.685));
    const sharedPlateGeo = new THREE.BoxGeometry(0.705, 0.52, 0.52);
    const surfaceTex = createMicroGridTexture();

    for (const spec of VOXEL_NODES) {
      const group = new THREE.Group();
      const [x, y, z] = spec.coords;

      const isTopLayer = y === 1;
      const baseColor = isTopLayer ? "#1B3B2B" : y === 0 ? "#131D18" : "#0D1411";

      const bodyMat = new THREE.MeshStandardMaterial({
        color: new THREE.Color(baseColor),
        roughness: isTopLayer ? 0.36 : 0.42,
        metalness: 0.68,
        bumpMap: surfaceTex,
        bumpScale: 0.008,
        emissive: new THREE.Color("#07120D"),
        emissiveIntensity: 0.15,
      });

      const bodyMesh = new THREE.Mesh(sharedBoxGeo, bodyMat);
      bodyMesh.castShadow = true;
      bodyMesh.receiveShadow = true;
      bodyMesh.userData.voxelIndex = spec.index;
      group.add(bodyMesh);
      this.pickableMeshes.push(bodyMesh);

      // Luminous accent plate on +X right-hand R-mark blocks or core nodes
      const plateMat = new THREE.MeshStandardMaterial({
        color: new THREE.Color(this.currentPalette.primary),
        emissive: new THREE.Color(this.currentPalette.primary),
        emissiveIntensity: spec.isRightRMark ? 0.85 : 0.18,
        roughness: 0.2,
        metalness: 0.3,
        transparent: true,
        opacity: spec.isRightRMark ? 0.95 : 0.22,
      });

      const corePlateMesh = new THREE.Mesh(sharedPlateGeo, plateMat);
      corePlateMesh.scale.set(
        spec.isRightRMark ? 1.0 : 0.78,
        spec.isRightRMark ? 0.82 : 0.78,
        spec.isRightRMark ? 0.82 : 0.78,
      );
      group.add(corePlateMesh);

      const edgeMat = new THREE.LineBasicMaterial({
        color: new THREE.Color(
          spec.isRightRMark ? this.currentPalette.specular : this.currentPalette.primary,
        ),
        transparent: true,
        opacity: spec.isRightRMark ? 0.62 : isTopLayer ? 0.38 : 0.2,
      });
      const edgeLines = new THREE.LineSegments(sharedEdgesGeo, edgeMat);
      group.add(edgeLines);

      const initPos = new THREE.Vector3(x * 0.76, y * 0.76, z * 0.76);
      group.position.copy(initPos);

      this.rigGroup.add(group);
      this.units.push({
        spec,
        group,
        bodyMesh,
        corePlateMesh,
        edgeLines,
        currentPos: initPos.clone(),
        targetPos: initPos.clone(),
        currentQuat: new THREE.Quaternion(),
        targetQuat: new THREE.Quaternion(),
        hoverAmount: 0,
        selectedAmount: 0,
      });
    }

    // Build data-bus conduit lines between connected voxel pairs
    this.conduitPositions = new Float32Array(this.conduitPairs.length * 6);
    const conduitGeo = new THREE.BufferGeometry();
    conduitGeo.setAttribute(
      "position",
      new THREE.BufferAttribute(this.conduitPositions, 3),
    );
    const conduitMat = new THREE.LineBasicMaterial({
      color: new THREE.Color(this.currentPalette.primary),
      transparent: true,
      opacity: 0.0,
    });
    this.conduitLines = new THREE.LineSegments(conduitGeo, conduitMat);
    this.rigGroup.add(this.conduitLines);

    // Build data-packet pulse particles travelling along the conduits
    const pulseCount = this.conduitPairs.length * 2;
    this.pulseCoords = new Float32Array(pulseCount * 3);
    const pulseGeo = new THREE.BufferGeometry();
    pulseGeo.setAttribute(
      "position",
      new THREE.BufferAttribute(this.pulseCoords, 3),
    );
    const pulseMat = new THREE.PointsMaterial({
      color: new THREE.Color(this.currentPalette.specular),
      size: 0.09,
      transparent: true,
      opacity: 0.0,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
    });
    this.pulsePoints = new THREE.Points(pulseGeo, pulseMat);
    this.rigGroup.add(this.pulsePoints);

    // Build 3 concentric launch-aperture shockwave rings for Stage 3 (Direct Launch)
    for (let i = 0; i < 3; i++) {
      const ringGeo = new THREE.RingGeometry(1.45 + i * 0.55, 1.48 + i * 0.55, 64);
      const ringMat = new THREE.MeshBasicMaterial({
        color: new THREE.Color(this.currentPalette.primary),
        side: THREE.DoubleSide,
        transparent: true,
        opacity: 0,
        blending: THREE.AdditiveBlending,
        depthWrite: false,
      });
      const ringMesh = new THREE.Mesh(ringGeo, ringMat);
      this.rigGroup.add(ringMesh);
      this.pulseRings.push(ringMesh);
    }

    // Build architectural reference floor grid below the core
    this.floorGrid = this.createArchitecturalFloor();
    this.root.add(this.floorGrid);
  }

  private createArchitecturalFloor(): THREE.Group {
    const group = new THREE.Group();
    group.position.set(0, -3.2, 0);

    const gridPoints: number[] = [];
    const extent = 9;
    const step = 1.5;
    for (let i = -extent; i <= extent; i += step) {
      gridPoints.push(-extent, 0, i, extent, 0, i);
      gridPoints.push(i, 0, -extent, i, 0, extent);
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.Float32BufferAttribute(gridPoints, 3));
    const mat = new THREE.LineBasicMaterial({
      color: new THREE.Color(this.currentPalette.primary),
      transparent: true,
      opacity: 0.06,
    });
    const lines = new THREE.LineSegments(geo, mat);
    group.add(lines);
    return group;
  }

  public setAccent(key: AccentKey): void {
    const palette = ACCENT_PALETTES[key];
    this.currentPalette = palette;
    this.targetPrimaryColor.set(palette.primary);
    this.targetSpecularColor.set(palette.specular);
    this.targetSurfaceTintColor.set(palette.surfaceTint);
  }

  public setHoveredVoxel(index: number | null): void {
    this.hoveredIndex = index;
  }

  public setSelectedVoxel(index: number | null): void {
    this.selectedIndex = index;
  }

  public getSelectedOrHoveredVoxel(): VoxelNodeSpec | null {
    if (this.selectedIndex !== null) {
      return VOXEL_NODES[this.selectedIndex] ?? null;
    }
    if (this.hoveredIndex !== null) {
      return VOXEL_NODES[this.hoveredIndex] ?? null;
    }
    return null;
  }

  public getVoxelWorldPosition(index: number, out = new THREE.Vector3()): THREE.Vector3 {
    const unit = this.units[index];
    if (!unit) return out.set(0, 0, 0);
    return unit.group.getWorldPosition(out);
  }

  /**
   * Computes the target position and orientation of each of the 27 subvoxels
   * as a deterministic function of scroll progress `scroll` in [0, 1].
   */
  public update(
    dt: number,
    elapsed: number,
    scroll: number,
    scrollVelocity: number,
    pointerX: number,
    pointerY: number,
    isMobile: boolean,
    reducedMotion: boolean,
  ): void {
    const lerpFactor = reducedMotion ? 1.0 : 1 - Math.exp(-dt * 9.5);
    const colorLerp = reducedMotion ? 1.0 : 1 - Math.exp(-dt * 6.0);

    // Smoothly transition palette colors
    this.livePrimaryColor.lerp(this.targetPrimaryColor, colorLerp);
    this.liveSpecularColor.lerp(this.targetSpecularColor, colorLerp);
    this.liveSurfaceTintColor.lerp(this.targetSurfaceTintColor, colorLerp);

    // Blend weights for the 5 scroll chapters:
    // Stage 0: [0.00, 0.20] Monolith
    // Stage 1: [0.20, 0.42] Stratified Isolation
    // Stage 2: [0.42, 0.64] Verified Pipeline
    // Stage 3: [0.64, 0.84] Direct Launch Aperture
    // Stage 4: [0.84, 1.00] Convergence
    const w0to1 = smoothstep(0.14, 0.26, scroll);
    const w1to2 = smoothstep(0.37, 0.48, scroll);
    const w2to3 = smoothstep(0.59, 0.70, scroll);
    const w3to4 = smoothstep(0.80, 0.90, scroll);

    const w0 = 1 - w0to1;
    const w1 = w0to1 * (1 - w1to2);
    const w2 = w1to2 * (1 - w2to3);
    const w3 = w2to3 * (1 - w3to4);
    const w4 = w3to4;

    // Rig horizontal staging offset: alternates sides on desktop so 3D core and editorial text
    // balance each other across vertical scroll chapters.
    const desktopSpread = isMobile ? 0 : 1.72;
    const targetRigX =
      (w0 * 1.05 + w1 * -0.95 + w2 * 0.98 + w3 * -0.92 + w4 * 1.02) * desktopSpread;
    const targetRigY = isMobile
      ? w0 * 0.55 + w1 * 0.35 + w2 * 0.45 + w3 * 0.4 + w4 * 0.65
      : w0 * 0.05 + w1 * -0.05 + w2 * 0.08 + w3 * 0.0 + w4 * 0.12;
    const targetRigZ = w0 * 0.0 + w1 * -0.35 + w2 * -0.5 + w3 * 0.35 + w4 * 0.15;

    this.rigGroup.position.x = THREE.MathUtils.lerp(
      this.rigGroup.position.x,
      targetRigX + pointerX * 0.22,
      lerpFactor,
    );
    this.rigGroup.position.y = THREE.MathUtils.lerp(
      this.rigGroup.position.y,
      targetRigY + pointerY * 0.18,
      lerpFactor,
    );
    this.rigGroup.position.z = THREE.MathUtils.lerp(
      this.rigGroup.position.z,
      targetRigZ,
      lerpFactor,
    );

    // Overall rig rotation choreographed by scroll progress + pointer parallax + scroll velocity impulse
    const idleSpin = reducedMotion ? 0 : Math.sin(elapsed * 0.45) * 0.04;
    const targetRotX =
      w0 * 0.48 +
      w1 * 0.34 +
      w2 * 0.24 +
      w3 * 0.06 +
      w4 * 0.46 -
      pointerY * 0.18 +
      Math.max(-0.25, Math.min(0.25, scrollVelocity * 0.08));

    const targetRotY =
      w0 * -0.76 +
      w1 * (-0.76 + scroll * 1.6) +
      w2 * 0.52 +
      w3 * 0.0 +
      w4 * -0.78 +
      pointerX * 0.26 +
      idleSpin;

    const targetRotZ = w2 * -0.08 + w3 * (scroll - 0.74) * 1.2;

    this.rigGroup.rotation.x = THREE.MathUtils.lerp(
      this.rigGroup.rotation.x,
      targetRotX,
      lerpFactor,
    );
    this.rigGroup.rotation.y = THREE.MathUtils.lerp(
      this.rigGroup.rotation.y,
      targetRotY,
      lerpFactor,
    );
    this.rigGroup.rotation.z = THREE.MathUtils.lerp(
      this.rigGroup.rotation.z,
      targetRotZ,
      lerpFactor,
    );

    // Local progress inside Stage 2 (pipeline wave) and Stage 3 (aperture pulse)
    const stage2Progress = smoothstep(0.42, 0.64, scroll);
    const stage3Progress = smoothstep(0.64, 0.84, scroll);
    const velocityGapBoost = Math.min(0.24, Math.abs(scrollVelocity) * 0.06);

    const p0 = new THREE.Vector3();
    const p1 = new THREE.Vector3();
    const p2 = new THREE.Vector3();
    const p3 = new THREE.Vector3();
    const p4 = new THREE.Vector3();
    const euler = new THREE.Euler();
    const darkMoss = new THREE.Color("#0D1511");
    const midObsidian = new THREE.Color("#131E18");

    for (let i = 0; i < this.units.length; i++) {
      const unit = this.units[i];
      const [cx, cy, cz] = unit.spec.coords;

      // Stage 0: Assembled 3x3x3 Monolith (with pre-unlock breathing as scroll approaches 0.20)
      const unlockBreath = smoothstep(0.05, 0.2, scroll) * 0.16;
      const s0Spacing = 0.75 + unlockBreath + velocityGapBoost;
      p0.set(cx * s0Spacing, cy * s0Spacing, cz * s0Spacing);

      // Stage 1: Stratified Isolation (3 horizontal strata separated along Y, radial expansion in X/Z)
      const layerTwist = cy * 0.36;
      const cosT = Math.cos(layerTwist);
      const sinT = Math.sin(layerTwist);
      const rx = cx * cosT - cz * sinT;
      const rz = cx * sinT + cz * cosT;
      const s1Radial = 1.02 + (cy === 0 ? 0.14 : 0);
      p1.set(rx * s1Radial, cy * 1.92, rz * s1Radial);

      // Stage 2: Verified Pipeline (3 spatial columns representing Preflight -> SHA-1 Verify -> Publish)
      const col = cy; // -1, 0, +1 -> 3 pipeline stages along X
      const rowIdx = (cz + 1) * 3 + (cx + 1); // 0..8 within each stage
      const ringAngle = (rowIdx / 9) * Math.PI * 2 + stage2Progress * Math.PI * 0.8;
      const radius = rowIdx === 4 ? 0.0 : 1.28;
      p2.set(
        -col * 1.85,
        Math.cos(ringAngle) * radius,
        Math.sin(ringAngle) * radius,
      );

      // Stage 3: Direct Launch Conduit (Axial portal aperture aligned with Z axis)
      if (i < 16) {
        const a = (i / 16) * Math.PI * 2 + stage3Progress * 0.9;
        const r = 2.25;
        p3.set(Math.cos(a) * r, Math.sin(a) * r, -0.55 + (i % 2) * 0.28);
      } else if (i < 24) {
        const a = ((i - 16) / 8) * Math.PI * 2 - stage3Progress * 1.3;
        const r = 1.25;
        p3.set(Math.cos(a) * r, Math.sin(a) * r, 0.45);
      } else {
        const coreIdx = i - 24; // 0, 1, 2 along Z axis
        p3.set(0, 0, -1.35 + coreIdx * 1.15);
      }

      // Stage 4: Reconverged precision monolith
      const s4Spacing = 0.75 + velocityGapBoost * 0.5;
      p4.set(cx * s4Spacing, cy * s4Spacing, cz * s4Spacing);

      // Combine all 5 stage coordinates
      unit.targetPos
        .set(0, 0, 0)
        .addScaledVector(p0, w0)
        .addScaledVector(p1, w1)
        .addScaledVector(p2, w2)
        .addScaledVector(p3, w3)
        .addScaledVector(p4, w4);

      // Hover & Selection normal displacement
      const isHovered = this.hoveredIndex === i;
      const isSelected = this.selectedIndex === i;
      unit.hoverAmount = THREE.MathUtils.lerp(
        unit.hoverAmount,
        isHovered ? 1 : 0,
        lerpFactor,
      );
      unit.selectedAmount = THREE.MathUtils.lerp(
        unit.selectedAmount,
        isSelected ? 1 : 0,
        lerpFactor,
      );

      const lift = unit.hoverAmount * 0.24 + unit.selectedAmount * 0.32;
      if (lift > 0.001) {
        const radialDir = unit.targetPos.clone();
        if (radialDir.lengthSq() < 0.01) radialDir.set(0, 1, 1);
        radialDir.normalize();
        unit.targetPos.addScaledVector(radialDir, lift);
      }

      unit.currentPos.lerp(unit.targetPos, lerpFactor);
      unit.group.position.copy(unit.currentPos);

      // Subvoxel local rotation
      euler.set(
        w2 * (rowIdx * 0.2) + w3 * (i * 0.15),
        w1 * layerTwist + w2 * (stage2Progress * 0.8),
        w3 * (i < 16 ? (i / 16) * Math.PI * 2 : 0),
      );
      unit.targetQuat.setFromEuler(euler);
      unit.currentQuat.slerp(unit.targetQuat, lerpFactor);
      unit.group.quaternion.copy(unit.currentQuat);

      // Dynamic material & seam illumination per stage
      const pipelineWave =
        w2 * Math.max(0, 1 - Math.abs(stage2Progress * 27 - i) * 0.28);
      const apertureGlow = w3 * (i >= 24 ? 1.0 : 0.45);
      const strataGlow = w1 * (cy === 0 ? 0.65 : 0.3);
      const rMarkGlow = (w0 + w4) * (unit.spec.isRightRMark ? 0.92 : 0.12);

      const highlightBoost =
        Math.max(rMarkGlow, strataGlow, pipelineWave, apertureGlow) +
        unit.hoverAmount * 0.85 +
        unit.selectedAmount * 1.1;

      if (cy === 1) {
        unit.bodyMesh.material.color.copy(this.liveSurfaceTintColor);
      } else if (cy === 0) {
        unit.bodyMesh.material.color.copy(midObsidian);
      } else {
        unit.bodyMesh.material.color.copy(darkMoss);
      }

      unit.bodyMesh.material.emissive.copy(this.livePrimaryColor);
      unit.bodyMesh.material.emissiveIntensity =
        0.04 + highlightBoost * 0.22;

      unit.corePlateMesh.material.color.copy(
        unit.spec.isRightRMark ? this.liveSpecularColor : this.livePrimaryColor,
      );
      unit.corePlateMesh.material.emissive.copy(this.livePrimaryColor);
      unit.corePlateMesh.material.emissiveIntensity =
        0.2 + highlightBoost * 1.05;
      unit.corePlateMesh.material.opacity = Math.min(
        1,
        0.18 + highlightBoost * 0.82,
      );

      unit.edgeLines.material.color.copy(
        unit.hoverAmount > 0.2 || unit.selectedAmount > 0.2 || unit.spec.isRightRMark
          ? this.liveSpecularColor
          : this.livePrimaryColor,
      );
      unit.edgeLines.material.opacity = Math.min(
        0.95,
        0.18 + highlightBoost * 0.65,
      );
    }

    // Update data-bus conduit line endpoints and pulse particles
    const conduitVisibility = Math.max(w1 * 0.52, w2 * 0.64, w3 * 0.28);
    this.conduitLines.material.color.copy(this.livePrimaryColor);
    this.conduitLines.material.opacity = conduitVisibility;

    this.pulsePoints.material.color.copy(this.liveSpecularColor);
    this.pulsePoints.material.opacity = Math.min(1, conduitVisibility * 1.5);

    const posAttr = this.conduitLines.geometry.attributes
      .position as THREE.BufferAttribute;
    const pulseAttr = this.pulsePoints.geometry.attributes
      .position as THREE.BufferAttribute;

    for (let c = 0; c < this.conduitPairs.length; c++) {
      const [idxA, idxB] = this.conduitPairs[c];
      const posA = this.units[idxA].currentPos;
      const posB = this.units[idxB].currentPos;

      const base = c * 6;
      this.conduitPositions[base] = posA.x;
      this.conduitPositions[base + 1] = posA.y;
      this.conduitPositions[base + 2] = posA.z;
      this.conduitPositions[base + 3] = posB.x;
      this.conduitPositions[base + 4] = posB.y;
      this.conduitPositions[base + 5] = posB.z;

      // Two pulse packets per conduit line, driven by scroll + elapsed time
      for (let p = 0; p < 2; p++) {
        const phase = reducedMotion
          ? (scroll * 3 + c * 0.13 + p * 0.5) % 1
          : (elapsed * 0.55 + scroll * 2.5 + c * 0.17 + p * 0.5) % 1;
        const pIdx = (c * 2 + p) * 3;
        this.pulseCoords[pIdx] = THREE.MathUtils.lerp(posA.x, posB.x, phase);
        this.pulseCoords[pIdx + 1] = THREE.MathUtils.lerp(posA.y, posB.y, phase);
        this.pulseCoords[pIdx + 2] = THREE.MathUtils.lerp(posA.z, posB.z, phase);
      }
    }
    posAttr.needsUpdate = true;
    pulseAttr.needsUpdate = true;

    // Update Stage 3 Direct Launch shockwave rings
    for (let r = 0; r < this.pulseRings.length; r++) {
      const ring = this.pulseRings[r];
      ring.material.color.copy(this.livePrimaryColor);
      if (w3 > 0.01) {
        const cycle = reducedMotion
          ? (r + 1) * 0.28
          : (elapsed * 0.45 + scroll * 4.0 + r * 0.33) % 1;
        ring.position.z = -1.8 + cycle * 3.8;
        const scale = 0.65 + cycle * 0.75;
        ring.scale.set(scale, scale, 1);
        ring.material.opacity = w3 * Math.sin(cycle * Math.PI) * 0.42;
      } else {
        ring.material.opacity = 0;
      }
    }
  }
}
