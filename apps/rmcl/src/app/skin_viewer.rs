//! The 3D skin preview entity.
//!
//! Keeping this in its own entity means the animation only repaints the stage
//! instead of rebuilding the whole skin page on every frame.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gpui_kit::{
    assets::IconName,
    base::ElementExt,
    component::{ActiveTheme, Icon, Sizable, Size},
    prelude::FluentBuilder,
    *,
};
use image::{Frame, ImageBuffer, Rgba as ImageRgba, RgbaImage};
use rmcl_app::{SkinPreviewRenderer, SkinTexture};
use rmcl_auth::SkinVariant;

const DEFAULT_PREVIEW_WIDTH: u32 = 432;
const DEFAULT_PREVIEW_HEIGHT: u32 = 620;
const AUTO_ROTATE_SPEED: f32 = 0.22;
const FRAME_INTERVAL: Duration = Duration::from_millis(33);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationMode {
    Auto,
    Dragging,
}

pub struct RenderedPreview {
    image: Arc<RenderImage>,
}

pub struct SkinViewer {
    runtime: tokio::runtime::Handle,
    texture: Option<SkinTexture>,
    variant: SkinVariant,
    source_key: Option<String>,
    yaw: f32,
    pitch: f32,
    rotation: RotationMode,
    dragging_from: Option<Point<Pixels>>,
    last_frame: Option<Instant>,
    last_render: Option<Instant>,
    dirty: bool,
    pending_render: Option<u64>,
    next_render: u64,
    cache: Option<RenderedPreview>,
    layout_width: u32,
    width: u32,
    height: u32,
    _render_task: Option<Task<()>>,
}

impl SkinViewer {
    pub fn new(runtime: tokio::runtime::Handle, _cx: &mut Context<Self>) -> Self {
        Self {
            runtime,
            texture: None,
            variant: SkinVariant::Classic,
            source_key: None,
            yaw: -0.35,
            pitch: 0.10,
            rotation: RotationMode::Auto,
            dragging_from: None,
            last_frame: None,
            last_render: None,
            dirty: true,
            pending_render: None,
            next_render: 0,
            cache: None,
            layout_width: DEFAULT_PREVIEW_WIDTH,
            width: DEFAULT_PREVIEW_WIDTH,
            height: DEFAULT_PREVIEW_HEIGHT,
            _render_task: None,
        }
    }

    /// Sets the width of the preview column.
    pub fn set_width(&mut self, width: u32, cx: &mut Context<Self>) {
        let width = width.max(1);
        if self.layout_width == width {
            return;
        }
        self.layout_width = width;
        self.dirty = true;
        self.invalidate_render();
        cx.notify();
    }

    /// Applies the height measured during prepaint. This is a physical/raster
    /// boundary, so it intentionally works in pixels.
    fn resize(&mut self, width: u32, height: u32, cx: &mut Context<Self>) {
        let width = width.max(1);
        let height = height.max(1);
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        self.dirty = true;
        self.invalidate_render();
        cx.notify();
    }

    pub fn set_skin(
        &mut self,
        source_key: impl Into<String>,
        texture: SkinTexture,
        variant: SkinVariant,
        reset_view: bool,
        cx: &mut Context<Self>,
    ) {
        let source_key = source_key.into();
        if self.source_key.as_deref() == Some(source_key.as_str())
            && self.variant == variant
            && self.texture.is_some()
        {
            return;
        }
        self.source_key = Some(source_key);
        self.texture = Some(texture);
        self.variant = variant;
        if reset_view {
            self.yaw = -0.35;
            self.pitch = 0.10;
        }
        self.dirty = true;
        self.invalidate_render();
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        if self.texture.is_none() && self.source_key.is_none() {
            return;
        }
        self.texture = None;
        self.source_key = None;
        self.dirty = true;
        self.invalidate_render();
        cx.notify();
    }

    pub fn reset_view(&mut self, cx: &mut Context<Self>) {
        self.yaw = -0.35;
        self.pitch = 0.10;
        self.rotation = RotationMode::Auto;
        self.last_frame = None;
        self.dirty = true;
        cx.notify();
    }

