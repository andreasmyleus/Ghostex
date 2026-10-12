//! Catch me up: the button at the composer's top right and the card above the composer. The core
//! owns when either shows and what the card holds (`packages/gx-chat-core/src/composer/catch_up.rs`).

use super::side_question::{answering_placeholder, side_button};
use super::{appearance::ChatAppearance, state::NativeChatView, transcript::text};
use crate::app::native_chat::cursor::ChatCursor as _;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Stateful, StatefulInteractiveElement as _, Styled as _, Window, div, px, svg,
};
use serde_json::json;

impl NativeChatView {
    /// CDXC:CatchMeUp 2026-10-11 DECISION:
    /// User: the Catch me up button sits in the composer itself, at its top right. It shares the field's row (beside the field's first line) instead of overlapping it, and stays out of the one-line collapsed and the maximized composer.
    pub(super) fn with_catch_up_button(
        &self,
        editor: Stateful<Div>,
        shown: bool,
        p: &ChatAppearance,
        cx: &Context<Self>,
    ) -> AnyElement {
        let catch_up = &self.snapshot["catchUp"];
        if !shown || catch_up["enabled"] != true {
            return editor.into_any_element();
        }
        let s = p.scale;
        let open = catch_up["open"] == true;
        let loading = catch_up["loading"] == true;
        let button = div()
            .id("catch-me-up")
            .role(gpui::Role::Button)
            .aria_label("Catch me up")
            .aria_expanded(open)
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(5.0 * s))
            .px(px(8.0 * s))
            .h(px(24.0 * s))
            .rounded_full()
            .border_1()
            .border_color(p.border)
            .text_size(px(12.0 * s))
            .text_color(if open { p.control_primary } else { p.primary })
            .when(open, |this| this.bg(p.border))
            .chat_cursor_pointer()
            .hover(|style| style.bg(p.border))
            .child(
                svg()
                    .path("titlebar/sparkles.svg")
                    .size(px(13.0 * s))
                    .text_color(if open { p.control_primary } else { p.primary }),
            )
            .child(if loading {
                "Catching up…"
            } else {
                "Catch me up"
            })
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new(
                    "Summarize where this session stands, from its recent messages",
                )
                .build(window, cx)
            })
            .on_click(cx.listener(|this, _, _, cx| this.invoke(json!({"type":"catchUp"}), cx)));
        div()
            .flex()
            .items_start()
            .gap(px(8.0 * s))
            .w_full()
            .min_w_0()
            .child(editor.flex_1().min_w_0())
            .child(button)
            .into_any_element()
    }

    /// The card above the composer: loading, the summary drawn with the transcript's marked
    /// sections, or why there is none, with Refresh, Copy and Close.
    pub(super) fn render_catch_up(
        &self,
        p: &ChatAppearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let catch_up = &self.snapshot["catchUp"];
        if catch_up["open"] != true {
            return None;
        }
        let s = p.scale;
        let loading = catch_up["loading"] == true;
        let header = div()
            .flex()
            .items_center()
            .gap(px(8.0 * s))
            .child(
                svg()
                    .path("titlebar/sparkles.svg")
                    .size(px(14.0 * s))
                    .text_color(p.primary),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(p.foreground)
                    .font_weight(FontWeight::MEDIUM)
                    .child("Catch me up"),
            )
            .when(loading, |this| {
                this.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(12.0 * s))
                        .text_color(p.muted)
                        .child("Reading the session…"),
                )
            })
            .into_any_element();
        let mut body = Vec::new();
        if loading {
            body.push(answering_placeholder(p));
        } else if let Some(error) = catch_up["error"].as_str() {
            body.push(
                div()
                    .text_size(px(13.0 * s))
                    .text_color(gpui::rgb(0xef9999))
                    .child(error.to_string())
                    .into_any_element(),
            );
        } else {
            body.push(self.side_answer_body(
                "catch-up".to_string(),
                text(catch_up, "markdown"),
                &catch_up["references"],
                Some(window.viewport_size().height.as_f32() * 0.55),
                p,
                cx,
            ));
            if let Some(caption) = catch_up["caption"].as_str() {
                body.push(
                    div()
                        .text_size(px(12.0 * s))
                        .text_color(p.muted)
                        .child(caption.to_string())
                        .into_any_element(),
                );
            }
        }
        let copy_text = text(catch_up, "copyText");
        let actions = vec![
            div().flex_1().into_any_element(),
            side_button(
                "catch-up-refresh",
                Some("titlebar/refresh.svg"),
                "Refresh",
                None,
                loading,
                Some("Write a fresh summary from the latest messages"),
                p,
                cx.listener(|this, _, _, cx| this.invoke(json!({"type":"catchUpRefresh"}), cx)),
            ),
            side_button(
                "catch-up-copy",
                Some("titlebar/copy.svg"),
                "Copy",
                None,
                copy_text.is_empty(),
                None,
                p,
                cx.listener(move |_, _, _, cx| {
                    crate::app::helpers::gpui_copy_to_clipboard(
                        gpui::ClipboardItem::new_string(copy_text.clone()),
                        cx,
                    );
                }),
            ),
            side_button(
                "catch-up-close",
                Some("titlebar/x.svg"),
                "Close",
                None,
                false,
                None,
                p,
                cx.listener(|this, _, _, cx| this.invoke(json!({"type":"catchUpClose"}), cx)),
            ),
        ];
        Some(self.status_card_with_header(header, body, actions, p))
    }
}
