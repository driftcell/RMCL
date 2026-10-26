import * as THREE from "three";
import { VoxelCoreSystem } from "./voxel-core";

export class VoxelPicker {
  private readonly raycaster = new THREE.Raycaster();
  private readonly ndc = new THREE.Vector2();

  public pick(
    clientX: number,
    clientY: number,
    camera: THREE.Camera,
    voxelCore: VoxelCoreSystem,
  ): number | null {
    const w = Math.max(1, window.innerWidth);
    const h = Math.max(1, window.innerHeight);
    this.ndc.x = (clientX / w) * 2 - 1;
    this.ndc.y = -(clientY / h) * 2 + 1;

    this.raycaster.setFromCamera(this.ndc, camera);
    const hits = this.raycaster.intersectObjects(voxelCore.pickableMeshes, false);
    if (hits.length === 0) return null;

    const first = hits[0].object;
    const idx = first.userData?.voxelIndex;
    return typeof idx === "number" ? idx : null;
  }
}
