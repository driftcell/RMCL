//! CPU-rendered 3D character previews for official Java skins.
//!
//! Minecraft characters are made of six textured cuboids. Rendering them with
//! a tiny software rasterizer keeps the preview independent of GPUI and avoids
//! pulling a general-purpose 3D engine into the launcher.

use image::{Rgba, RgbaImage};
use rmcl_auth::SkinVariant;

use crate::skins::SkinTexture;

const SUPERSAMPLE: u32 = 2;
const CAMERA_DISTANCE: f32 = 70.0;
const MODEL_CENTER_Y: f32 = -2.0;
const FOCAL_SCALE: f32 = 1.45;
/// Keep the widest part of the character inside narrow canvases as well.
const WIDTH_FOCAL_SCALE: f32 = 2.8;
const MAX_PREVIEW_DIMENSION: u32 = 2048;
const KEY_LIGHT: Vec3 = Vec3::new(-0.45, 0.75, 0.6);
const FILL_LIGHT: Vec3 = Vec3::new(0.5, -0.15, -0.45);

/// Renders a modern or legacy Java skin into a transparent RGBA image.
#[derive(Debug, Clone, Copy)]
pub struct SkinPreviewRenderer {
    width: u32,
    height: u32,
    supersample: u32,
}

impl SkinPreviewRenderer {
    /// Creates a renderer for the given output size in physical pixels.
    pub fn new(width: u32, height: u32) -> Self {
        Self::with_supersample(width, height, SUPERSAMPLE)
    }

    /// Creates a renderer that trades anti-aliasing for speed. Use it for
    /// interactive frames and [`Self::new`] for the settled view.
    pub fn fast(width: u32, height: u32) -> Self {
        Self::with_supersample(width, height, 1)
    }

