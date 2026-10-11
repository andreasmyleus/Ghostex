use super::{appearance::ChatAppearance, state::NativeChatView, transcript::text};
use crate::app::native_chat::cursor::ChatCursor as _;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, px, svg,
};
use serde_json::{Value, json};

#[derive(Clone, Copy)]
enum ReplyAction {
    Copy,
    Annotate,
    SaveMarkdown,
}

impl NativeChatView {
    pub(super) fn reply_focus(
        &self,
        message: &Value,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Option<(gpui::FocusHandle, bool)> {
        if !self.has_reply_actions(message) {
            return None;
        }
        let state = window.use_keyed_state(
            format!("reply-focus:{}", text(message, "id")),
            cx,
            |window, cx| {
                let focus = cx.focus_handle();
                cx.on_focus_in(&focus, window, |_, _, cx| cx.notify())
                    .detach();
                cx.on_focus_out(&focus, window, |_, _, _, cx| cx.notify())
                    .detach();
                focus
            },
        );
        let focus = state.read(cx).clone();
        let focused = focus.contains_focused(window, cx);
        Some((focus, focused))
    }

    pub(super) fn has_reply_actions(&self, message: &Value) -> bool {
        message["role"] == "assistant"
            && message["actionContent"]["copyable"] == true
            && self.snapshot["finalIds"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id == &message["id"]))
    }

