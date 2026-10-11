//! How a marked section looks: the palette, the three styles, and the sample
//! reply the Settings preview draws in each.
//!
//! The file depends only on gpui and the chat core, so Settings includes it as its own module
//! (`settings_modal/tabs/general/marked_sections_preview.rs`): the Settings page also builds into
//! the standalone modal demo, which has no chat renderer to call into.
//!
//! CDXC:MarkedHandovers 2026-10-11 DECISION:
//! User picked three styles from the mockups (docs/2026-10-11/chat-section-markers/index.html): A, a thin left rule in the kind's colour with the emoji kept; C, a coloured badge naming the kind in place of the emoji, dropping a bold label that only repeats it; D, the emoji kept and the label coloured, the default. The chat colours marked sections whenever a reply has them: orchestrators and their threads mark their handovers even while Marked handovers is off (that switch extends marking to every other session). Supersedes the same day's summary strip at the top of the message (user: "let's actually just remove the summary at the top i don't think it's useful at all").
//! SEE-ALSO: apps/desktop/src/app/native_chat/marked_sections.rs (the transcript's use of it), apps/mobile/app/src/chat/native/transcript/markdown/MarkedSections.tsx (the phone draws the same, with the same palette).
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px, rgb,
};
use serde::Deserialize;

pub(crate) const SECTION_OPEN: &str = "\u{E000}section:";
pub(crate) const SECTION_CLOSE: &str = "\u{E000}/section";

/// The "In progress" colour (user pick 2026-10-11: #00A6ED). It reads 2.7:1 on a white chat, too
/// faint for a label, so the light chat takes the same hue darkened to #0077B6 (4.9:1).
const IN_PROGRESS_DARK: u32 = 0x00a6ed;
const IN_PROGRESS_LIGHT: u32 = 0x0077b6;

/// How the chat draws marked sections (`markedHandoversStyle`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MarkerStyle {
    /// A: a thin rule in the kind's colour down the section's left edge.
    Rule,
    /// C: a pill naming the kind in place of the emoji.
    Badge,
    /// D: the emoji kept and the label coloured.
    Labels,
}

impl MarkerStyle {
    /// The style the settings ask for. It applies whether or not Marked handovers is on, because
    /// orchestrators and their threads mark their handovers either way.
    pub(crate) fn from_settings(settings: &serde_json::Map<String, serde_json::Value>) -> Self {
        Self::from_value(
            settings
                .get("markedHandoversStyle")
                .and_then(serde_json::Value::as_str),
        )
    }

    pub(crate) fn from_value(value: Option<&str>) -> Self {
        match value {
            Some("rule") => Self::Rule,
            Some("badge") => Self::Badge,
            _ => Self::Labels,
        }
    }

    /// Whether the style draws the section's first line itself (from the core's label and lead)
    /// instead of the line as written.
    pub(crate) fn draws_first_line(self) -> bool {
        !matches!(self, Self::Rule)
    }
}

/// A section as the core described it (`packages/gx-chat-core/src/transcript/marked_sections.rs`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct SectionMeta {
    pub(crate) kind: String,
    pub(crate) emoji: String,
    pub(crate) name: String,
    pub(crate) label: String,
    pub(crate) label_repeats: bool,
    pub(crate) lead: String,
}

/// The chat tones a section is drawn against.
#[derive(Clone)]
pub(crate) struct MarkedLook {
    pub(crate) light: bool,
    pub(crate) scale: f32,
    pub(crate) foreground: Hsla,
    pub(crate) prose: Hsla,
    pub(crate) border: Hsla,
    /// The transcript's prose line height, unscaled.
    pub(crate) line_height: f32,
    /// The gap between Markdown blocks, scaled.
    pub(crate) block_gap: f32,
}

impl MarkedLook {
    /// The chat's default tones, for a preview drawn outside a chat.
    pub(crate) fn chat_default(light: bool) -> Self {
        let color = |dark: u32, light_color: u32| -> Hsla {
            rgb(if light { light_color } else { dark }).into()
        };
        Self {
            light,
            scale: 1.0,
            foreground: color(0xfcfcfc, 0x27272a),
            prose: color(0xb4b8c0, 0x4d4d50),
            border: color(0x1c1c1c, 0xe4e4e7),
            line_height: 22.75,
            block_gap: 12.0,
        }
    }
}