    fn with_supersample(width: u32, height: u32, supersample: u32) -> Self {
        Self {
            width: width.clamp(1, MAX_PREVIEW_DIMENSION),
            height: height.clamp(1, MAX_PREVIEW_DIMENSION),
            supersample: supersample.clamp(1, SUPERSAMPLE),
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Renders one frame.
    ///
    /// `yaw` and `pitch` are radians. `time` drives a subtle idle pose; pass
    /// zero for a static frame.
    pub fn render(
        &self,
        texture: &SkinTexture,
        variant: SkinVariant,
        yaw: f32,
        pitch: f32,
        time: f32,
    ) -> RgbaImage {
        let width = self.width.saturating_mul(self.supersample);
        let height = self.height.saturating_mul(self.supersample);
        let focal = (height as f32 * FOCAL_SCALE).min(width as f32 * WIDTH_FOCAL_SCALE);
        let yaw = if yaw.is_finite() { yaw } else { 0.0 };
        let pitch = if pitch.is_finite() {
            pitch.clamp(-0.6, 0.6)
        } else {
            0.0
        };
        let time = if time.is_finite() { time.max(0.0) } else { 0.0 };

        let mut canvas = Canvas::new(width, height);
        let faces = build_parts(texture, variant, time)
            .into_iter()
            .flat_map(|part| part.faces())
            .filter_map(|face| project_face(face, yaw, pitch, width, height, focal))
            .collect::<Vec<_>>();

        if let Some(feet) =
            project_point(Vec3::new(0.0, -18.0, 0.0), yaw, pitch, width, height, focal)
        {
            let max_radius = (width as f32 * 0.45).max(1.0);
            let radius_x = (focal / feet.depth * 7.5).clamp(1.0, max_radius);
            let radius_y = (radius_x * 0.22).max(1.0);
            canvas.draw_shadow(feet.screen, radius_x, radius_y);
        }

        for face in faces.iter().filter(|face| !face.overlay) {
            canvas.raster_face(face, texture, true, true);
        }

        let mut overlays = faces.iter().filter(|face| face.overlay).collect::<Vec<_>>();
        overlays.sort_by(|left, right| {
            right
                .depth
                .partial_cmp(&left.depth)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for face in overlays {
            canvas.raster_face(face, texture, false, false);
        }

        downsample(&mut canvas, self.width, self.height, self.supersample)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3 {
    const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }

    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }

    fn mul(self, scalar: f32) -> Self {
        Self::new(self.x * scalar, self.y * scalar, self.z * scalar)
    }

    fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    fn normalize(self) -> Self {
        let length = self.dot(self).sqrt();
        if length <= f32::EPSILON {
            self
        } else {
            self.mul(1.0 / length)
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Vec2 {
    x: f32,
    y: f32,
}

impl Vec2 {
    const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FaceKind {
    Top,
    Bottom,
    Left,
    Front,
    Right,
    Back,
}

#[derive(Debug, Clone, Copy)]
struct Cuboid {
    center: Vec3,
    size: Vec3,
    uv_size: Vec3,
    uv: Vec2,
    scale: f32,
    mirror: bool,
}

impl Cuboid {
    fn faces(self, overlay: bool) -> [Face; 6] {
        [
            self.face(FaceKind::Top, overlay),
            self.face(FaceKind::Bottom, overlay),
            self.face(FaceKind::Left, overlay),
            self.face(FaceKind::Front, overlay),
            self.face(FaceKind::Right, overlay),
            self.face(FaceKind::Back, overlay),
        ]
    }

    fn face(self, kind: FaceKind, overlay: bool) -> Face {
        let half = self.size.mul(0.5);
        let x0 = self.center.x - half.x;
        let x1 = self.center.x + half.x;
        let y0 = self.center.y - half.y;
        let y1 = self.center.y + half.y;
        let z0 = self.center.z - half.z;
        let z1 = self.center.z + half.z;

        let (vertices, normal) = match kind {
            FaceKind::Top => (
                [
                    Vec3::new(x0, y1, z1),
                    Vec3::new(x1, y1, z1),
                    Vec3::new(x1, y1, z0),
                    Vec3::new(x0, y1, z0),
                ],
                Vec3::new(0.0, 1.0, 0.0),
            ),
            FaceKind::Bottom => (
                [
                    Vec3::new(x0, y0, z0),
                    Vec3::new(x1, y0, z0),
                    Vec3::new(x1, y0, z1),
                    Vec3::new(x0, y0, z1),
                ],
                Vec3::new(0.0, -1.0, 0.0),
            ),
            FaceKind::Left => (
                [
                    Vec3::new(x0, y0, z0),
                    Vec3::new(x0, y0, z1),
                    Vec3::new(x0, y1, z1),
                    Vec3::new(x0, y1, z0),
                ],
                Vec3::new(-1.0, 0.0, 0.0),
            ),
            FaceKind::Front => (
                [
                    Vec3::new(x0, y0, z1),
                    Vec3::new(x1, y0, z1),
                    Vec3::new(x1, y1, z1),
                    Vec3::new(x0, y1, z1),
                ],
                Vec3::new(0.0, 0.0, 1.0),
            ),
            FaceKind::Right => (
                [
                    Vec3::new(x1, y0, z1),
                    Vec3::new(x1, y0, z0),
                    Vec3::new(x1, y1, z0),
                    Vec3::new(x1, y1, z1),
                ],
                Vec3::new(1.0, 0.0, 0.0),
            ),
            FaceKind::Back => (
                [
                    Vec3::new(x1, y0, z0),
                    Vec3::new(x0, y0, z0),
                    Vec3::new(x0, y1, z0),
                    Vec3::new(x1, y1, z0),
                ],
                Vec3::new(0.0, 0.0, -1.0),
            ),
        };

        let (width, height, depth) = (self.uv_size.x, self.uv_size.y, self.uv_size.z);
        // A mirrored legacy limb swaps its side strips as well as flipping U.
        let uv_kind = if self.mirror {
            match kind {
                FaceKind::Left => FaceKind::Right,
                FaceKind::Right => FaceKind::Left,
                other => other,
            }
        } else {
            kind
        };
        let (u0, v0, u1, v1) = match uv_kind {
            FaceKind::Top => (
                self.uv.x + depth,
                self.uv.y,
                self.uv.x + width + depth,
                self.uv.y + depth,
            ),
            FaceKind::Bottom => (
                self.uv.x + width + depth,
                self.uv.y,
                self.uv.x + width * 2.0 + depth,
                self.uv.y + depth,
            ),
            FaceKind::Left => (
                self.uv.x,
                self.uv.y + depth,
                self.uv.x + depth,
                self.uv.y + depth + height,
            ),
            FaceKind::Front => (
                self.uv.x + depth,
                self.uv.y + depth,
                self.uv.x + width + depth,
                self.uv.y + depth + height,
            ),
            FaceKind::Right => (
                self.uv.x + width + depth,
                self.uv.y + depth,
                self.uv.x + width + depth * 2.0,
                self.uv.y + depth + height,
            ),
            FaceKind::Back => (
                self.uv.x + width + depth * 2.0,
                self.uv.y + depth,
                self.uv.x + width * 2.0 + depth * 2.0,
                self.uv.y + depth + height,
            ),
        };

        let mut uvs = [
            Vec2::new(u0, v1),
            Vec2::new(u1, v1),
            Vec2::new(u1, v0),
            Vec2::new(u0, v0),
        ]
        .map(|uv| Vec2::new(uv.x * self.scale, uv.y * self.scale));
        if self.mirror {
            for uv in &mut uvs {
                uv.x = (u0 + u1) * self.scale - uv.x;
            }
        }

        Face {
            vertices,
            uvs,
            normal,
            overlay,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Face {
    vertices: [Vec3; 4],
    uvs: [Vec2; 4],
    normal: Vec3,
    overlay: bool,
}

#[derive(Debug, Clone, Copy)]
struct Part {
    pivot: Vec3,
    rotation: Vec3,
    base: Cuboid,
    overlay: Option<Cuboid>,
}

impl Part {
    fn faces(self) -> Vec<Face> {
        let mut faces = Vec::with_capacity(if self.overlay.is_some() { 12 } else { 6 });
        for mut face in self.base.faces(false) {
            face.vertices = face
                .vertices
                .map(|vertex| rotate_around(vertex, self.pivot, self.rotation));
            face.normal = rotate_vector(face.normal, self.rotation);
            faces.push(face);
        }
        if let Some(overlay) = self.overlay {
            for mut face in overlay.faces(true) {
                face.vertices = face
                    .vertices
                    .map(|vertex| rotate_around(vertex, self.pivot, self.rotation));
                face.normal = rotate_vector(face.normal, self.rotation);
                faces.push(face);
            }
        }
        faces
    }
}

fn build_parts(texture: &SkinTexture, variant: SkinVariant, time: f32) -> Vec<Part> {
    let scale = texture.scale() as f32;
    let legacy = texture.is_legacy();
    let slim = variant == SkinVariant::Slim;
    let arm_width = if slim { 3.0 } else { 4.0 };
    let arm_x = 4.0 + arm_width / 2.0;
    let swing = if time > 0.0 {
        (time * 1.6).sin() * 0.035
    } else {
        0.0
    };
    let head_yaw = if time > 0.0 {
        (time * 0.7).sin() * 0.05
    } else {
        0.0
    };

    let body_uv = Vec2::new(16.0, 16.0);
    let right_arm_uv = Vec2::new(40.0, 16.0);
    let left_arm_uv = Vec2::new(32.0, 48.0);
    let right_leg_uv = Vec2::new(0.0, 16.0);
    let left_leg_uv = Vec2::new(16.0, 48.0);

    let mut parts = Vec::with_capacity(6);
    parts.push(Part {
        pivot: Vec3::new(0.0, 6.0, 0.0),
        rotation: Vec3::new(0.0, head_yaw, 0.0),
        base: Cuboid {
            center: Vec3::new(0.0, 10.0, 0.0),
            size: Vec3::new(8.0, 8.0, 8.0),
            uv_size: Vec3::new(8.0, 8.0, 8.0),
            uv: Vec2::new(0.0, 0.0),
            scale,
            mirror: false,
        },
        overlay: Some(Cuboid {
            center: Vec3::new(0.0, 10.0, 0.0),
            size: Vec3::new(9.0, 9.0, 9.0),
            uv_size: Vec3::new(8.0, 8.0, 8.0),
            uv: Vec2::new(32.0, 0.0),
            scale,
            mirror: false,
        }),
    });

    parts.push(Part {
        pivot: Vec3::new(0.0, 0.0, 0.0),
        rotation: Vec3::new(0.0, 0.0, 0.0),
        base: Cuboid {
            center: Vec3::new(0.0, 0.0, 0.0),
            size: Vec3::new(8.0, 12.0, 4.0),
            uv_size: Vec3::new(8.0, 12.0, 4.0),
            uv: body_uv,
            scale,
            mirror: false,
        },
        overlay: (!legacy).then_some(Cuboid {
            center: Vec3::new(0.0, 0.0, 0.0),
            size: Vec3::new(8.5, 12.5, 4.5),
            uv_size: Vec3::new(8.0, 12.0, 4.0),
            uv: Vec2::new(16.0, 32.0),
            scale,
            mirror: false,
        }),
    });

    parts.push(Part {
        pivot: Vec3::new(-arm_x, 6.0, 0.0),
        rotation: Vec3::new(swing, 0.0, 0.0),
        base: Cuboid {
            center: Vec3::new(-arm_x, 0.0, 0.0),
            size: Vec3::new(arm_width, 12.0, 4.0),
            uv_size: Vec3::new(arm_width, 12.0, 4.0),
            uv: right_arm_uv,
            scale,
            mirror: false,
        },
        overlay: (!legacy).then_some(Cuboid {
            center: Vec3::new(-arm_x, 0.0, 0.0),
            size: Vec3::new(arm_width + 0.5, 12.5, 4.5),
            uv_size: Vec3::new(arm_width, 12.0, 4.0),
            uv: Vec2::new(40.0, 32.0),
            scale,
            mirror: false,
        }),
    });

    parts.push(Part {
        pivot: Vec3::new(arm_x, 6.0, 0.0),
        rotation: Vec3::new(-swing, 0.0, 0.0),
        base: Cuboid {
            center: Vec3::new(arm_x, 0.0, 0.0),
            size: Vec3::new(arm_width, 12.0, 4.0),
            uv_size: Vec3::new(arm_width, 12.0, 4.0),
            uv: if legacy { right_arm_uv } else { left_arm_uv },
            scale,
            mirror: legacy,
        },
        overlay: (!legacy).then_some(Cuboid {
            center: Vec3::new(arm_x, 0.0, 0.0),
            size: Vec3::new(arm_width + 0.5, 12.5, 4.5),
            uv_size: Vec3::new(arm_width, 12.0, 4.0),
            uv: Vec2::new(48.0, 48.0),
            scale,
            mirror: false,
        }),
    });

    parts.push(Part {
        pivot: Vec3::new(-2.0, -6.0, 0.0),
        rotation: Vec3::new(0.0, 0.0, 0.0),
        base: Cuboid {
            center: Vec3::new(-2.0, -12.0, 0.0),
            size: Vec3::new(4.0, 12.0, 4.0),
            uv_size: Vec3::new(4.0, 12.0, 4.0),
            uv: right_leg_uv,
            scale,
            mirror: false,
        },
        overlay: (!legacy).then_some(Cuboid {
            center: Vec3::new(-2.0, -12.0, 0.0),
            size: Vec3::new(4.5, 12.5, 4.5),
            uv_size: Vec3::new(4.0, 12.0, 4.0),
            uv: Vec2::new(0.0, 32.0),
            scale,
            mirror: false,
        }),
    });

    parts.push(Part {
        pivot: Vec3::new(2.0, -6.0, 0.0),
        rotation: Vec3::new(0.0, 0.0, 0.0),
        base: Cuboid {
            center: Vec3::new(2.0, -12.0, 0.0),
            size: Vec3::new(4.0, 12.0, 4.0),
            uv_size: Vec3::new(4.0, 12.0, 4.0),
            uv: if legacy { right_leg_uv } else { left_leg_uv },
            scale,
            mirror: legacy,
        },
        overlay: (!legacy).then_some(Cuboid {
            center: Vec3::new(2.0, -12.0, 0.0),
            size: Vec3::new(4.5, 12.5, 4.5),
            uv_size: Vec3::new(4.0, 12.0, 4.0),
            uv: Vec2::new(0.0, 48.0),
            scale,
            mirror: false,
        }),
    });

    parts
}

fn rotate_x(vector: Vec3, angle: f32) -> Vec3 {
    let (sin, cos) = angle.sin_cos();
    Vec3::new(
        vector.x,
        vector.y * cos - vector.z * sin,
        vector.y * sin + vector.z * cos,
    )
}

fn rotate_y(vector: Vec3, angle: f32) -> Vec3 {
    let (sin, cos) = angle.sin_cos();
    Vec3::new(
        vector.x * cos + vector.z * sin,
        vector.y,
        -vector.x * sin + vector.z * cos,
    )
}

fn rotate_z(vector: Vec3, angle: f32) -> Vec3 {
    let (sin, cos) = angle.sin_cos();
    Vec3::new(
        vector.x * cos - vector.y * sin,
        vector.x * sin + vector.y * cos,
        vector.z,
    )
}

fn rotate_vector(vector: Vec3, rotation: Vec3) -> Vec3 {
    rotate_x(
        rotate_y(rotate_z(vector, rotation.z), rotation.y),
        rotation.x,
    )
}

fn rotate_around(point: Vec3, pivot: Vec3, rotation: Vec3) -> Vec3 {
    rotate_vector(point.sub(pivot), rotation).add(pivot)
}

fn view_point(point: Vec3, yaw: f32, pitch: f32) -> Vec3 {
    let centered = Vec3::new(point.x, point.y - MODEL_CENTER_Y, point.z);
    rotate_x(rotate_y(centered, yaw), pitch)
}

#[derive(Debug, Clone, Copy)]
struct ProjectedPoint {
    screen: Vec2,
    depth: f32,
}

fn project_point(
    point: Vec3,
    yaw: f32,
    pitch: f32,
    width: u32,
    height: u32,
    focal: f32,
) -> Option<ProjectedPoint> {
    let view = view_point(point, yaw, pitch);
    let depth = CAMERA_DISTANCE - view.z;
    if depth <= 0.1 {
        return None;
    }
    Some(ProjectedPoint {
        screen: Vec2::new(
            width as f32 * 0.5 + view.x * focal / depth,
            height as f32 * 0.5 - view.y * focal / depth,
        ),
        depth,
    })
}

#[derive(Debug, Clone, Copy)]
struct ProjectedVertex {
    screen: Vec2,
    uv: Vec2,
    inv_depth: f32,
}

#[derive(Debug, Clone, Copy)]
struct ProjectedFace {
    vertices: [ProjectedVertex; 4],
    shade: f32,
    overlay: bool,
    depth: f32,
}

fn project_face(
    face: Face,
    yaw: f32,
    pitch: f32,
    width: u32,
    height: u32,
    focal: f32,
) -> Option<ProjectedFace> {
    let mut vertices = [ProjectedVertex {
        screen: Vec2::default(),
        uv: Vec2::default(),
        inv_depth: 0.0,
    }; 4];
    let mut depth_sum = 0.0;
    for (index, (position, uv)) in face.vertices.iter().zip(face.uvs).enumerate() {
        let view = view_point(*position, yaw, pitch);
        let depth = CAMERA_DISTANCE - view.z;
        if depth <= 0.1 {
            return None;
        }
        vertices[index] = ProjectedVertex {
            screen: Vec2::new(
                width as f32 * 0.5 + view.x * focal / depth,
                height as f32 * 0.5 - view.y * focal / depth,
            ),
            uv,
            inv_depth: 1.0 / depth,
        };
        depth_sum += depth;
    }
    let normal = rotate_x(rotate_y(face.normal, yaw), pitch);
    let key = normal.dot(KEY_LIGHT.normalize()).max(0.0);
    let fill = normal.dot(FILL_LIGHT.normalize()).max(0.0);
    let shade = (0.62 + key * 0.38 + fill * 0.08).clamp(0.45, 1.1);
    Some(ProjectedFace {
        vertices,
        shade,
        overlay: face.overlay,
        depth: depth_sum * 0.25,
    })
}

struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    depth: Vec<f32>,
}

impl Canvas {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; width as usize * height as usize * 4],
            depth: vec![f32::INFINITY; width as usize * height as usize],
        }
    }

    fn draw_shadow(&mut self, center: Vec2, radius_x: f32, radius_y: f32) {
        let min_x = (center.x - radius_x).floor().max(0.0) as u32;
        let max_x = (center.x + radius_x)
            .ceil()
            .min(self.width as f32 - 1.0)
            .max(0.0) as u32;
        let min_y = (center.y - radius_y).floor().max(0.0) as u32;
        let max_y = (center.y + radius_y)
            .ceil()
            .min(self.height as f32 - 1.0)
            .max(0.0) as u32;
        if min_x > max_x || min_y > max_y {
            return;
        }
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let dx = (x as f32 + 0.5 - center.x) / radius_x;
                let dy = (y as f32 + 0.5 - center.y) / radius_y;
                let distance = dx * dx + dy * dy;
                if distance >= 1.0 {
                    continue;
                }
                let alpha = (1.0 - distance).powi(2) * 0.22;
                self.blend(x as usize, y as usize, [0.0, 0.0, 0.0], alpha);
            }
        }
    }

    fn raster_face(
        &mut self,
        face: &ProjectedFace,
        texture: &SkinTexture,
        opaque: bool,
        write_depth: bool,
    ) {
        self.raster_triangle(
            &face.vertices[0],
            &face.vertices[1],
            &face.vertices[2],
            face,
            texture,
            opaque,
            write_depth,
            Some((1, true)),
        );
        self.raster_triangle(
            &face.vertices[0],
            &face.vertices[2],
            &face.vertices[3],
            face,
            texture,
            opaque,
            write_depth,
            Some((1, false)),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn raster_triangle(
        &mut self,
        a: &ProjectedVertex,
        b: &ProjectedVertex,
        c: &ProjectedVertex,
        face: &ProjectedFace,
        texture: &SkinTexture,
        opaque: bool,
        write_depth: bool,
        shared_edge: Option<(usize, bool)>,
    ) {
        let area = edge(a.screen, b.screen, c.screen);
        if area.abs() <= f32::EPSILON {
            return;
        }
        let inv_area = 1.0 / area;
        let min_x = a.screen.x.min(b.screen.x).min(c.screen.x).floor().max(0.0) as u32;
        let max_x = a
            .screen
            .x
            .max(b.screen.x)
            .max(c.screen.x)
            .ceil()
            .min(self.width as f32 - 1.0)
            .max(0.0) as u32;
        let min_y = a.screen.y.min(b.screen.y).min(c.screen.y).floor().max(0.0) as u32;
        let max_y = a
            .screen
            .y
            .max(b.screen.y)
            .max(c.screen.y)
            .ceil()
            .min(self.height as f32 - 1.0)
            .max(0.0) as u32;
        if min_x > max_x || min_y > max_y {
            return;
        }

        let image = texture.image();
        let texture_width = image.width() as f32;
        let texture_height = image.height() as f32;

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let point = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let w0 = edge(b.screen, c.screen, point) * inv_area;
                let w1 = edge(c.screen, a.screen, point) * inv_area;
                let w2 = edge(a.screen, b.screen, point) * inv_area;
                let weights = [w0, w1, w2];
                if weights.iter().any(|weight| *weight < -0.0001) {
                    continue;
                }
                if let Some((index, include_zero)) = shared_edge {
                    let weight = weights[index];
                    if weight < 0.0 || (!include_zero && weight == 0.0) {
                        continue;
                    }
                }
                let inv_depth = w0 * a.inv_depth + w1 * b.inv_depth + w2 * c.inv_depth;
                if inv_depth <= 0.0 {
                    continue;
                }
                let depth = 1.0 / inv_depth;
                let index = y as usize * self.width as usize + x as usize;
                if depth >= self.depth[index] {
                    continue;
                }
                let u = (w0 * a.uv.x * a.inv_depth
                    + w1 * b.uv.x * b.inv_depth
                    + w2 * c.uv.x * c.inv_depth)
                    / inv_depth;
                let v = (w0 * a.uv.y * a.inv_depth
                    + w1 * b.uv.y * b.inv_depth
                    + w2 * c.uv.y * c.inv_depth)
                    / inv_depth;
                if !u.is_finite() || !v.is_finite() {
                    continue;
                }
                let texel = image.get_pixel(
                    u.floor().clamp(0.0, texture_width - 1.0) as u32,
                    v.floor().clamp(0.0, texture_height - 1.0) as u32,
                );
                let alpha = if opaque { 1.0 } else { texel[3] as f32 / 255.0 };
                if alpha <= 0.0 {
                    continue;
                }
                let shade = face.shade;
                let color = [
                    (texel[0] as f32 * shade).clamp(0.0, 255.0),
                    (texel[1] as f32 * shade).clamp(0.0, 255.0),
                    (texel[2] as f32 * shade).clamp(0.0, 255.0),
                ];
                if write_depth {
                    self.depth[index] = depth;
                }
                if alpha >= 1.0 {
                    let index = index * 4;
                    self.pixels[index] = color[0].round() as u8;
                    self.pixels[index + 1] = color[1].round() as u8;
                    self.pixels[index + 2] = color[2].round() as u8;
                    self.pixels[index + 3] = 255;
                } else {
                    self.blend(x as usize, y as usize, color, alpha);
                }
            }
        }
    }

    fn blend(&mut self, x: usize, y: usize, color: [f32; 3], alpha: f32) {
        let index = (y * self.width as usize + x) * 4;
        let alpha = alpha.clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }
        let destination_alpha = self.pixels[index + 3] as f32 / 255.0;
        let output_alpha = alpha + destination_alpha * (1.0 - alpha);
        if output_alpha <= 0.0 {
            return;
        }
        if destination_alpha <= 0.0 {
            self.pixels[index] = color[0].round().clamp(0.0, 255.0) as u8;
            self.pixels[index + 1] = color[1].round().clamp(0.0, 255.0) as u8;
            self.pixels[index + 2] = color[2].round().clamp(0.0, 255.0) as u8;
            self.pixels[index + 3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
            return;
        }
        for (channel, source) in color.iter().enumerate() {
            let destination = self.pixels[index + channel] as f32;
            let value =
                (*source * alpha + destination * destination_alpha * (1.0 - alpha)) / output_alpha;
            self.pixels[index + channel] = value.round().clamp(0.0, 255.0) as u8;
        }
        self.pixels[index + 3] = (output_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
    }
}

fn edge(a: Vec2, b: Vec2, point: Vec2) -> f32 {
    (point.x - a.x) * (b.y - a.y) - (point.y - a.y) * (b.x - a.x)
}

fn downsample(canvas: &mut Canvas, width: u32, height: u32, supersample: u32) -> RgbaImage {
    if supersample == 1 {
        // Canvas pixels already use the output resolution and straight alpha.
        return RgbaImage::from_raw(width, height, std::mem::take(&mut canvas.pixels))
            .expect("canvas dimensions match its pixel buffer");
    }
    let mut output = RgbaImage::new(width, height);
    let sample_count = (supersample * supersample) as f32;
    for y in 0..height {
        for x in 0..width {
            let mut red = 0.0;
            let mut green = 0.0;
            let mut blue = 0.0;
            let mut alpha = 0.0;
            for sample_y in 0..supersample {
                for sample_x in 0..supersample {
                    let source_x = x * supersample + sample_x;
                    let source_y = y * supersample + sample_y;
                    let index = (source_y as usize * canvas.width as usize + source_x as usize) * 4;
                    let sample_alpha = canvas.pixels[index + 3] as f32 / 255.0;
                    red += canvas.pixels[index] as f32 * sample_alpha;
                    green += canvas.pixels[index + 1] as f32 * sample_alpha;
                    blue += canvas.pixels[index + 2] as f32 * sample_alpha;
                    alpha += sample_alpha;
                }
            }
            let output_alpha = alpha / sample_count;
            if output_alpha <= 0.0 {
                continue;
            }
            output.put_pixel(
                x,
                y,
                Rgba([
                    (red / alpha).round().clamp(0.0, 255.0) as u8,
                    (green / alpha).round().clamp(0.0, 255.0) as u8,
                    (blue / alpha).round().clamp(0.0, 255.0) as u8,
                    (output_alpha * 255.0).round().clamp(0.0, 255.0) as u8,
                ]),
            );
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skin_texture(width: u32, height: u32, color: [u8; 4]) -> SkinTexture {
        SkinTexture::from_rgba(RgbaImage::from_pixel(width, height, Rgba(color))).unwrap()
    }

    fn alpha_bounds(image: &RgbaImage) -> (u32, u32, u32, u32) {
        let mut min_x = u32::MAX;
        let mut min_y = u32::MAX;
        let mut max_x = 0;
        let mut max_y = 0;
        let mut found = false;
        for (x, y, pixel) in image.enumerate_pixels() {
            if pixel[3] > 128 {
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
        if !found {
            (0, 0, 0, 0)
        } else {
            (min_x, min_y, max_x, max_y)
        }
    }

    #[test]
    fn renders_requested_size_and_visible_character() {
        let texture = skin_texture(64, 64, [180, 120, 80, 255]);
        let renderer = SkinPreviewRenderer::new(256, 320);

        let image = renderer.render(&texture, SkinVariant::Classic, 0.35, 0.1, 0.0);

        assert_eq!(image.dimensions(), (256, 320));
        let opaque = image.pixels().filter(|pixel| pixel[3] > 0).count();
        assert!(opaque > 1000, "expected a visible character, got {opaque}");
    }

    #[test]
    fn slim_model_is_narrower_than_classic() {
        let texture = skin_texture(64, 64, [80, 160, 200, 255]);
        let renderer = SkinPreviewRenderer::new(320, 360);

        let classic = renderer.render(&texture, SkinVariant::Classic, 0.0, 0.0, 0.0);
        let slim = renderer.render(&texture, SkinVariant::Slim, 0.0, 0.0, 0.0);
        let (classic_left, _, classic_right, _) = alpha_bounds(&classic);
        let (slim_left, _, slim_right, _) = alpha_bounds(&slim);
        let classic_width = classic_right - classic_left;
        let slim_width = slim_right - slim_left;

        assert!(
            slim_width < classic_width,
            "slim {slim_width} should be narrower than classic {classic_width}"
        );
    }

    #[test]
    fn legacy_texture_renders_mirrored_limbs() {
        let texture = skin_texture(64, 32, [200, 80, 120, 255]);
        let renderer = SkinPreviewRenderer::new(240, 320);

        let image = renderer.render(&texture, SkinVariant::Classic, 0.0, 0.0, 0.0);

        assert_eq!(image.dimensions(), (240, 320));
        let opaque = image.pixels().filter(|pixel| pixel[3] > 0).count();
        assert!(opaque > 500, "expected legacy limbs to render");
    }

    #[test]
    fn head_front_color_appears_in_the_output() {
        let mut image = RgbaImage::from_pixel(64, 64, Rgba([0, 0, 0, 0]));
        for y in 8..16 {
            for x in 8..16 {
                image.put_pixel(x, y, Rgba([240, 20, 20, 255]));
            }
        }
        let texture = SkinTexture::from_rgba(image).unwrap();
        let renderer = SkinPreviewRenderer::new(200, 260);

        let rendered = renderer.render(&texture, SkinVariant::Classic, 0.0, 0.0, 0.0);

        let red = rendered
            .pixels()
            .filter(|pixel| pixel[3] > 200 && pixel[0] > 150 && pixel[1] < 100)
            .count();
        assert!(red > 0, "head face color should be visible");
    }

    #[test]
    fn idle_pose_is_deterministic_for_the_same_time() {
        let texture = skin_texture(64, 64, [120, 180, 90, 255]);
        let renderer = SkinPreviewRenderer::new(180, 240);

        let first = renderer.render(&texture, SkinVariant::Classic, 0.3, 0.1, 1.25);
        let second = renderer.render(&texture, SkinVariant::Classic, 0.3, 0.1, 1.25);

        assert_eq!(first.into_raw(), second.into_raw());
    }

    #[test]
    fn renders_tiny_canvases_without_panicking() {
        let texture = skin_texture(64, 64, [20, 120, 200, 255]);
        let image =
            SkinPreviewRenderer::new(1, 1).render(&texture, SkinVariant::Classic, 0.0, 0.0, 0.0);

        assert_eq!(image.dimensions(), (1, 1));
    }

    #[test]
    fn narrow_canvas_keeps_the_character_inside() {
        let texture = skin_texture(64, 64, [80, 160, 200, 255]);
        let image = SkinPreviewRenderer::new(120, 400).render(
            &texture,
            SkinVariant::Classic,
            0.0,
            0.0,
            0.0,
        );
        let (left, _, right, _) = alpha_bounds(&image);

        assert!(left > 0, "left edge was clipped");
        assert!(right < 119, "right edge was clipped");
    }

    #[test]
    fn mirroring_swaps_legacy_side_strip_uvs() {
        let cuboid = Cuboid {
            center: Vec3::default(),
            size: Vec3::new(4.0, 12.0, 4.0),
            uv_size: Vec3::new(4.0, 12.0, 4.0),
            uv: Vec2::new(40.0, 16.0),
            scale: 1.0,
            mirror: true,
        };

        let left = cuboid.face(FaceKind::Left, false);
        let right = cuboid.face(FaceKind::Right, false);
        let left_u = left.uvs.iter().map(|uv| uv.x).collect::<Vec<_>>();
        let right_u = right.uvs.iter().map(|uv| uv.x).collect::<Vec<_>>();

        assert!(left_u.iter().all(|u| (48.0..=52.0).contains(u)));
        assert!(right_u.iter().all(|u| (40.0..=44.0).contains(u)));
    }
}
