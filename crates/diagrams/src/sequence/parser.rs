use winnow::combinator::{alt, opt};
use winnow::prelude::*;
use winnow::token::take_while;

use crate::common::error::{ParseError, ParseErrorKind};
use crate::common::tokens::{identifier, skip, ws};

use super::ir::*;

/// Parse a mermaid sequence diagram string into IR.
pub fn parse(input: &str) -> Result<SequenceDiagram, ParseError> {
    let mut rest = input;
    parse_sequence_diagram(&mut rest).map_err(|_| {
        let offset = input.len() - rest.len();
        ParseError::new(ParseErrorKind::UnexpectedToken, offset..offset, input)
    })
}

/// Shared state accumulated during parsing (participants, title, autonumber).
/// Items are returned from `parse_items` rather than accumulated here,
/// because different scopes (top-level vs fragment sections) own their items.
#[derive(Default)]
struct ParseContext {
    participants: Vec<Participant>,
    title: Option<String>,
    autonumber: Option<AutoNumber>,
}

fn parse_sequence_diagram(input: &mut &str) -> ModalResult<SequenceDiagram> {
    skip.parse_next(input)?;
    "sequenceDiagram".parse_next(input)?;

    let mut ctx = ParseContext::default();
    let items = parse_items(input, &mut ctx, &[])?;

    Ok(SequenceDiagram {
        title: ctx.title,
        participants: ctx.participants,
        items,
        autonumber: ctx.autonumber,
    })
}

// ── Body / item parsing ────────────────────────────────────

/// Parse items until EOF or a stop keyword is reached.
/// Stop keywords are NOT consumed — the caller handles them.
fn parse_items(
    input: &mut &str,
    ctx: &mut ParseContext,
    stop_keywords: &[&str],
) -> ModalResult<Vec<SequenceItem>> {
    let mut items = Vec::new();
    loop {
        skip.parse_next(input)?;
        if input.is_empty() || at_keyword_boundary(input, stop_keywords) {
            return Ok(items);
        }
        if !try_parse_statement(input, ctx, &mut items)? {
            // Skip one unrecognised **char** (not byte). `&input[1..]` used
            // to panic here on any multi-byte UTF-8 start (CJK, symbols,
            // the `▋` streaming cursor, etc.) because byte index 1 lands
            // mid-codepoint.
            if !input.is_empty() {
                let mut chars = input.chars();
                chars.next();
                *input = chars.as_str();
            }
        }
    }
}

/// Check if input starts with one of the given keywords at a word boundary.
fn at_keyword_boundary(input: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|kw| {
        input.starts_with(kw) && {
            let rest = &input[kw.len()..];
            rest.is_empty()
                || rest.starts_with(|c: char| c.is_ascii_whitespace())
                || rest.starts_with("%%")
        }
    })
}

/// Check if input starts with a keyword followed by whitespace/EOL/comment/colon.
fn starts_with_keyword(input: &str, keyword: &str) -> bool {
    input.starts_with(keyword) && {
        let rest = &input[keyword.len()..];
        rest.is_empty()
            || rest.starts_with(|c: char| c.is_ascii_whitespace())
            || rest.starts_with("%%")
            || rest.starts_with(':')
    }
}

/// Case-insensitive keyword check (for "Note"/"note").
fn starts_with_keyword_ci(input: &str, keyword: &str) -> bool {
    input.len() >= keyword.len() && input[..keyword.len()].eq_ignore_ascii_case(keyword) && {
        let rest = &input[keyword.len()..];
        rest.is_empty()
            || rest.starts_with(|c: char| c.is_ascii_whitespace())
            || rest.starts_with("%%")
    }
}

