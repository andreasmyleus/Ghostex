//! The answers a reader sent from an agent's HTML page, drawn as a card instead of the JSON message
//! gxserver typed into the session (`pageAnswers`, packages/gx-chat-core/src/transcript/page_answers.rs).
//!
//! CDXC:SessionChat 2026-10-11 SEE-ALSO:
//! The phone draws the same card in apps/mobile/app/src/chat/native/transcript/PageAnswersCard.tsx; the page's own card reads Answered in visual.rs. Change them together.

use super::ask_card::{AskTone, answer_aside, answer_chip, ask_head, receipt_row};
use super::{appearance::ChatAppearance, state::NativeChatView, transcript::text};
use gpui::{AnyElement, Hsla, IntoElement, ParentElement as _, Styled as _, div, px, rgb};
use serde_json::Value;

/// The green the chat's charts use for good news, which also marks an answered page.
pub(super) fn answered_color(p: &ChatAppearance) -> Hsla {
    rgb(if p.light { 0x059669 } else { 0x34d399 }).into()
}

impl NativeChatView {
    /// CDXC:SessionChat 2026-10-11 DECISION: User: the answers card takes the "Clear ask, clear answer" receipt (ask_card.rs): a green check tile, numbered rows, each answer a green chip. A long or multi-line answer (a text question's) stays plain text so it keeps the card's width.
    pub(super) fn page_answers_card(&self, answers: &Value, p: &ChatAppearance) -> AnyElement {
        let rows: Vec<Value> = answers["rows"].as_array().cloned().unwrap_or_default();
        let more = answers["more"].as_u64().unwrap_or(0) as usize;
        let count = rows.len() + more;
        let head = ask_head(
            "titlebar/check.svg",
            AskTone::Answered,
            format!("You answered “{}”", text(answers, "title")),
            if count == 1 {
                "1 answer".to_string()
            } else {
                format!("{count} answers")
            },
            Vec::new(),
            p,
        )
        .into_any_element();
        let s = p.scale;
        let mut body = div().mt(px(-6.0 * s)).flex().flex_col().w_full().min_w_0();
        for (index, row) in rows.iter().enumerate() {
            let value = text(row, "value");
            let note = text(row, "note");
            // CDXC:SessionChat 2026-10-11 DECISION: User: "make it show the Q above the Answer so they both have more width area": each row is the question (muted) on its own line, the answer below it at the card's full width, then its note. The phone (PageAnswersCard.tsx) stacks the same way.
            let answer = if value.chars().count() > 80 || value.contains('\n') {
                div()
                    .min_w_0()
                    .text_size(px(14.0 * s))
                    .line_height(px(20.0 * s))
                    .text_color(p.foreground)
                    .child(value)
                    .into_any_element()
            } else {
                answer_chip(value, p)
            };
            let below = if note.is_empty() {
                Vec::new()
            } else {
                vec![answer_aside(note, false, p)]
            };
            body = body.child(receipt_row(
                index + 1,
                text(row, "label"),
                vec![answer],
                below,
                index == 0,
                p,
            ));
        }
        if more > 0 {
            body = body.child(div().pt(px(4.0 * s)).child(answer_aside(
                format!("and {more} more"),
                false,
                p,
            )));
        }
        self.ask_card(head, Some(body.into_any_element()), Vec::new(), None, p)
    }
}