/// A kind's colour in the chat's light or dark appearance (the mockup's palette).
pub(crate) fn kind_color(kind: &str, light: bool) -> Hsla {
    let (dark, light_color) = match kind {
        "need" => (0x60a5fa, 0x2563eb),
        "done" => (0x4ade80, 0x15803d),
        "read" => (0xfbbf24, 0xb45309),
        "kicked" => (0xf472b6, 0xbe185d),
        "progress" => (IN_PROGRESS_DARK, IN_PROGRESS_LIGHT),
        "untested" => (0x2dd4bf, 0x0f766e),
        "shipped" => (0x818cf8, 0x4f46e5),
        "blocked" => (0xf87171, 0xdc2626),
        _ => (0x9e9e9e, 0x71717b),
    };
    rgb(if light { light_color } else { dark }).into()
}

/// The faint fill and the edge a chip or badge takes from its kind's colour.
fn tint(look: &MarkedLook) -> (f32, f32) {
    if look.light {
        (0.07, 0.28)
    } else {
        (0.11, 0.30)
    }
}

/// What the first line opens with in styles C and D: the badge (C) or the emoji (D), then the
/// label (coloured in D, plain bold in C unless it only repeats the badge).
fn first_line_opening(
    style: MarkerStyle,
    meta: &SectionMeta,
    look: &MarkedLook,
) -> Vec<AnyElement> {
    let s = look.scale;
    let color = kind_color(&meta.kind, look.light);
    let line = px(look.line_height * s);
    let label = |color: Hsla| {
        div()
            .flex_shrink_0()
            .line_height(line)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(color)
            .child(meta.label.clone())
            .into_any_element()
    };
    let mut parts: Vec<AnyElement> = Vec::new();
    if style == MarkerStyle::Badge {
        let (fill, edge) = tint(look);
        parts.push(
            div()
                .flex_shrink_0()
                .h(line)
                .flex()
                .items_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.0 * s))
                        .pl(px(7.0 * s))
                        .pr(px(8.0 * s))
                        .py(px(3.0 * s))
                        .rounded_full()
                        .border_1()
                        .border_color(color.opacity(edge))
                        .bg(color.opacity(fill + 0.05))
                        .text_color(color)
                        .text_size(px(11.0 * s))
                        .line_height(px(13.0 * s))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(div().size(px(6.0 * s)).rounded_full().bg(color))
                        .child(meta.name.to_uppercase()),
                )
                .into_any_element(),
        );
        if !meta.label.is_empty() && !meta.label_repeats {
            parts.push(label(look.foreground));
        }
    } else {
        parts.push(
            div()
                .flex_shrink_0()
                .line_height(line)
                .child(meta.emoji.clone())
                .into_any_element(),
        );
        if !meta.label.is_empty() {
            parts.push(label(color));
        }
    }
    parts
}

/// One section in `style`. `lead` is the rest of the first line drawn as Markdown (styles C and D);
/// `body` is the section's lines drawn as Markdown: all of them for A, the ones after the first
/// for C and D.
pub(crate) fn section(
    style: MarkerStyle,
    meta: &SectionMeta,
    lead: Option<AnyElement>,
    body: Vec<AnyElement>,
    look: &MarkedLook,
) -> AnyElement {
    let s = look.scale;
    let column = div()
        .relative()
        .flex()
        .flex_col()
        .w_full()
        .min_w_0()
        .gap(px(look.block_gap));
    if !style.draws_first_line() {
        return column
            .border_l(px(3.0 * s))
            .border_color(kind_color(&meta.kind, look.light))
            .pl(px(12.0 * s))
            .children(body)
            .into_any_element();
    }
    let header = div()
        .flex()
        .items_start()
        .w_full()
        .min_w_0()
        .gap(px(6.0 * s))
        .text_size(px(14.0 * s))
        .text_color(look.prose)
        .children(first_line_opening(style, meta, look))
        .when_some(lead, |row, lead| {
            row.child(div().flex_1().min_w_0().child(lead))
        });
    column.child(header).children(body).into_any_element()
}

