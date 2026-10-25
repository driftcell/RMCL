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
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(page.subtitle()),
                ),
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
