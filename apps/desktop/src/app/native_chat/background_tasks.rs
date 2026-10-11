//! The working strip's background-task card: one row per task with its own clock, and an (i) beside
//! the clock that says the work continues in the background (`extras::background_tasks` in
//! gx-chat-core words it). Two or more tasks fold under one header drawn by the Threads card's own
//! `panel_header`, folded until clicked.
//!
//! CDXC:SessionChat 2026-10-11 SEE-ALSO:
//! The phone draws the same card in apps/mobile/app/src/chat/native/cards/WorkingStrip.tsx; the
//! decision and its wording live in packages/gx-chat-core/src/extras/background_tasks.rs.

use super::{appearance::ChatAppearance, state::NativeChatView};
use crate::app::helpers::ThrottledAnimationExt;
use gpui::StatefulInteractiveElement as _;
use gpui::{
    AnyElement, FontFeatures, FontWeight, InteractiveElement, IntoElement, ParentElement, Styled,
    div, px, relative, svg,
};
use serde_json::Value;
use std::time::Duration;

impl NativeChatView {
    pub(super) fn render_background_tasks(
        &self,
        tasks: &Value,
        p: &ChatAppearance,
        reduced_motion: bool,
        cx: &gpui::Context<Self>,
    ) -> AnyElement {
        let s = p.scale;
        let info = tasks["info"].as_str().unwrap_or_default();
        let mut rows = div().flex().flex_col().gap(px(6.0 * s)).min_w_0();
        let named = tasks["rows"].as_array().into_iter().flatten();
        let mut any_named = false;
        for (index, row) in named.enumerate() {
            any_named = true;
            rows = rows.child(self.background_task_row(index, row, info, p, reduced_motion));
        }
        if let Some(more) = tasks["more"].as_str() {
            rows = rows.child(
                div()
                    // A remainder under named rows lines up with their titles; alone it is the line.
                    .pl(px(if any_named { 22.0 * s } else { 0.0 }))
                    .text_size(px(13.0 * s))
                    .line_height(relative(1.5))
                    .text_color(p.muted)
                    .child(more.to_owned()),
            );
        }
        let card = if tasks["grouped"] == true {
            let open = tasks["open"] == true;
            let motion = self.disclosure_frame("background-tasks", open, cx);
            let toggle = serde_json::json!({"type": "toggleBackgroundTasks", "open": !open});
            let header = self.panel_header(
                super::panel_card::PanelHeader {
                    id: "chat-background-tasks-header",
                    icon: "titlebar/terminal-2.svg",
                    title: tasks["title"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned()
                        .into(),
                    meta: tasks["meta"].as_str().unwrap_or_default().to_owned(),
                    open,
                    has_body: open || motion.is_some(),
                    toggle_label: if open {
                        "Hide background tasks"
                    } else {
                        "Show background tasks"
                    },
                    trailing: None,
                    command: toggle,
                },
                p,
                cx,
            );
            let body = if open || motion.is_some() {
                vec![rows.into_any_element()]
            } else {
                Vec::new()
            };
            self.status_card_with_header_motion(
                super::cards::CardBodyMotion {
                    key: "background-tasks",
                    frame: motion,
                    shut_body: false,
                    shut: !open,
                },
                header,
                body,
                Vec::new(),
                p,
            )
        } else {
            self.status_card_with_header(rows.into_any_element(), Vec::new(), Vec::new(), p)
        };
        div()
            .id("chat-working-activity")
            .role(gpui::Role::Status)
            .w_full()
            .child(card)
            .into_any_element()
    }

    fn background_task_row(
        &self,
        index: usize,
        row: &Value,
        info: &str,
        p: &ChatAppearance,
        reduced_motion: bool,
    ) -> AnyElement {
        let s = p.scale;
        let glyph = svg()
            .path("titlebar/loader2.svg")
            .size(px(12.0 * s))
            .text_color(p.control_primary);
        let glyph = if reduced_motion {
            glyph.into_any_element()
        } else {
            glyph
                .with_throttled_animation(
                    ("chat-background-task", index),
                    Duration::from_secs(1),
                    |glyph, progress| {
                        glyph.with_transformation(gpui::Transformation::rotate(gpui::percentage(
                            progress,
                        )))
                    },
                )
                .into_any_element()
        };
        // One line of two runs, so the regular "Watching" and the bold title share a baseline and
        // truncate together.
        let prefix = row["prefix"]
            .as_str()
            .map(|prefix| format!("{prefix} "))
            .unwrap_or_default();
        let title = row["title"].as_str().unwrap_or_default();
        let run = |len: usize, weight: FontWeight| gpui::TextRun {
            len,
            font: gpui::Font {
                weight,
                ..gpui::font(p.font.clone())
            },
            color: p.foreground,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = vec![
            run(prefix.len(), FontWeight::NORMAL),
            run(title.len(), FontWeight::SEMIBOLD),
        ]
        .into_iter()
        .filter(|run| run.len > 0)
        .collect();
        let mut name = div()
            .id(("chat-background-task-title", index))
            .flex_1()
            .min_w_0()
            .truncate()
            .child(gpui::StyledText::new(format!("{prefix}{title}")).with_runs(runs));
        // The command the agent ran, for whoever wants to know exactly what it is.
        if let Some(command) = row["command"]
            .as_str()
            .filter(|command| !command.is_empty())
        {
            let command = command.to_owned();
            name = name.tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(command.clone()).build(window, cx)
            });
        }
        let line = div()
            .flex()
            .items_center()
            .gap(px(10.0 * s))
            .min_w_0()
            .text_size(px(14.0 * s))
            .line_height(relative(1.5))
            .child(
                div()
                    .size(px(12.0 * s))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(glyph),
            )
            .child(name);
        // The (i) sits right against the clock it explains.
        let mut trailing = div().flex().items_center().gap(px(6.0 * s)).flex_shrink_0();
        if !info.is_empty() {
            let info = info.to_owned();
            trailing = trailing.child(
                div()
                    .id(("chat-background-task-info", index))
                    .role(gpui::Role::Image)
                    .aria_label(info.clone())
                    .flex()
                    .items_center()
                    .justify_center()
                    .tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new(info.clone()).build(window, cx)
                    })
                    .child(
                        svg()
                            .path("titlebar/info-circle.svg")
                            .size(px(14.0 * s))
                            .text_color(p.muted),
                    ),
            );
        }
        if let Some(elapsed) = row["elapsed"].as_str() {
            trailing = trailing.child(
                div()
                    .text_color(p.muted)
                    .font_features(FontFeatures(vec![("tnum".into(), 1)].into()))
                    .child(elapsed.to_owned()),
            );
        }
        let line = line.child(trailing);
        line.into_any_element()
    }
}