fn try_parse_statement(
    input: &mut &str,
    ctx: &mut ParseContext,
    items: &mut Vec<SequenceItem>,
) -> ModalResult<bool> {
    // Title
    if starts_with_keyword(input, "title") {
        let checkpoint = *input;
        if let Ok(t) = parse_title(input) {
            ctx.title = Some(t);
            return Ok(true);
        }
        *input = checkpoint;
    }

    // Autonumber
    if starts_with_keyword(input, "autonumber") {
        let checkpoint = *input;
        if let Ok(an) = parse_autonumber(input) {
            ctx.autonumber = an;
            return Ok(true);
        }
        *input = checkpoint;
    }

    // Participant / Actor
    if starts_with_keyword(input, "participant") || starts_with_keyword(input, "actor") {
        let checkpoint = *input;
        if let Ok(p) = parse_participant_decl(input) {
            if !ctx.participants.iter().any(|existing| existing.id == p.id) {
                ctx.participants.push(p);
            }
            return Ok(true);
        }
        *input = checkpoint;
    }

    // Activate / Deactivate
    if starts_with_keyword(input, "activate") {
        let checkpoint = *input;
        *input = &input[8..];
        ws.parse_next(input)?;
        if let Ok(actor) = identifier.parse_next(input) {
            items.push(SequenceItem::Activation(Activation {
                actor: actor.to_string(),
                active: true,
            }));
            return Ok(true);
        }
        *input = checkpoint;
    }
    if starts_with_keyword(input, "deactivate") {
        let checkpoint = *input;
        *input = &input[10..];
        ws.parse_next(input)?;
        if let Ok(actor) = identifier.parse_next(input) {
            items.push(SequenceItem::Activation(Activation {
                actor: actor.to_string(),
                active: false,
            }));
            return Ok(true);
        }
        *input = checkpoint;
    }

    // Note
    if starts_with_keyword_ci(input, "note") {
        let checkpoint = *input;
        if let Ok(note) = parse_note(input) {
            items.push(SequenceItem::Note(note));
            return Ok(true);
        }
        *input = checkpoint;
    }

    // Fragment (loop/alt/opt/par/critical/break)
    {
        let checkpoint = *input;
        if let Ok(frag) = parse_fragment(input, ctx) {
            items.push(SequenceItem::Fragment(frag));
            return Ok(true);
        }
        *input = checkpoint;
    }

    // Message (last — actor identifiers are permissive)
    {
        let checkpoint = *input;
        if let Ok(msg) = parse_message(input, &mut ctx.participants) {
            items.push(SequenceItem::Message(msg));
            return Ok(true);
        }
        *input = checkpoint;
    }

    Ok(false)
}

// ── Title ──────────────────────────────────────────────────

fn parse_title(input: &mut &str) -> ModalResult<String> {
    "title".parse_next(input)?;
    ws.parse_next(input)?;
    opt(':').parse_next(input)?;
    ws.parse_next(input)?;
    let text = take_while(1.., |c: char| c != '\n' && c != '\r').parse_next(input)?;
    Ok(text.trim().to_string())
}

// ── Autonumber ─────────────────────────────────────────────

fn parse_autonumber(input: &mut &str) -> ModalResult<Option<AutoNumber>> {
    "autonumber".parse_next(input)?;
    ws.parse_next(input)?;

    // "autonumber off" disables numbering
    if starts_with_keyword(input, "off") {
        *input = &input[3..];
        return Ok(None);
    }

    let start_str: &str = take_while(0.., |c: char| c.is_ascii_digit()).parse_next(input)?;
    let start = if start_str.is_empty() {
        1
    } else {
        start_str.parse::<u32>().unwrap_or(1)
    };

    ws.parse_next(input)?;

    let step_str: &str = take_while(0.., |c: char| c.is_ascii_digit()).parse_next(input)?;
    let step = if step_str.is_empty() {
        1
    } else {
        step_str.parse::<u32>().unwrap_or(1)
    };

    Ok(Some(AutoNumber { start, step }))
}

// ── Participant / Actor ────────────────────────────────────

