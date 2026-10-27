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
    base::Selectable,
    component::{
        ActiveTheme, Icon, Sizable, Size,
        button::{Button, ButtonVariants},
        h_flex,
    },
    prelude::FluentBuilder,
    *,
};
use image::{Frame, ImageBuffer, Rgba as ImageRgba, RgbaImage};
use rmcl_app::{SkinPreviewRenderer, SkinTexture};
use rmcl_auth::SkinVariant;

const DEFAULT_PREVIEW_WIDTH: u32 = 520;
const DEFAULT_PREVIEW_HEIGHT: u32 = 620;
const AUTO_ROTATE_SPEED: f32 = 0.22;
const FRAME_INTERVAL: Duration = Duration::from_millis(33);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationMode {
    Static,
    Auto,
    Dragging { resume_auto: bool },
}

pub struct RenderedPreview {
    image: Arc<RenderImage>,
}

pub struct SkinViewer {
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
    cache: Option<RenderedPreview>,
    width: u32,
    height: u32,
}

impl SkinViewer {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
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
            cache: None,
            width: DEFAULT_PREVIEW_WIDTH,
            height: DEFAULT_PREVIEW_HEIGHT,
        }
    }

    /// Sets the raster size of the stage. This is a physical/raster boundary,
    /// so it is one of the few places that intentionally uses pixels.
    pub fn set_size(&mut self, width: u32, height: u32, cx: &mut Context<Self>) {
        let width = width.max(1);
        let height = height.max(1);
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        self.dirty = true;
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
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        if self.texture.is_none() && self.source_key.is_none() {
            return;
        }
        self.texture = None;
        self.source_key = None;
        self.dirty = true;
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

    pub fn toggle_auto_rotation(&mut self, cx: &mut Context<Self>) {
        self.rotation = if matches!(self.rotation, RotationMode::Auto) {
            RotationMode::Static
        } else {
            RotationMode::Auto
        };
        self.last_frame = None;
        cx.notify();
    }

    fn set_front(&mut self, cx: &mut Context<Self>) {
        self.yaw = 0.0;
        self.pitch = 0.08;
        self.rotation = RotationMode::Static;
        self.last_frame = None;
        self.dirty = true;
        cx.notify();
    }

    fn set_back(&mut self, cx: &mut Context<Self>) {
        self.yaw = std::f32::consts::PI;
        self.pitch = 0.08;
        self.rotation = RotationMode::Static;
        self.last_frame = None;
        self.dirty = true;
        cx.notify();
    }

    fn begin_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let resume_auto = matches!(self.rotation, RotationMode::Auto);
        self.rotation = RotationMode::Dragging { resume_auto };
        self.dragging_from = Some(position);
        self.last_frame = None;
        cx.notify();
    }

    fn drag_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !matches!(self.rotation, RotationMode::Dragging { .. }) {
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
        let RotationMode::Dragging { resume_auto } = self.rotation else {
            return;
        };
        self.rotation = if resume_auto {
            RotationMode::Auto
        } else {
            RotationMode::Static
        };
        self.dragging_from = None;
        self.last_frame = None;
        self.dirty = true;
        cx.notify();
    }

    fn ensure_preview(&mut self, window: &mut Window) {
        let Some(texture) = self.texture.as_ref() else {
            self.cache = None;
            return;
        };
        if !self.dirty && self.cache.is_some() {
            return;
        }
        if matches!(self.rotation, RotationMode::Auto)
            && self
                .last_render
                .is_some_and(|last| last.elapsed() < FRAME_INTERVAL)
        {
            return;
        }
        let renderer = SkinPreviewRenderer::new(self.width, self.height);
        let image = renderer.render(texture, self.variant, self.yaw, self.pitch, 0.0);
        if let Some(previous) = self.cache.take() {
            let _ = window.drop_image(previous.image);
        }
        self.cache = Some(RenderedPreview {
            image: render_image(image),
        });
        self.dirty = false;
        self.last_render = Some(Instant::now());
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
        self.ensure_preview(window);

        let tint = self.ambient_color(cx);
        let theme = cx.theme();
        let has_texture = self.texture.is_some();
        let auto_rotation = matches!(self.rotation, RotationMode::Auto);

        div()
            .relative()
            .w(px(self.width as f32))
            .h(px(self.height as f32))
            .flex_shrink_0()
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
            .child(
                h_flex()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_4()
                    .justify_center()
                    .gap_2()
                    .child(
                        Button::new("skin-view-front")
                            .label("正面")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.set_front(cx))),
                    )
                    .child(
                        Button::new("skin-view-back")
                            .label("背面")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.set_back(cx))),
                    )
                    .child(
                        Button::new("skin-view-auto")
                            .label(if auto_rotation {
                                "自动旋转中"
                            } else {
                                "自动旋转"
                            })
                            .outline()
                            .selected(auto_rotation)
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_auto_rotation(cx))),
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