    fn begin_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.rotation = RotationMode::Dragging;
        self.dragging_from = Some(position);
        self.last_frame = None;
        self.dirty = true;
        cx.notify();
    }

    fn drag_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !matches!(self.rotation, RotationMode::Dragging) {
            return;
        }
        let Some(previous) = self.dragging_from.replace(position) else {
            return;
        };
        let delta_x = f32::from(position.x - previous.x);
        let delta_y = f32::from(position.y - previous.y);
        self.yaw += delta_x * 0.012;
        self.pitch = (self.pitch + delta_y * 0.006).clamp(-0.35, 0.45);
        self.dirty = true;
        cx.notify();
    }

    fn finish_drag(&mut self, cx: &mut Context<Self>) {
        if self.rotation != RotationMode::Dragging {
            return;
        }
        self.rotation = RotationMode::Auto;
        self.dragging_from = None;
        self.last_frame = None;
        self.dirty = true;
        cx.notify();
    }

    fn invalidate_render(&mut self) {
        self.pending_render = None;
        self._render_task = None;
    }

    /// Starts at most one background render. While it runs, newer input only
    /// marks the preview dirty; the finished frame is discarded in favor of the
    /// latest pose. This keeps dragging responsive even when rasterization is
    /// slower than the input rate.
    fn request_preview(&mut self, cx: &mut Context<Self>) {
        if self.texture.is_none() {
            return;
        }
        if self.pending_render.is_some() || !self.dirty {
            return;
        }
        if matches!(self.rotation, RotationMode::Auto)
            && self
                .last_render
                .is_some_and(|last| last.elapsed() < FRAME_INTERVAL)
        {
            return;
        }
        let Some(texture) = self.texture.clone() else {
            return;
        };
        let fast = matches!(self.rotation, RotationMode::Auto | RotationMode::Dragging);
        let width = self.width;
        let height = self.height;
        let variant = self.variant;
        let yaw = self.yaw;
        let pitch = self.pitch;

        self.next_render = self.next_render.wrapping_add(1);
        let request = self.next_render;
        self.pending_render = Some(request);
        self.dirty = false;

        let runtime = self.runtime.clone();
        self._render_task = Some(cx.spawn(async move |this, cx| {
            let image = runtime
                .spawn(async move {
                    let renderer = if fast {
                        SkinPreviewRenderer::fast(width, height)
                    } else {
                        SkinPreviewRenderer::new(width, height)
                    };
                    render_image(renderer.render(&texture, variant, yaw, pitch, 0.0))
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.finish_preview(request, image.ok(), window, cx);
            });
        }));
    }

    fn finish_preview(
        &mut self,
        request: u64,
        image: Option<Arc<RenderImage>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_render != Some(request) {
            return;
        }
        self.pending_render = None;
        self.last_render = Some(Instant::now());
        if let Some(image) = image {
            if let Some(previous) = self.cache.take() {
                let _ = window.drop_image(previous.image);
            }
            self.cache = Some(RenderedPreview { image });
        }
        cx.notify();
    }

    fn ambient_color(&self, cx: &App) -> Hsla {
        let Some(texture) = self.texture.as_ref() else {
            return cx.theme().primary;
        };
        let image = texture.image();
        let mut red = 0.0;
        let mut green = 0.0;
        let mut blue = 0.0;
        let mut count = 0.0;
        for pixel in image.pixels() {
            if pixel[3] < 200 {
                continue;
            }
            red += pixel[0] as f32;
            green += pixel[1] as f32;
            blue += pixel[2] as f32;
            count += 1.0;
        }
        if count <= 0.0 {
            return cx.theme().primary;
        }
        // Audited data color: the ambient light is derived from the skin
        // texture itself, not from product chrome.
        Hsla::from(gpui_kit::Rgba {
            r: red / count / 255.0,
            g: green / count / 255.0,
            b: blue / count / 255.0,
            a: 1.0,
        })
    }
}

impl Render for SkinViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.texture.is_none()
            && let Some(previous) = self.cache.take()
        {
            let _ = window.drop_image(previous.image);
        }
        if matches!(self.rotation, RotationMode::Auto) && self.texture.is_some() {
            let now = Instant::now();
            if let Some(previous) = self.last_frame {
                let delta = now.duration_since(previous).as_secs_f32().min(0.1);
                self.yaw += AUTO_ROTATE_SPEED * delta;
                self.dirty = true;
            }
            self.last_frame = Some(now);
            window.request_animation_frame();
        } else {
            self.last_frame = None;
        }
        self.request_preview(cx);

        let tint = self.ambient_color(cx);
        let theme = cx.theme();
        let has_texture = self.texture.is_some();
        let measured = cx.entity().downgrade();

        div()
            .relative()
            // NOTE: The preview is a raster surface. The page fixes the width;
            // the height fills the content row so both cards share an edge.
            .w(px(self.layout_width as f32))
            .h_full()
            .flex_shrink_0()
            .on_prepaint(move |bounds, _, cx| {
                let width = bounds.size.width.as_f32().round() as u32;
                let height = bounds.size.height.as_f32().round() as u32;
                let _ = measured.update(cx, |this, cx| this.resize(width, height, cx));
            })
            .rounded(theme.radius_lg)
            .overflow_hidden()
            .border_1()
            .border_color(theme.border)
            .bg(linear_gradient(
                180.0,
                linear_color_stop(tint.opacity(0.16), 0.0),
                linear_color_stop(theme.background, 1.0),
            ))
            .child(
                div()
                    .absolute()
                    .left_16()
                    .right_16()
                    .bottom_20()
                    .h(px(1.))
                    .bg(theme.foreground.opacity(0.08)),
            )
            .when_some(self.cache.as_ref(), |this, preview| {
                this.child(
                    img(preview.image.clone())
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Contain),
                )
            })
            .when(!has_texture, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::CircleUser)
                                .with_size(Size::Large)
                                .text_color(theme.muted_foreground),
                        ),
                )
            })
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.begin_drag(event.position, cx);
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        this.drag_to(event.position, cx);
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_drag(cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_drag(cx)),
                    ),
            )
    }
}

fn render_image(image: RgbaImage) -> Arc<RenderImage> {
    let (width, height) = image.dimensions();
    let mut bytes = image.into_raw();
    for pixel in bytes.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let Some(buffer) = ImageBuffer::from_raw(width, height, bytes) else {
        return Arc::new(RenderImage::new(vec![Frame::new(ImageBuffer::from_pixel(
            1,
            1,
            ImageRgba([0, 0, 0, 0]),
        ))]));
    };
    Arc::new(RenderImage::new(vec![Frame::new(buffer)]))
}
