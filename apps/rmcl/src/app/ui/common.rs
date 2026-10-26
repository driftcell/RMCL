use gpui_kit::{
    assets::IconName,
    component::{ActiveTheme, Icon, Sizable, Size, StyledExt, h_flex, tag::Tag, v_flex},
    prelude::FluentBuilder,
    *,
};

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
) -> impl IntoElement {
    v_flex()
        .w_full()
        .items_center()
        .justify_center()
        .gap_3()
        .py_12()
        .child(
            div()
                .size(px(56.))
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
        .child(action)
}

pub fn type_tag(version_type: Option<&str>) -> Tag {
    let tag = match version_type {
        Some("release") => Tag::success(),
        Some("snapshot") => Tag::info(),
        Some("old_beta") | Some("old_alpha") => Tag::secondary(),
        _ => Tag::new().outline(),
    };
    tag.child(super::super::version_type_label(version_type))
        .small()
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