    /// A turn's final answer with no tool calls of its own: it is drawn flush with the chat column,
    /// without the reply dot.
    ///
    /// CDXC:SessionChat 2026-10-09 DECISION: User: the dot before the agent's last message shifted the whole message right, so "the card for the html mockup" was narrower than the chat box; remove the dot for the agent's final reply. Commentary between tool calls keeps its dot, and an answer with tool calls keeps its chevron.
    pub(super) fn is_flush_reply(&self, message: &Value) -> bool {
        message["role"] == "assistant"
            && !message["tools"]
                .as_array()
                .is_some_and(|tools| !tools.is_empty())
            && self.snapshot["finalIds"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id == &message["id"]))
    }

    pub(super) fn reply_marker(&self, p: &ChatAppearance) -> AnyElement {
        div()
            .relative()
            .w(px(16.0 * p.scale))
            .h(px(22.75 * p.scale))
            .ml(px(2.0 * p.scale))
            .flex_shrink_0()
            .child(
                div()
                    .absolute()
                    .left(px(6.0 * p.scale))
                    .top(px(9.5 * p.scale))
                    .size(px(4.0 * p.scale))
                    .rounded_full()
                    .bg(p.primary),
            )
            .into_any_element()
    }

    /// CDXC:SessionChat 2026-10-11 SEE-ALSO:
    /// A final reply's actions sit below it, starting at the prose column, as one small labelled toolbar (Copy · Annotate · Save) followed by the time it arrived, shown on hover or focus. This supersedes the 2026-09-19 row of three bare icons; the user's pick is recorded in gx-chat-core `transcript/work_summary.rs`, and the phone's `MessageActions.tsx` draws the same toolbar.
    pub(super) fn reply_actions(
        &self,
        message: &Value,
        focused: bool,
        p: &ChatAppearance,
        cx: &Context<Self>,
    ) -> AnyElement {
        let id = text(message, "id");
        let markdown = text(message, "copyText");
        let s = p.scale;
        let button = |action: ReplyAction,
                      label: &'static str,
                      short: &'static str,
                      icon: &'static str| {
            let click_markdown = markdown.clone();
            div()
                .id(format!("reply-action:{id}:{label}"))
                .group("native-chat-reply-action")
                .role(gpui::Role::Button)
                .aria_label(label)
                .tab_index(0)
                .h(px(24.0 * s))
                .px(px(8.0 * s))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(5.0 * s))
                .rounded(px(7.0 * s))
                .text_size(px(12.0 * s))
                .text_color(p.muted)
                .border(px(s))
                .border_color(gpui::transparent_black())
                .chat_cursor_pointer()
                .focus_visible(|mut style| {
                    style = style.border_color(p.ring);
                    style.box_shadow = Some(vec![gpui::BoxShadow {
                        color: p.ring.opacity(0.2),
                        offset: gpui::point(px(0.0), px(0.0)),
                        blur_radius: px(0.0),
                        spread_radius: px(3.0 * p.scale),
                        inset: false,
                    }]);
                    style
                })
                .hover(|style| style.bg(p.foreground.opacity(0.06)).text_color(p.foreground))
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(label).build(window, cx)
                })
                .child(
                    svg()
                        .path(icon)
                        .size(px(12.0 * s))
                        .text_color(p.muted)
                        .group_hover("native-chat-reply-action", |style| {
                            style.text_color(p.foreground)
                        }),
                )
                .child(short)
                .on_click(cx.listener(move |chat, _, _, cx| {
                    chat.perform_reply_action(action, &click_markdown, cx)
                }))
        };
        let separator = || {
            div()
                .w(px(1.0))
                .h(px(14.0 * s))
                .flex_shrink_0()
                .bg(p.foreground.opacity(0.1))
        };
        let can_annotate =
            self.config.app.is_some() && message["actionContent"]["canAnnotate"] == true;
        let can_save = message["actionContent"]["canSaveMarkdown"] == true;
        let buttons = div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .gap(px(2.0 * s))
            .p(px(2.0 * s))
            .rounded(px(9.0 * s))
            .border(px(s))
            .border_color(p.foreground.opacity(0.07))
            .bg(p.input.opacity(0.75))
            .child(button(
                ReplyAction::Copy,
                "Copy message",
                "Copy",
                "chat-actions/copy",
            ))
            .when(can_annotate, |actions| {
                actions.child(separator()).child(button(
                    ReplyAction::Annotate,
                    "Reply by Annotating",
                    "Annotate",
                    "chat-actions/annotate",
                ))
            })
            .when(can_save, |actions| {
                actions.child(separator()).child(button(
                    ReplyAction::SaveMarkdown,
                    "Save message to Markdown",
                    "Save",
                    "chat-actions/save",
                ))
            });
        // The prose column: the marker's 2px inset, its 16px slot, and the 6px gap before the text,
        // or the column's own edge for an answer drawn without the dot.
        let prose_column = if self.is_flush_reply(message) {
            0.0
        } else {
            24.0
        };
        message_actions_row(p, focused)
            .pl(px(prose_column * p.scale))
            .tab_group()
            .child(buttons)
            .when_some(message_time(&id, message, p), |row, time| row.child(time))
            .into_any_element()
    }

    /// CDXC:SavedPrompts 2026-09-06 DECISION:
    /// User: add Save prompt between Copy and Rewind on user messages, using the input box's stack-push icon.
    ///
    /// The prompt's own action row (React: `CopyFooter` on a user row). Rewind is offered only when the
    /// host can reach `/api/rewindSessionChat`, the session runs an agent whose rewind Ghostex
    /// drives, and the composer could send right now, because the daemon types the rewind into that
    /// same pane. Which prompt is a rewind target at all is decided in the core
    /// (`canRewind`, packages/gx-chat-core/src/transcript/presentation.rs).
    pub(super) fn user_actions(
        &self,
        message: &Value,
        p: &ChatAppearance,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let id = text(message, "id");
        let prompt = text(message, "copyText");
        // Nothing to copy, save, or rewind to: an empty prompt has no action row at all.
        if prompt.is_empty() {
            return None;
        }
        let saved = self.snapshot["savedPrompts"][id.as_str()]
            .as_str()
            .unwrap_or("");
        // A rewind types into the session's own pane, so a child transcript never offers one
        // (React's subagent viewer mounted its list without `rewindToMessage` for the same reason).
        let rewindable = !self.in_subagent
            && message["canRewind"] == true
            && self.snapshot["rewindAvailable"] == true
            && self.snapshot["rewindEnabled"] == true;
        // React passed `onSavePrompt` only when the host had a stash bridge, the same capability
        // behind the composer's Stash control, so a host without one offers Copy alone.
        let savable = self.snapshot["composerActions"]["stash"] == true;
        /*
        CDXC:SessionChat 2026-09-19 SEE-ALSO:
        `.ghostex-chat-message-actions` in packages/core-ui/styles/chat.css (deleted 2026-09-25)
        carried the user decision this mirrors: the prompt's actions sit right-aligned below the bubble, led by the time it
        was sent, in the order Rewind, Save prompt, Copy, so Copy lands at the bubble's edge.
        */
        let button = |key: &str, label: String, icon: &'static str, action: Value| {
            let icon_color = p.muted;
            div()
                .id(gpui::SharedString::from(format!("{key}:{id}")))
                .group("native-chat-user-action")
                .role(gpui::Role::Button)
                .aria_label(label.clone())
                .tab_index(0)
                .size(px(24.0 * p.scale))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0 * p.scale))
                .chat_cursor_pointer()
                .hover(|style| style.bg(p.border.opacity(0.4)))
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(label.clone()).build(window, cx)
                })
                .child(
                    svg()
                        .path(icon)
                        .size(px(12.0 * p.scale))
                        .text_color(icon_color)
                        .group_hover("native-chat-user-action", |style| {
                            style.text_color(p.foreground)
                        }),
                )
                .on_click(cx.listener(move |chat, _, _, cx| {
                    if action["type"] == "copyPrompt" {
                        crate::app::helpers::gpui_copy_to_clipboard(
                            gpui::ClipboardItem::new_string(text(&action, "text")),
                            cx,
                        );
                    } else {
                        chat.invoke(action.clone(), cx);
                    }
                }))
        };
        let buttons = div()
            .flex()
            .items_center()
            .gap(px(2.0 * p.scale))
            .when(rewindable, |buttons| {
                buttons.child(button(
                    "rewind",
                    "Rewind to here".into(),
                    "chat-actions/rewind",
                    json!({"type":"rewindOpen","messageId":id.clone(),"prompt":prompt.clone()}),
                ))
            })
            .when(savable, |buttons| {
                buttons.child(button(
                    "save-prompt",
                    match saved {
                        "saved" => "Prompt saved",
                        "saving" => "Saving prompt",
                        "error" => "Could not save prompt. Click to retry.",
                        _ => "Save prompt",
                    }
                    .into(),
                    if saved == "saved" {
                        "chat-actions/saved"
                    } else {
                        "chat-actions/savePrompt"
                    },
                    json!({"type":"savePrompt","messageId":id.clone(),"prompt":prompt.clone()}),
                ))
            })
            .child(button(
                "copy",
                "Copy message".into(),
                "chat-actions/copy",
                json!({"type":"copyPrompt","text":prompt.clone()}),
            ));
        Some(
            message_actions_row(p, false)
                .pr(px(4.0 * p.scale))
                .when_some(message_time(&id, message, p), |row, time| row.child(time))
                .child(buttons)
                .into_any_element(),
        )
    }

    fn perform_reply_action(
        &mut self,
        action: ReplyAction,
        markdown: &str,
        cx: &mut Context<Self>,
    ) {
        match action {
            ReplyAction::Copy => {
                crate::app::helpers::gpui_copy_to_clipboard(
                    gpui::ClipboardItem::new_string(markdown.to_owned()),
                    cx,
                );
            }
            ReplyAction::SaveMarkdown => {
                self.invoke(json!({"type":"markdownSaveOpen","markdown":markdown}), cx)
            }
            ReplyAction::Annotate => self.host("annotateReply", json!({"markdown":markdown}), cx),
        }
    }
}

