//! Answered question cards in the transcript.
//!
//! CDXC:SessionChat 2026-09-18 SEE-ALSO:
//! The rule that lifts a turn's exchanges out of its collapsed work fold lives in
//! packages/gx-chat-core/src/questions/hoisting.rs. The live question card
//! (question.rs) and this settled one share their card (ask_card.rs), so an answer reads the
//! same after the fact as it did while it was being given.

use super::ask_card::{
    AskBodyMotion, AskTone, answer_aside, answer_chip, ask_head, chevron, press_head, receipt_row,
};
use super::{appearance::ChatAppearance, state::NativeChatView, transcript::text};
use gpui::{
    AnyElement, Context, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use serde_json::Value;

impl NativeChatView {
    /// Every answered exchange carried by one message or hoisted onto one completed turn.
    pub(super) fn question_exchange_cards(
        &self,
        key: &str,
        exchanges: &Value,
        p: &ChatAppearance,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let exchanges = exchanges.as_array().filter(|list| !list.is_empty())?;
        let mut column = div()
            .flex()
            .flex_col()
            .min_w_0()
            .w_full()
            .gap(px(12.0 * p.scale))
            .py(px(6.0 * p.scale));
        for (index, exchange) in exchanges.iter().enumerate() {
            column = column.child(self.question_exchange_card(
                &format!("{key}:{index}"),
                exchange,
                p,
                cx,
            ));
        }
        Some(column.into_any_element())
    }

    /// CDXC:SessionChat 2026-10-11 DECISION: User picked "Clear ask, clear answer": an answered exchange is a compact green receipt ("You answered 2 questions", its headers under it) that folds to that one line; each question is a numbered row with the answer as a chip and its description beside it. A question nobody answered says Skipped or Dismissed, and an exchange with no answer at all is a muted "Question skipped" card.
    fn question_exchange_card(
        &self,
        key: &str,
        exchange: &Value,
        p: &ChatAppearance,
        cx: &Context<Self>,
    ) -> AnyElement {
        let s = p.scale;
        let questions = exchange["questions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let answers = exchange["answers"].as_array().cloned();
        let total = questions.len();
        let fallback = text(exchange, "fallbackText");
        let answer_at = |index: usize| {
            answers
                .as_ref()
                .and_then(|answers| answers.get(index))
                .cloned()
                .unwrap_or(Value::Null)
        };
        let answered = (0..total)
            .filter(|index| answer_given(&answer_at(*index)))
            .count();
        let dismissed = (0..total).any(|index| answer_at(index)["dismissed"] == true);
        let headers: Vec<String> = questions
            .iter()
            .map(|question| text(question, "header"))
            .filter(|header| !header.is_empty())
            .collect();
        let (tone, icon, title) = if answered > 0 || !fallback.is_empty() {
            (
                AskTone::Answered,
                "titlebar/check.svg",
                if total <= 1 || answered == 0 {
                    "You answered the question".to_string()
                } else if answered == total {
                    format!("You answered {total} questions")
                } else {
                    format!("You answered {answered} of {total} questions")
                },
            )
        } else if dismissed {
            (
                AskTone::Quiet,
                "titlebar/help-circle.svg",
                "Dismissed without answering".to_string(),
            )
        } else {
            (
                AskTone::Quiet,
                "titlebar/help-circle.svg",
                if total > 1 {
                    "Questions skipped".to_string()
                } else {
                    "Question skipped".to_string()
                },
            )
        };
        let mut detail = headers.join(" · ");
        if tone == AskTone::Quiet {
            let note = "No answer was given";
            detail = if detail.is_empty() {
                note.to_string()
            } else {
                format!("{detail} · {note}")
            };
        } else if detail.is_empty() && total == 1 {
            detail = text(&questions[0], "question");
        }
        let fold_key = format!("question-receipt:{key}");
        let open = !self.collapsed.contains(&fold_key);
        let motion = self.disclosure_frame(&fold_key, open, cx);
        let mut rows = div().flex().flex_col().min_w_0().w_full();
        for (index, question) in questions.iter().enumerate() {
            rows = rows.child(self.question_exchange_row(
                &format!("{key}:{index}"),
                question,
                &answer_at(index),
                answers.is_some(),
                index,
                p,
                cx,
            ));
        }
        if !fallback.is_empty() {
            rows = rows.child(receipt_row(
                total + 1,
                String::new(),
                vec![answer_chip(fallback, p)],
                Vec::new(),
                total == 0,
                p,
            ));
        }
        let body = div().mt(px(-6.0 * s)).child(rows).into_any_element();
        let toggle_key = fold_key.clone();
        let head = press_head(
            ask_head(icon, tone, title, detail, vec![chevron(open, p)], p),
            SharedString::from(format!("question-receipt-head:{key}")),
            if open {
                "Hide the answers"
            } else {
                "Show the answers"
            },
            open || motion.is_some(),
            p,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if !this.collapsed.remove(&toggle_key) {
                this.collapsed.insert(toggle_key.clone());
            }
            cx.notify();
        }))
        .into_any_element();
        let has_rows = total > 0 || !text(exchange, "fallbackText").is_empty();
        self.ask_card(
            head,
            (has_rows && (open || motion.is_some())).then_some(body),
            Vec::new(),
            Some(AskBodyMotion {
                key: &fold_key,
                frame: motion,
            }),
            p,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn question_exchange_row(
        &self,
        key: &str,
        question: &Value,
        answer: &Value,
        parsed_answers: bool,
        index: usize,
        p: &ChatAppearance,
        cx: &Context<Self>,
    ) -> AnyElement {
        let s = p.scale;
        let selected: Vec<usize> = answer["selectedIndices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_u64().map(|value| value as usize))
            .collect();
        let options = question["options"].as_array().cloned().unwrap_or_default();
        let other = text(answer, "otherText");
        let mut prompt = text(question, "question");
        if prompt.is_empty() {
            prompt = text(question, "header");
        }
        let mut chips = Vec::new();
        let picked: Vec<&Value> = selected
            .iter()
            .filter_map(|option_index| options.get(*option_index))
            .collect();
        for option in &picked {
            chips.push(answer_chip(text(option, "label"), p));
        }
        // One picked option's description reads beside its chip, as the card showed it while it was asked.
        if let [option] = picked.as_slice() {
            let description = text(option, "description");
            if !description.is_empty() && description != text(option, "label") {
                chips.push(answer_aside(description, false, p));
            }
        }
        let mut below = Vec::new();
        if !other.is_empty() {
            if picked.is_empty() {
                chips.push(answer_chip(other, p));
                chips.push(answer_aside("Your own answer".to_string(), false, p));
            } else {
                below.push(answer_aside(format!("Note: {other}"), false, p));
            }
        }
        // A question the reader walked away from still says so, rather than looking unanswered.
        if chips.is_empty() {
            if answer["dismissed"] == true {
                chips.push(answer_aside(
                    "Dismissed without answering".to_string(),
                    true,
                    p,
                ));
            } else if parsed_answers {
                chips.push(answer_aside("Skipped".to_string(), true, p));
            }
        }
        if !options.is_empty() {
            let options_key = format!("question-exchange:{key}");
            let expanded = self.expanded.contains(&options_key);
            let motion = self.disclosure_frame(&options_key, expanded, cx);
            below.push(
                div()
                    .text_size(px(12.5 * s))
                    .text_color(p.muted)
                    .child(self.disclosure(
                        options_key.clone(),
                        if expanded {
                            "Hide options".to_string()
                        } else {
                            format!("Show all {} options", options.len())
                        },
                        expanded,
                        None,
                        p,
                        cx,
                    ))
                    .into_any_element(),
            );
            if expanded || motion.is_some() {
                let mut rows = div().flex().flex_col().gap(px(6.0 * s)).pt(px(2.0 * s));
                for (option_index, option) in options.iter().enumerate() {
                    rows = rows.child(self.choice_row(
                        format!("{key}:option:{option_index}"),
                        text(option, "label"),
                        text(option, "description"),
                        selected.contains(&option_index),
                        None,
                        true,
                        false,
                        // Settled answers are a record, never a control.
                        true,
                        Value::Null,
                        p,
                        cx,
                    ));
                }
                below.push(self.disclosure_body_motion(
                    &options_key,
                    motion,
                    0.0,
                    rows.into_any_element(),
                ));
            }
        }
        receipt_row(index + 1, prompt, chips, below, index == 0, p)
    }
}

/// Whether the reader gave this question an answer: a pick or their own words.
fn answer_given(answer: &Value) -> bool {
    answer["selectedIndices"]
        .as_array()
        .is_some_and(|picked| !picked.is_empty())
        || !text(answer, "otherText").is_empty()
}