/// The reply the Settings preview draws in every style: the mockup's orchestrator handover.
const PREVIEW_REPLY: &str = "Here's where the three threads stand after this round.\n\n✅ **Done:**\n- The questions form draws on desktop and the web build; picks survive a reload.\n- Settings > Skills: the trash button comes first.\n\n🛫 **Kicked off:** a thread for the Windows installer.\n\n🔄 **In progress:**\n- Phone chat: the note box still loses focus when the keyboard opens.\n- Windows installer: started this turn.\n\n⚠️ **Read this:** a questions block with more than 20 questions is now refused with an error.\n\n❓ **Needs you:**\n- **Q1** Should a sent form collapse to one line? (a) collapse (b) stay open\n- **Q2** OK to ship the phone part first? (a) yes (b) wait\n\n🧪 Untested: answering from the floating page on Windows.";

/// The preview reply's marked text split at its marks: prose and sections.
enum PreviewPart {
    Prose(String),
    Section(SectionMeta, Vec<String>),
}

fn preview_parts(lines: &[&str]) -> Vec<PreviewPart> {
    let mut parts = Vec::new();
    let mut prose: Vec<&str> = Vec::new();
    let flush = |prose: &mut Vec<&str>, parts: &mut Vec<PreviewPart>| {
        if prose.iter().any(|line| !line.trim().is_empty()) {
            parts.push(PreviewPart::Prose(prose.join("\n")));
        }
        prose.clear();
    };
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if let Some(meta) = line.strip_prefix(SECTION_OPEN) {
            flush(&mut prose, &mut parts);
            let end = lines[index + 1..]
                .iter()
                .position(|line| *line == SECTION_CLOSE)
                .map_or(lines.len(), |offset| index + 1 + offset);
            parts.push(PreviewPart::Section(
                serde_json::from_str(meta).unwrap_or_default(),
                lines[index + 1..end]
                    .iter()
                    .map(|line| (*line).to_string())
                    .collect(),
            ));
            index = end;
        } else {
            prose.push(line);
        }
        index += 1;
    }
    flush(&mut prose, &mut parts);
    parts
}

/// The sample reply drawn in `style`, as the chat draws it; `text` draws a piece of Markdown.
pub(crate) fn preview_reply(
    style: MarkerStyle,
    look: &MarkedLook,
    text: impl Fn(String, String) -> AnyElement,
) -> AnyElement {
    use ghostex_gx_chat_core::transcript::{line_breaks, marked_sections, native_markdown};
    let body = line_breaks::agent_line_breaks(PREVIEW_REPLY, line_breaks::AgentLineBreaks::Every);
    let marked = marked_sections::mark_sections(native_markdown::native_markdown(&body, false));
    let lines: Vec<&str> = marked.lines().collect();
    let id = format!("marked-preview-{style:?}");
    let children = preview_parts(&lines)
        .into_iter()
        .enumerate()
        .map(|(index, part)| {
            let key = format!("{id}:{index}");
            match part {
                PreviewPart::Prose(content) => text(key, content),
                PreviewPart::Section(meta, lines) => {
                    let (lead, body) = if style.draws_first_line() {
                        let lead = (!meta.lead.is_empty())
                            .then(|| text(format!("{key}:lead"), meta.lead.clone()));
                        (lead, lines.get(1..).unwrap_or_default().join("\n"))
                    } else {
                        (None, lines.join("\n"))
                    };
                    let body = (!body.trim().is_empty())
                        .then(|| text(format!("{key}:body"), body))
                        .into_iter()
                        .collect();
                    section(style, &meta, lead, body, look)
                }
            }
        })
        .collect::<Vec<_>>();
    div()
        .flex()
        .flex_col()
        .w_full()
        .min_w_0()
        .gap(px(look.block_gap))
        .text_size(px(14.0 * look.scale))
        .text_color(look.prose)
        .children(children)
        .into_any_element()
}