/// The row under a message that holds its actions and time; hidden until the message is hovered
/// or, for a reply, holds keyboard focus (React: `.ghostex-chat-message-actions`).
///
/// CDXC:SessionChat 2026-09-19 WHY:
/// The row takes no negative margin and no `flex_shrink_0`. The list measures a row at min-content height, where taffy sizes a non-shrinking item with a negative main-axis margin as `basis + basis * margin`, which made a reply's row about 94px too short and drew the next message over it.
fn message_actions_row(p: &ChatAppearance, focused: bool) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.0 * p.scale))
        .opacity(if focused { 1.0 } else { 0.0 })
        .group_hover("native-chat-message", |style| style.opacity(1.0))
}

/// The label from packages/gx-chat-core/src/transcript/message_time.rs, with its long form
/// as the tooltip.
fn message_time(id: &str, message: &Value, p: &ChatAppearance) -> Option<AnyElement> {
    let label = text(&message["time"], "label");
    if label.is_empty() {
        return None;
    }
    let title = gpui::SharedString::from(text(&message["time"], "title"));
    Some(
        div()
            .id(gpui::SharedString::from(format!("message-time:{id}")))
            .flex_shrink_0()
            .whitespace_nowrap()
            .text_size(px(12.0 * p.scale))
            .text_color(p.muted)
            .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(title.clone()).build(window, cx)
            })
            .child(label)
            .into_any_element(),
    )
}