fn parse_participant_decl(input: &mut &str) -> ModalResult<Participant> {
    let kind = alt((
        "participant".value(ParticipantKind::Box),
        "actor".value(ParticipantKind::Actor),
    ))
    .parse_next(input)?;

    ws.parse_next(input)?;
    let id = identifier.parse_next(input)?;
    let label = parse_as_label(input)?;
    let label = label.unwrap_or_else(|| id.to_string());

    Ok(Participant {
        id: id.to_string(),
        label,
        kind,
    })
}

/// Parse optional `as Label` or `as "Quoted Label"`.
fn parse_as_label(input: &mut &str) -> ModalResult<Option<String>> {
    ws.parse_next(input)?;
    if !input.starts_with("as") {
        return Ok(None);
    }
    let after = &input[2..];
    if !after.is_empty() && !after.starts_with(|c: char| c.is_ascii_whitespace()) {
        return Ok(None);
    }
    *input = after;
    ws.parse_next(input)?;

    if input.starts_with('"') {
        *input = &input[1..];
        let label = take_while(0.., |c: char| c != '"').parse_next(input)?;
        '"'.parse_next(input)?;
        Ok(Some(label.to_string()))
    } else {
        let label = take_while(1.., |c: char| c != '\n' && c != '\r').parse_next(input)?;
        Ok(Some(label.trim().to_string()))
    }
}

// ── Note ───────────────────────────────────────────────────

fn parse_note(input: &mut &str) -> ModalResult<Note> {
    // Accept "Note" (mermaid.js canonical) and "note"
    alt(("Note", "note")).parse_next(input)?;
    ws.parse_next(input)?;

    let position = if input.starts_with("left of") {
        *input = &input[7..];
        ws.parse_next(input)?;
        let actor = identifier.parse_next(input)?;
        NotePosition::LeftOf(actor.to_string())
    } else if input.starts_with("right of") {
        *input = &input[8..];
        ws.parse_next(input)?;
        let actor = identifier.parse_next(input)?;
        NotePosition::RightOf(actor.to_string())
    } else if input.starts_with("over") {
        *input = &input[4..];
        ws.parse_next(input)?;
        let mut actors = vec![identifier.parse_next(input)?.to_string()];
        loop {
            ws.parse_next(input)?;
            if !input.starts_with(',') {
                break;
            }
            *input = &input[1..];
            ws.parse_next(input)?;
            actors.push(identifier.parse_next(input)?.to_string());
        }
        NotePosition::Over(actors)
    } else {
        return Err(winnow::error::ErrMode::Backtrack(
            winnow::error::ContextError::new(),
        ));
    };

    ws.parse_next(input)?;
    ':'.parse_next(input)?;
    ws.parse_next(input)?;
    let text = take_while(1.., |c: char| c != '\n' && c != '\r').parse_next(input)?;

    Ok(Note {
        position,
        text: text.trim().to_string(),
    })
}

// ── Fragment ───────────────────────────────────────────────

