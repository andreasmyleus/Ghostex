//! The answers a reader sent from an agent's HTML page, drawn as a card instead of the JSON message
//! gxserver typed into the session (`pageAnswers`, packages/gx-chat-core/src/transcript/page_answers.rs).
//!
//! CDXC:SessionChat 2026-10-11 SEE-ALSO:
//! The phone draws the same card in apps/mobile/app/src/chat/native/transcript/PageAnswersCard.tsx; the page's own card reads Answered in visual.rs. Change them together.

use super::ask_card::{AskTone, answer_aside, ask_head, plain_answer, receipt_row};
use super::{appearance::ChatAppearance, state::NativeChatView, transcript::text};
use gpui::{AnyElement, Hsla, IntoElement, ParentElement as _, Styled as _, div, px, rgb};
use serde_json::Value;

/// The green the chat's charts use for good news, which also marks an answered page.
pub(super) fn answered_color(p: &ChatAppearance) -> Hsla {
    rgb(if p.light { 0x059669 } else { 0x34d399 }).into()
}

impl NativeChatView {
    /// CDXC:SessionChat 2026-10-12 DECISION: User, on the answer card under an answered form: "please make sure the selected answer doesn't have that green thing around it, make it more simple should just say the answer and question there with bit different color/fontweight etc (like we have in other cards) but no background behind the answer like you're showing / also plase remove "1 answer" no need for it". The card keeps its green check tile and numbered rows; each answer is plain foreground text at medium weight under its muted question, with no chip, border or fill, and the head has no answer count. Supersedes the 2026-10-11 green answer chips on this card. The agent's own question receipts (question_exchange.rs) take the same plain answer (`plain_answer`, ask_card.rs).
    pub(super) fn page_answers_card(&self, answers: &Value, p: &ChatAppearance) -> AnyElement {
        let rows: Vec<Value> = answers["rows"].as_array().cloned().unwrap_or_default();
        let more = answers["more"].as_u64().unwrap_or(0) as usize;
        let head = ask_head(
            "titlebar/check.svg",
            AskTone::Answered,
            format!("You answered “{}”", text(answers, "title")),
            String::new(),
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
            let answer = plain_answer(value, p);
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
