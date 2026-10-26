import * as THREE from "three";
import {
  ACCENT_PALETTES,
  type AccentKey,
  type AccentPalette,
  type ThemeMode,
  VOXEL_NODES,
  type VoxelNodeSpec,
} from "../data/rmcl-spec";

export interface VoxelUnit {
  spec: VoxelNodeSpec;
  group: THREE.Group;
  bodyMesh: THREE.Mesh<THREE.BufferGeometry, THREE.MeshStandardMaterial>;
  corePlateMesh: THREE.Mesh<THREE.BufferGeometry, THREE.MeshBasicMaterial>;
  edgeLines: THREE.LineSegments<THREE.EdgesGeometry, THREE.LineBasicMaterial>;
  currentPos: THREE.Vector3;
  targetPos: THREE.Vector3;
  currentQuat: THREE.Quaternion;
  targetQuat: THREE.Quaternion;
  hoverAmount: number;
  selectedAmount: number;
}

/**
 * Lightweight chamfered cube (2x2x2 segments = 48 triangles per cube instead of 192),
 * giving crisp specular bevels at 1/4th the vertex count.
 */
function createChamferedBoxGeometry(size = 0.68, bevel = 0.042): THREE.BufferGeometry {
  const geo = new THREE.BoxGeometry(size, size, size, 2, 2, 2);
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
  private floorLines!: THREE.LineSegments<THREE.BufferGeometry, THREE.LineBasicMaterial>;

  private currentPalette: AccentPalette = ACCENT_PALETTES.emerald;
  private themeMode: ThemeMode = "light";

  private targetPrimaryColor = new THREE.Color(ACCENT_PALETTES.emerald.darkPrimary);
  private targetSpecularColor = new THREE.Color(ACCENT_PALETTES.emerald.darkSpecular);
  private targetTopBlockColor = new THREE.Color(ACCENT_PALETTES.emerald.lightPrimary);
  private targetMidBlockColor = new THREE.Color("#1E2F27");
  private targetBotBlockColor = new THREE.Color("#14201B");

  private livePrimaryColor = new THREE.Color(ACCENT_PALETTES.emerald.darkPrimary);
  private liveSpecularColor = new THREE.Color(ACCENT_PALETTES.emerald.darkSpecular);
  private liveTopBlockColor = new THREE.Color(ACCENT_PALETTES.emerald.lightPrimary);
  private liveMidBlockColor = new THREE.Color("#1E2F27");
  private liveBotBlockColor = new THREE.Color("#14201B");

  // Pre-allocated scratch objects to guarantee zero GC allocations per frame
  private readonly scratchP0 = new THREE.Vector3();
  private readonly scratchP1 = new THREE.Vector3();
  private readonly scratchP2 = new THREE.Vector3();
  private readonly scratchP3 = new THREE.Vector3();
  private readonly scratchP4 = new THREE.Vector3();
  private readonly scratchRadial = new THREE.Vector3();
  private readonly scratchEuler = new THREE.Euler();

  private hoveredIndex: number | null = null;
  private selectedIndex: number | null = null;

  private readonly conduitPairs: Array<[number, number]> = [
    [0, 9], [9, 18],
    [2, 11], [11, 20],
    [4, 13], [13, 22],
    [6, 15], [15, 24],
    [8, 17], [17, 26],
    [1, 10], [10, 13],
    [3, 12], [12, 13],
    [5, 14], [14, 13],
    [7, 16], [16, 13],
    [0, 1], [1, 2], [3, 4], [4, 5], [6, 7], [7, 8],
    [9, 10], [10, 11], [12, 13], [13, 14], [15, 16], [16, 17],
    [18, 19], [19, 20], [21, 22], [22, 23], [24, 25], [25, 26],
  ];

  constructor() {
    this.root.add(this.rigGroup);

    const sharedBoxGeo = createChamferedBoxGeometry(0.68, 0.042);
    const sharedEdgesGeo = new THREE.EdgesGeometry(new THREE.BoxGeometry(0.686, 0.686, 0.686));
    const sharedPlateGeo = new THREE.BoxGeometry(0.704, 0.52, 0.52);

    for (const spec of VOXEL_NODES) {
      const group = new THREE.Group();
      const [x, y, z] = spec.coords;
      const isTopLayer = y === 1;

      const bodyMat = new THREE.MeshStandardMaterial({
        color: new THREE.Color(isTopLayer ? "#1F8A5B" : y === 0 ? "#1E2F27" : "#14201B"),
        roughness: isTopLayer ? 0.34 : 0.42,
        metalness: 0.48,
        emissive: new THREE.Color("#07120D"),
        emissiveIntensity: 0.08,
      });

      const bodyMesh = new THREE.Mesh(sharedBoxGeo, bodyMat);
      bodyMesh.userData.voxelIndex = spec.index;
      group.add(bodyMesh);
      this.pickableMeshes.push(bodyMesh);

      const plateMat = new THREE.MeshBasicMaterial({
        color: new THREE.Color(this.currentPalette.darkPrimary),
        transparent: true,
        opacity: spec.isRightRMark ? 0.95 : 0.2,
      });

      const corePlateMesh = new THREE.Mesh(sharedPlateGeo, plateMat);
      corePlateMesh.scale.set(
        spec.isRightRMark ? 1.0 : 0.78,
        spec.isRightRMark ? 0.82 : 0.78,
        spec.isRightRMark ? 0.82 : 0.78,
      );
      group.add(corePlateMesh);

      const edgeMat = new THREE.LineBasicMaterial({
        color: new THREE.Color(this.currentPalette.darkSpecular),
        transparent: true,
        opacity: spec.isRightRMark ? 0.7 : isTopLayer ? 0.42 : 0.24,
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

    // Data-bus conduit lines
    this.conduitPositions = new Float32Array(this.conduitPairs.length * 6);
    const conduitGeo = new THREE.BufferGeometry();
    conduitGeo.setAttribute(
      "position",
      new THREE.BufferAttribute(this.conduitPositions, 3),
    );
    const conduitMat = new THREE.LineBasicMaterial({
      color: new THREE.Color(this.currentPalette.lightPrimary),
      transparent: true,
      opacity: 0.0,
    });
    this.conduitLines = new THREE.LineSegments(conduitGeo, conduitMat);
    this.rigGroup.add(this.conduitLines);

    // Data-packet pulse points along conduits (scroll-driven)
    const pulseCount = this.conduitPairs.length * 2;
    this.pulseCoords = new Float32Array(pulseCount * 3);
    const pulseGeo = new THREE.BufferGeometry();
    pulseGeo.setAttribute(
      "position",
      new THREE.BufferAttribute(this.pulseCoords, 3),
    );
    const pulseMat = new THREE.PointsMaterial({
      color: new THREE.Color(this.currentPalette.darkPrimary),
      size: 0.095,
      transparent: true,
      opacity: 0.0,
      depthWrite: false,
    });
    this.pulsePoints = new THREE.Points(pulseGeo, pulseMat);
    this.rigGroup.add(this.pulsePoints);

    // 3 Launch aperture rings for Stage 3
    for (let i = 0; i < 3; i++) {
      const ringGeo = new THREE.RingGeometry(1.45 + i * 0.55, 1.49 + i * 0.55, 48);
      const ringMat = new THREE.MeshBasicMaterial({
        color: new THREE.Color(this.currentPalette.darkPrimary),
        side: THREE.DoubleSide,
        transparent: true,
        opacity: 0,
        depthWrite: false,
      });
      const ringMesh = new THREE.Mesh(ringGeo, ringMat);
      this.rigGroup.add(ringMesh);
      this.pulseRings.push(ringMesh);
    }

    this.root.add(this.createArchitecturalFloor());
    this.refreshTargetColors();
  }

  private createArchitecturalFloor(): THREE.Group {
    const group = new THREE.Group();
    group.position.set(0, -3.2, 0);

    const gridPoints: number[] = [];
    const extent = 7.5;
    const step = 1.5;
    for (let i = -extent; i <= extent; i += step) {
      gridPoints.push(-extent, 0, i, extent, 0, i);
      gridPoints.push(i, 0, -extent, i, 0, extent);
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.Float32BufferAttribute(gridPoints, 3));
    const mat = new THREE.LineBasicMaterial({
      color: new THREE.Color(this.currentPalette.lightPrimary),
      transparent: true,
      opacity: 0.09,
    });
    this.floorLines = new THREE.LineSegments(geo, mat);
    group.add(this.floorLines);
    return group;
  }

  private refreshTargetColors(): void {
    const p = this.currentPalette;
    if (this.themeMode === "light") {
      this.targetPrimaryColor.set(p.darkPrimary);
      this.targetSpecularColor.set(p.darkSpecular);
      this.targetTopBlockColor.set(p.lightPrimary);
      this.targetMidBlockColor.set("#1C2B24");
      this.targetBotBlockColor.set("#121D18");
      if (this.floorLines) {
        this.floorLines.material.color.set(p.lightPrimary);
        this.floorLines.material.opacity = 0.11;
      }
    } else {
      this.targetPrimaryColor.set(p.darkPrimary);
      this.targetSpecularColor.set(p.darkSpecular);
      this.targetTopBlockColor.set(p.lightPrimary);
      this.targetMidBlockColor.set("#141F1A");
      this.targetBotBlockColor.set("#0D1411");
      if (this.floorLines) {
        this.floorLines.material.color.set(p.darkPrimary);
        this.floorLines.material.opacity = 0.06;
      }
    }
  }

  public setThemeAndAccent(mode: ThemeMode, key: AccentKey): void {
    this.themeMode = mode;
    this.currentPalette = ACCENT_PALETTES[key];
    this.refreshTargetColors();
  }

  public setHoveredVoxel(index: number | null): void {
    this.hoveredIndex = index;
  }

  public setSelectedVoxel(index: number | null): void {
    this.selectedIndex = index;
  }

  public getVoxelWorldPosition(index: number, out = new THREE.Vector3()): THREE.Vector3 {
    const unit = this.units[index];
    if (!unit) return out.set(0, 0, 0);
    return unit.group.getWorldPosition(out);
  }

  /**
   * Updates all 27 voxels based on scroll progress & pointer position.
   * Returns `true` if any transform/color is still actively interpolating so the
   * demand-driven render loop can sleep at 0% CPU once settled.
   */
  public update(
    dt: number,
    scroll: number,
    scrollVelocity: number,
    pointerX: number,
    pointerY: number,
    isMobile: boolean,
    reducedMotion: boolean,
  ): boolean {
    const lerpFactor = reducedMotion ? 1.0 : 1 - Math.exp(-dt * 10.5);
    const colorLerp = reducedMotion ? 1.0 : 1 - Math.exp(-dt * 8.0);

    this.livePrimaryColor.lerp(this.targetPrimaryColor, colorLerp);
    this.liveSpecularColor.lerp(this.targetSpecularColor, colorLerp);
    this.liveTopBlockColor.lerp(this.targetTopBlockColor, colorLerp);
    this.liveMidBlockColor.lerp(this.targetMidBlockColor, colorLerp);
    this.liveBotBlockColor.lerp(this.targetBotBlockColor, colorLerp);

    const w0to1 = smoothstep(0.14, 0.26, scroll);
    const w1to2 = smoothstep(0.37, 0.48, scroll);
    const w2to3 = smoothstep(0.59, 0.70, scroll);
    const w3to4 = smoothstep(0.80, 0.90, scroll);

    const w0 = 1 - w0to1;
    const w1 = w0to1 * (1 - w1to2);
    const w2 = w1to2 * (1 - w2to3);
    const w3 = w2to3 * (1 - w3to4);
    const w4 = w3to4;

    const desktopSpread = isMobile ? 0 : 1.72;
    const targetRigX =
      (w0 * 1.05 + w1 * -0.95 + w2 * 0.98 + w3 * -0.92 + w4 * 1.02) * desktopSpread +
      pointerX * 0.22;
    const targetRigY =
      (isMobile
        ? w0 * 0.55 + w1 * 0.35 + w2 * 0.45 + w3 * 0.4 + w4 * 0.65
        : w0 * 0.05 + w1 * -0.05 + w2 * 0.08 + w3 * 0.0 + w4 * 0.12) +
      pointerY * 0.18;
    const targetRigZ = w0 * 0.0 + w1 * -0.35 + w2 * -0.5 + w3 * 0.35 + w4 * 0.15;

    let maxDelta =
      Math.abs(this.rigGroup.position.x - targetRigX) +
      Math.abs(this.rigGroup.position.y - targetRigY) +
      Math.abs(this.rigGroup.position.z - targetRigZ);

    this.rigGroup.position.x = THREE.MathUtils.lerp(
      this.rigGroup.position.x,
      targetRigX,
      lerpFactor,
    );
    this.rigGroup.position.y = THREE.MathUtils.lerp(
      this.rigGroup.position.y,
      targetRigY,
      lerpFactor,
    );
    this.rigGroup.position.z = THREE.MathUtils.lerp(
      this.rigGroup.position.z,
      targetRigZ,
      lerpFactor,
    );

    const targetRotX =
      w0 * 0.48 +
      w1 * 0.34 +
      w2 * 0.24 +
      w3 * 0.06 +
      w4 * 0.46 -
      pointerY * 0.18 +
      Math.max(-0.22, Math.min(0.22, scrollVelocity * 0.07));

    const targetRotY =
      w0 * -0.76 +
      w1 * (-0.76 + scroll * 1.6) +
      w2 * 0.52 +
      w3 * 0.0 +
      w4 * -0.78 +
      pointerX * 0.26;

    const targetRotZ = w2 * -0.08 + w3 * (scroll - 0.74) * 1.2;

    maxDelta +=
      Math.abs(this.rigGroup.rotation.x - targetRotX) +
      Math.abs(this.rigGroup.rotation.y - targetRotY) +
      Math.abs(this.rigGroup.rotation.z - targetRotZ);

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

    const stage2Progress = smoothstep(0.42, 0.64, scroll);
    const stage3Progress = smoothstep(0.64, 0.84, scroll);
    const velocityGapBoost = Math.min(0.22, Math.abs(scrollVelocity) * 0.05);

    const p0 = this.scratchP0;
    const p1 = this.scratchP1;
    const p2 = this.scratchP2;
    const p3 = this.scratchP3;
    const p4 = this.scratchP4;
    const euler = this.scratchEuler;

    for (let i = 0; i < this.units.length; i++) {
      const unit = this.units[i];
      const [cx, cy, cz] = unit.spec.coords;

      // Stage 0: Assembled 3x3x3 Monolith
      const unlockBreath = smoothstep(0.05, 0.2, scroll) * 0.16;
      const s0Spacing = 0.75 + unlockBreath + velocityGapBoost;
      p0.set(cx * s0Spacing, cy * s0Spacing, cz * s0Spacing);

      // Stage 1: Stratified Isolation
      const layerTwist = cy * 0.36;
      const cosT = Math.cos(layerTwist);
      const sinT = Math.sin(layerTwist);
      const rx = cx * cosT - cz * sinT;
      const rz = cx * sinT + cz * cosT;
      const s1Radial = 1.02 + (cy === 0 ? 0.14 : 0);
      p1.set(rx * s1Radial, cy * 1.9, rz * s1Radial);

      // Stage 2: Community Content & Dependency Flow Lattice
      const col = cy;
      const rowIdx = (cz + 1) * 3 + (cx + 1);
      const ringAngle = (rowIdx / 9) * Math.PI * 2 + stage2Progress * Math.PI * 0.8;
      const radius = rowIdx === 4 ? 0.0 : 1.28;
      p2.set(-col * 1.85, Math.cos(ringAngle) * radius, Math.sin(ringAngle) * radius);

      // Stage 3: Direct Server Launch Aperture
      if (i < 16) {
        const a = (i / 16) * Math.PI * 2 + stage3Progress * 0.9;
        p3.set(Math.cos(a) * 2.25, Math.sin(a) * 2.25, -0.55 + (i % 2) * 0.28);
      } else if (i < 24) {
        const a = ((i - 16) / 8) * Math.PI * 2 - stage3Progress * 1.3;
        p3.set(Math.cos(a) * 1.25, Math.sin(a) * 1.25, 0.45);
      } else {
        const coreIdx = i - 24;
        p3.set(0, 0, -1.35 + coreIdx * 1.15);
      }

      // Stage 4: Reconverged Core
      const s4Spacing = 0.75 + velocityGapBoost * 0.5;
      p4.set(cx * s4Spacing, cy * s4Spacing, cz * s4Spacing);

      unit.targetPos
        .set(0, 0, 0)
        .addScaledVector(p0, w0)
        .addScaledVector(p1, w1)
        .addScaledVector(p2, w2)
        .addScaledVector(p3, w3)
        .addScaledVector(p4, w4);

      const isHovered = this.hoveredIndex === i;
      const isSelected = this.selectedIndex === i;
      const targetHover = isHovered ? 1 : 0;
      const targetSelected = isSelected ? 1 : 0;

      maxDelta +=
        Math.abs(unit.hoverAmount - targetHover) +
        Math.abs(unit.selectedAmount - targetSelected);

      unit.hoverAmount = THREE.MathUtils.lerp(unit.hoverAmount, targetHover, lerpFactor);
      unit.selectedAmount = THREE.MathUtils.lerp(
        unit.selectedAmount,
        targetSelected,
        lerpFactor,
      );

      const lift = unit.hoverAmount * 0.24 + unit.selectedAmount * 0.32;
      if (lift > 0.001) {
        this.scratchRadial.copy(unit.targetPos);
        if (this.scratchRadial.lengthSq() < 0.01) this.scratchRadial.set(0, 1, 1);
        this.scratchRadial.normalize();
        unit.targetPos.addScaledVector(this.scratchRadial, lift);
      }

      maxDelta += unit.currentPos.distanceToSquared(unit.targetPos);
      unit.currentPos.lerp(unit.targetPos, lerpFactor);
      unit.group.position.copy(unit.currentPos);

      euler.set(
        w2 * (rowIdx * 0.2) + w3 * (i * 0.15),
        w1 * layerTwist + w2 * (stage2Progress * 0.8),
        w3 * (i < 16 ? (i / 16) * Math.PI * 2 : 0),
      );
      unit.targetQuat.setFromEuler(euler);
      unit.currentQuat.slerp(unit.targetQuat, lerpFactor);
      unit.group.quaternion.copy(unit.currentQuat);

      const pipelineWave =
        w2 * Math.max(0, 1 - Math.abs(stage2Progress * 27 - i) * 0.28);
      const apertureGlow = w3 * (i >= 24 ? 1.0 : 0.45);
      const strataGlow = w1 * (cy === 0 ? 0.65 : 0.3);
      const rMarkGlow = (w0 + w4) * (unit.spec.isRightRMark ? 0.92 : 0.15);

      const highlightBoost =
        Math.max(rMarkGlow, strataGlow, pipelineWave, apertureGlow) +
        unit.hoverAmount * 0.85 +
        unit.selectedAmount * 1.1;

      if (cy === 1) {
        unit.bodyMesh.material.color.copy(this.liveTopBlockColor);
      } else if (cy === 0) {
        unit.bodyMesh.material.color.copy(this.liveMidBlockColor);
      } else {
        unit.bodyMesh.material.color.copy(this.liveBotBlockColor);
      }

      unit.bodyMesh.material.emissive.copy(this.livePrimaryColor);
      unit.bodyMesh.material.emissiveIntensity = 0.04 + highlightBoost * 0.18;

      unit.corePlateMesh.material.color.copy(
        unit.spec.isRightRMark ? this.liveSpecularColor : this.livePrimaryColor,
      );
      unit.corePlateMesh.material.opacity = Math.min(1, 0.2 + highlightBoost * 0.8);

      unit.edgeLines.material.color.copy(
        unit.hoverAmount > 0.2 || unit.selectedAmount > 0.2 || unit.spec.isRightRMark
          ? this.liveSpecularColor
          : this.livePrimaryColor,
      );
      unit.edgeLines.material.opacity = Math.min(0.95, 0.22 + highlightBoost * 0.62);
    }

    // Update conduit lines only when visible (Stage 1, 2, 3)
    const conduitVisibility = Math.max(w1 * 0.55, w2 * 0.65, w3 * 0.3);
    this.conduitLines.material.color.copy(this.livePrimaryColor);
    this.conduitLines.material.opacity = conduitVisibility;
    this.pulsePoints.material.color.copy(this.liveSpecularColor);
    this.pulsePoints.material.opacity = Math.min(1, conduitVisibility * 1.4);

    if (conduitVisibility > 0.005) {
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

        // Scroll-scrubbed pulse position along each conduit
        for (let p = 0; p < 2; p++) {
          const phase = (scroll * 4.5 + c * 0.13 + p * 0.5) % 1;
          const pIdx = (c * 2 + p) * 3;
          this.pulseCoords[pIdx] = THREE.MathUtils.lerp(posA.x, posB.x, phase);
          this.pulseCoords[pIdx + 1] = THREE.MathUtils.lerp(posA.y, posB.y, phase);
          this.pulseCoords[pIdx + 2] = THREE.MathUtils.lerp(posA.z, posB.z, phase);
        }
      }
      posAttr.needsUpdate = true;
      pulseAttr.needsUpdate = true;
    }

    // Stage 3 Launch Aperture Rings (scroll-driven)
    for (let r = 0; r < this.pulseRings.length; r++) {
      const ring = this.pulseRings[r];
      ring.material.color.copy(this.livePrimaryColor);
      if (w3 > 0.01) {
        const cycle = (scroll * 5.0 + r * 0.33) % 1;
        ring.position.z = -1.8 + cycle * 3.8;
        const scale = 0.65 + cycle * 0.75;
        ring.scale.set(scale, scale, 1);
        ring.material.opacity = w3 * Math.sin(cycle * Math.PI) * 0.45;
      } else {
        ring.material.opacity = 0;
      }
    }

    return maxDelta > 0.00025;
  }
}
