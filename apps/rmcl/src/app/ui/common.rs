use std::{path::Path, sync::Arc};

use gpui_kit::{
    assets::IconName,
    component::{ActiveTheme, Icon, Sizable, Size, StyledExt, h_flex, tag::Tag, v_flex},
    prelude::FluentBuilder,
    *,
};
use image::{Frame, ImageBuffer, Rgba as ImageRgba, RgbaImage};
use rmcl_store::home_directory;

use super::super::Page;

pub fn page_header(page: Page, actions: impl IntoElement, cx: &App) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_start()
        .justify_between()
        .gap_4()
        .child(
            v_flex()
                .gap_1()
                .child(div().text_2xl().font_semibold().child(page.title()))
                .when(!page.subtitle().is_empty(), |this| {
                    this.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(page.subtitle()),
                    )
                }),
        )
        .child(h_flex().gap_2().child(actions))
}

/// Converts decoded RGBA pixels into the BGRA buffer GPUI expects.
pub fn render_rgba_image(image: RgbaImage) -> Arc<RenderImage> {
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

pub fn card(cx: &App) -> Div {
    v_flex()
        .w_full()
        .items_stretch()
        .rounded(cx.theme().radius_lg)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().group_box)
        .text_color(cx.theme().group_box_foreground)
        .p_5()
        .gap_4()
}

pub fn section_title(title: &str, subtitle: &str, cx: &App) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(div().text_base().font_semibold().child(title.to_owned()))
        .when(!subtitle.is_empty(), |this| {
            this.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(subtitle.to_owned()),
            )
        })
}

pub fn empty_state(
    icon: IconName,
    title: &str,
    description: &str,
    action: impl IntoElement,
    cx: &App,
) -> AnyElement {
    empty_state_layout(
        icon,
        title,
        description,
        Some(action.into_any_element()),
        cx,
    )
}

/// An empty state whose primary action already lives in the surrounding toolbar.
pub fn empty_state_message(icon: IconName, title: &str, description: &str, cx: &App) -> AnyElement {
    empty_state_layout(icon, title, description, None, cx)
}

fn empty_state_layout(
    icon: IconName,
    title: &str,
    description: &str,
    action: Option<AnyElement>,
    cx: &App,
) -> AnyElement {
    v_flex()
        .w_full()
        .items_center()
        .justify_center()
        .gap_3()
        .py_12()
        .child(
            div()
                .size(rems(3.5))
                .rounded_full()
                .bg(cx.theme().primary.opacity(0.12))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(icon)
                        .with_size(Size::Large)
                        .text_color(cx.theme().primary),
                ),
        )
        .child(div().text_lg().font_semibold().child(title.to_owned()))
        .child(
            div()
                .max_w_96()
                .text_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(description.to_owned()),
        )
        .when_some(action, |this, action| this.child(action))
        .into_any_element()
}

pub fn type_tag(version_type: Option<&str>) -> Tag {
    Tag::new()
        .outline()
        .child(super::super::version_type_label(version_type))
        .small()
}

pub fn server_tile(path: Option<&std::path::Path>, size: Rems, cx: &App) -> AnyElement {
    if let Some(path) = path {
        return div()
            .size(size)
            .flex_shrink_0()
            .rounded(cx.theme().radius)
            .overflow_hidden()
            .bg(cx.theme().primary.opacity(0.15))
            .child(
                img(path.to_path_buf())
                    .size_full()
                    .object_fit(ObjectFit::Cover),
            )
            .into_any_element();
    }
    div()
        .size(size)
        .flex_shrink_0()
        .rounded(cx.theme().radius)
        .bg(cx.theme().primary.opacity(0.12))
        .flex()
        .items_center()
        .justify_center()
        .child(Icon::new(IconName::Network).text_color(cx.theme().primary))
        .into_any_element()
}

/// Formats a user-facing path with the home directory shortened to `~`.
///
/// Only paths below the user's home directory are abbreviated; user-editable
/// path inputs keep the full path they operate on.
pub fn display_path(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    match home_directory() {
        Some(home) => abbreviate_home(path, home),
        None => path.display().to_string(),
    }
}

fn abbreviate_home(path: impl AsRef<Path>, home: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    let home = home.as_ref();
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        Err(_) => path.display().to_string(),
    }
}

pub fn mono(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .font_family(cx.theme().mono_font_family.clone())
        .text_size(cx.theme().mono_font_size)
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

pub fn field_label(label: &str) -> impl IntoElement {
    div().text_sm().font_medium().child(label.to_owned())
}

/// Secondary prose that explains a control. Keep `mono` for paths and data.
pub fn help(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

pub fn error_banner(title: &str, message: &str, cx: &App) -> impl IntoElement {
    banner(IconName::CircleAlert, cx.theme().danger, title, message, cx)
}

pub fn warning_banner(title: &str, message: &str, cx: &App) -> impl IntoElement {
    banner(
        IconName::TriangleAlert,
        cx.theme().warning,
        title,
        message,
        cx,
    )
}

fn banner(icon: IconName, color: Hsla, title: &str, message: &str, cx: &App) -> impl IntoElement {
    h_flex()
        .w_full()
        .gap_3()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(color.opacity(0.35))
        .bg(color.opacity(0.08))
        .p_3()
        .child(Icon::new(icon).small().text_color(color))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_medium()
                        .text_color(color)
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(message.to_owned()),
                ),
        )
}

pub fn format_size(bytes: u64) -> String {
    const KIB: f64 = 1024.;
    const MIB: f64 = KIB * 1024.;
    const GIB: f64 = MIB * 1024.;
    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.1} GiB", bytes / GIB)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes / KIB)
    } else {
        format!("{bytes:.0} B")
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::abbreviate_home;

    #[test]
    fn display_path_shortens_the_home_directory() {
        let home = Path::new("home").join("steve");
        let path = home.join(".minecraft").join("versions");

        assert_eq!(
            abbreviate_home(&path, &home),
            format!(
                "~{}.minecraft{}versions",
                std::path::MAIN_SEPARATOR,
                std::path::MAIN_SEPARATOR
            )
        );
        assert_eq!(abbreviate_home(&home, &home), "~");
    }

    #[test]
    fn display_path_keeps_paths_outside_home() {
        let home = Path::new("home").join("steve");
        let other_user = Path::new("home").join("alex").join(".minecraft");

        assert_eq!(
            abbreviate_home(&other_user, &home),
            other_user.display().to_string()
        );
    }
}