fn parse_fragment(input: &mut &str, ctx: &mut ParseContext) -> ModalResult<Fragment> {
    let kind = alt((
        "loop".value(FragmentKind::Loop),
        "critical".value(FragmentKind::Critical),
        "break".value(FragmentKind::Break),
        "alt".value(FragmentKind::Alt),
        "opt".value(FragmentKind::Opt),
        "par".value(FragmentKind::Par),
    ))
    .parse_next(input)?;

    // Verify word boundary
    if !input.is_empty()
        && !input.starts_with(|c: char| c.is_ascii_whitespace())
        && !input.starts_with("%%")
    {
        return Err(winnow::error::ErrMode::Backtrack(
            winnow::error::ContextError::new(),
        ));
    }

    // Fragment label (rest of line)
    ws.parse_next(input)?;
    let label_text: &str = take_while(0.., |c: char| c != '\n' && c != '\r').parse_next(input)?;
    let label = if label_text.trim().is_empty() {
        None
    } else {
        Some(label_text.trim().to_string())
    };

    // Determine section divider keyword
    let divider = match kind {
        FragmentKind::Alt => Some("else"),
        FragmentKind::Par => Some("and"),
        FragmentKind::Critical => Some("option"),
        _ => None,
    };

    // Stop keywords for inner sections
    let inner_stop: Vec<&str> = match divider {
        Some(d) => vec!["end", d],
        None => vec!["end"],
    };

    let mut frag = Fragment::new(kind);
    frag.label = label.clone();

    // First section
    let section_items = parse_items(input, ctx, &inner_stop)?;
    let mut first_section = FragmentSection::new();
    first_section.label = label;
    first_section.items = section_items;
    frag.sections.push(first_section);

    // Additional sections (else/and/option)
    if let Some(d) = divider {
        while at_keyword_boundary(input, &[d]) {
            *input = &input[d.len()..];
            ws.parse_next(input)?;
            let section_label: &str =
                take_while(0.., |c: char| c != '\n' && c != '\r').parse_next(input)?;
            let section_label = if section_label.trim().is_empty() {
                None
            } else {
                Some(section_label.trim().to_string())
            };

            let section_items = parse_items(input, ctx, &inner_stop)?;
            let mut section = FragmentSection::new();
            section.label = section_label;
            section.items = section_items;
            frag.sections.push(section);
        }
    }

    // Consume "end"
    skip.parse_next(input)?;
    if at_keyword_boundary(input, &["end"]) {
        *input = &input[3..];
    }

    Ok(frag)
}

// ── Message ────────────────────────────────────────────────

fn parse_message(input: &mut &str, participants: &mut Vec<Participant>) -> ModalResult<Message> {
    let from = identifier.parse_next(input)?;
    let from_str = from.to_string();

    ws.parse_next(input)?;
    let arrow = parse_arrow(input)?;

    // Optional activation suffix: + activates target, - deactivates source
    let (activate, deactivate) = match input.chars().next() {
        Some('+') => {
            *input = &input[1..];
            (true, false)
        }
        Some('-') => {
            *input = &input[1..];
            (false, true)
        }
        _ => (false, false),
    };

    ws.parse_next(input)?;
    let to = identifier.parse_next(input)?;
    let to_str = to.to_string();

    ensure_participant(participants, &from_str);
    ensure_participant(participants, &to_str);

    // Optional `: label`
    ws.parse_next(input)?;
    let label = if input.starts_with(':') {
        *input = &input[1..];
        ws.parse_next(input)?;
        let text = take_while(0.., |c: char| c != '\n' && c != '\r').parse_next(input)?;
        let text = text.trim();
        if text.is_empty() {
            None
        } else {
            Some(text.to_string())
        }
    } else {
        None
    };

    Ok(Message {
        from: from_str,
        to: to_str,
        label,
        arrow,
        activate,
        deactivate,
    })
}

fn ensure_participant(participants: &mut Vec<Participant>, id: &str) {
    if !participants.iter().any(|p| p.id == id) {
        participants.push(Participant::new(id, id));
    }
}

/// Parse a sequence diagram arrow. Longest match first to avoid prefix ambiguity.
fn parse_arrow(input: &mut &str) -> ModalResult<ArrowStyle> {
    alt((
        "-->>".value(ArrowStyle::DOTTED_FILLED),
        "--x".value(ArrowStyle::DOTTED_CROSS),
        "--)".value(ArrowStyle {
            line: LineStyle::Dotted,
            head: ArrowHead::None,
        }),
        "-->".value(ArrowStyle::DOTTED_OPEN),
        "->>".value(ArrowStyle::SOLID_FILLED),
        "-x".value(ArrowStyle::SOLID_CROSS),
        "-)".value(ArrowStyle {
            line: LineStyle::Solid,
            head: ArrowHead::None,
        }),
        "->".value(ArrowStyle::SOLID_OPEN),
    ))
    .parse_next(input)
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod parser_tests;
