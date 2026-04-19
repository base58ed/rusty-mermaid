use winnow::combinator::{alt, opt};
use winnow::prelude::*;
use winnow::token::take_while;

use rusty_mermaid_core::Direction;

use crate::common::error::{ParseError, ParseErrorKind};
use crate::common::styling::{ClassDef, class_apply_body, class_def_body, style_stmt_body};
use crate::common::tokens::{identifier, skip, unescape_unicode, ws};

use super::ir::*;

/// Accumulates parsed statements during recursive descent.
struct ParseCtx {
    states: Vec<StateNode>,
    transitions: Vec<StateTransition>,
    notes: Vec<StateNote>,
    class_defs: Vec<ClassDef>,
    style_stmts: Vec<StateStyleStmt>,
    direction: Direction,
}

/// Parse a mermaid state diagram string into IR.
pub fn parse(input: &str) -> Result<StateDiagram, ParseError> {
    let mut rest = input;
    parse_state_diagram(&mut rest).map_err(|_| {
        let offset = input.len() - rest.len();
        ParseError::new(ParseErrorKind::UnexpectedToken, offset..offset, input)
    })
}

fn parse_state_diagram(input: &mut &str) -> ModalResult<StateDiagram> {
    skip.parse_next(input)?;
    header.parse_next(input)?;
    skip.parse_next(input)?;

    let mut ctx = ParseCtx {
        states: Vec::new(),
        transitions: Vec::new(),
        notes: Vec::new(),
        class_defs: Vec::new(),
        style_stmts: Vec::new(),
        direction: Direction::TB,
    };
    parse_body(input, &mut ctx, true)?;

    Ok(StateDiagram {
        states: ctx.states,
        transitions: ctx.transitions,
        notes: ctx.notes,
        class_defs: ctx.class_defs,
        style_stmts: ctx.style_stmts,
        direction: ctx.direction,
    })
}

fn header(input: &mut &str) -> ModalResult<()> {
    alt(("stateDiagram-v2".void(), "stateDiagram".void())).parse_next(input)
}

/// Parse statements in the current scope (top-level or inside a composite state).
fn parse_body(input: &mut &str, ctx: &mut ParseCtx, is_top_level: bool) -> ModalResult<()> {
    loop {
        skip.parse_next(input)?;
        if input.is_empty() || input.starts_with('}') {
            break;
        }
        if !try_parse_statement(input, ctx, is_top_level)? && !input.is_empty() {
            // Char-level advance so multi-byte UTF-8 (CJK, `▋`, emoji) can't
            // split a codepoint and panic the str slicer.
            let mut chars = input.chars();
            chars.next();
            *input = chars.as_str();
        }
    }
    Ok(())
}

/// Try to parse a single statement. Returns true if something was parsed.
fn try_parse_statement(
    input: &mut &str,
    ctx: &mut ParseCtx,
    update_direction: bool,
) -> ModalResult<bool> {
    if input.starts_with("classDef") {
        let checkpoint = *input;
        *input = &input[8..];
        if let Ok(cd) = class_def_body.parse_next(input) {
            ctx.class_defs.push(cd);
            return Ok(true);
        }
        *input = checkpoint;
    }

    if input.starts_with("style ") {
        let checkpoint = *input;
        *input = &input[5..];
        if let Ok(ss) = style_stmt_body.parse_next(input) {
            ctx.style_stmts.push(ss);
            return Ok(true);
        }
        *input = checkpoint;
    }

    if input.starts_with("class ") {
        let checkpoint = *input;
        *input = &input[5..];
        if let Ok(ca) = class_apply_body.parse_next(input) {
            for id in &ca.ids {
                if let Some(s) = ctx.states.iter_mut().find(|s| s.id == *id) {
                    s.classes.push(ca.class_name.clone());
                }
            }
            return Ok(true);
        }
        *input = checkpoint;
    }

    if let Some(dir) = opt(parse_direction_stmt).parse_next(input)? {
        if update_direction {
            ctx.direction = dir;
        }
        return Ok(true);
    }

    if let Some(note) = opt(parse_note).parse_next(input)? {
        ctx.notes.push(note);
        return Ok(true);
    }

    {
        let checkpoint = *input;
        if let Ok(node) = parse_composite_state(input, ctx) {
            upsert_state_full(&mut ctx.states, node);
            return Ok(true);
        }
        *input = checkpoint;
    }

    if let Some(node) = opt(parse_state_decl).parse_next(input)? {
        upsert_state_full(&mut ctx.states, node);
        return Ok(true);
    }

    if let Some(trans) = opt(parse_transition).parse_next(input)? {
        ensure_state(&mut ctx.states, &trans.src);
        ensure_state(&mut ctx.states, &trans.dst);
        ctx.transitions.push(trans);
        return Ok(true);
    }

    if let Some(node) = opt(parse_state_label).parse_next(input)? {
        upsert_state(&mut ctx.states, node);
        return Ok(true);
    }

    Ok(false)
}

/// Ensure a state exists in the list (auto-create from transitions).
fn ensure_state(states: &mut Vec<StateNode>, id: &str) {
    if id == "[*]" {
        return; // pseudo-states don't need explicit nodes
    }
    if !states.iter().any(|s| s.id == id) {
        states.push(StateNode::new(id, StateKind::Normal));
    }
}

/// Replace an existing placeholder state with a fully-defined one (composite, stereotype),
/// or insert if new. This upgrades auto-created Normal states from transitions.
fn upsert_state_full(states: &mut Vec<StateNode>, node: StateNode) {
    if let Some(existing) = states.iter_mut().find(|s| s.id == node.id) {
        *existing = node;
    } else {
        states.push(node);
    }
}

/// Insert or update a state (e.g., when a label is assigned after first reference).
fn upsert_state(states: &mut Vec<StateNode>, node: StateNode) {
    if let Some(existing) = states.iter_mut().find(|s| s.id == node.id) {
        if node.label.is_some() {
            existing.label = node.label;
        }
    } else {
        states.push(node);
    }
}

/// Parse a state ID — either `[*]` or an identifier.
fn state_id<'i>(input: &mut &'i str) -> ModalResult<&'i str> {
    alt(("[*]", identifier)).parse_next(input)
}

/// Parse `direction LR` / `direction TB` etc.
fn parse_direction_stmt(input: &mut &str) -> ModalResult<Direction> {
    ("direction", ws, parse_direction_keyword)
        .map(|(_, _, d)| d)
        .parse_next(input)
}

fn parse_direction_keyword(input: &mut &str) -> ModalResult<Direction> {
    alt((
        "TB".value(Direction::TB),
        "TD".value(Direction::TB),
        "BT".value(Direction::BT),
        "LR".value(Direction::LR),
        "RL".value(Direction::RL),
    ))
    .parse_next(input)
}

/// Parse `note right of StateId : text` or `note left of StateId : text`.
/// Also handles multi-line notes: `note right of StateId\n  text\nend note`.
fn parse_note(input: &mut &str) -> ModalResult<StateNote> {
    "note".parse_next(input)?;
    ws.parse_next(input)?;
    let position = alt((
        "right".value(NotePosition::Right),
        "left".value(NotePosition::Left),
    ))
    .parse_next(input)?;
    ws.parse_next(input)?;
    "of".parse_next(input)?;
    ws.parse_next(input)?;
    let state_id = state_id.parse_next(input)?;
    ws.parse_next(input)?;

    // Inline note: `: text`
    // Multi-line note: newline ... end note
    let text = if opt(":").parse_next(input)?.is_some() {
        ws.parse_next(input)?;
        let t = take_while(1.., |c: char| c != '\n' && c != '\r').parse_next(input)?;
        t.trim().to_string()
    } else {
        // Multi-line: consume until "end note"
        parse_multiline_note_body(input)?
    };

    Ok(StateNote {
        position,
        state_id: state_id.to_string(),
        text,
    })
}

fn parse_multiline_note_body(input: &mut &str) -> ModalResult<String> {
    let mut lines = Vec::new();
    loop {
        // Skip to next line
        take_while(0.., |c: char| {
            c == '\n' || c == '\r' || c == ' ' || c == '\t'
        })
        .parse_next(input)?;
        if input.starts_with("end note") {
            "end note".parse_next(input)?;
            break;
        }
        if input.is_empty() {
            break;
        }
        let line = take_while(1.., |c: char| c != '\n' && c != '\r').parse_next(input)?;
        lines.push(line.trim());
    }
    Ok(lines.join("\n"))
}

/// Parse `state "Label" as Name {` or `state Name {`.
fn parse_composite_state(input: &mut &str, outer_ctx: &mut ParseCtx) -> ModalResult<StateNode> {
    "state".parse_next(input)?;
    ws.parse_next(input)?;

    let (label, id) = if input.starts_with('"') {
        '"'.parse_next(input)?;
        let label = take_while(1.., |c: char| c != '"').parse_next(input)?;
        '"'.parse_next(input)?;
        ws.parse_next(input)?;
        "as".parse_next(input)?;
        ws.parse_next(input)?;
        let id = identifier.parse_next(input)?;
        (Some(unescape_unicode(label)), id.to_string())
    } else {
        let id = identifier.parse_next(input)?;
        (None, id.to_string())
    };

    ws.parse_next(input)?;
    '{'.parse_next(input)?;

    // Child context shares class_defs/style_stmts with outer, but has own states/transitions
    let mut inner_ctx = ParseCtx {
        states: Vec::new(),
        transitions: Vec::new(),
        notes: Vec::new(),
        class_defs: std::mem::take(&mut outer_ctx.class_defs),
        style_stmts: std::mem::take(&mut outer_ctx.style_stmts),
        direction: Direction::TB,
    };

    let mut regions: Vec<ConcurrentRegion> = Vec::new();
    let mut region_children: Vec<StateNode> = Vec::new();
    let mut region_trans: Vec<StateTransition> = Vec::new();
    let mut has_divider = false;
    let mut composite_direction = None;

    loop {
        skip.parse_next(input)?;
        if input.is_empty() {
            break;
        }
        if opt('}').parse_next(input)?.is_some() {
            break;
        }

        if input.starts_with("--") && !input.starts_with("-->") {
            "--".parse_next(input)?;
            take_while(0.., |c: char| c == '-').parse_next(input)?;
            regions.push(ConcurrentRegion {
                children: std::mem::take(&mut region_children),
                transitions: std::mem::take(&mut region_trans),
            });
            has_divider = true;
            continue;
        }

        if let Some(dir) = opt(parse_direction_stmt).parse_next(input)? {
            composite_direction = Some(dir);
            continue;
        }

        // Parse into region buffers via inner ctx
        inner_ctx.states = std::mem::take(&mut region_children);
        inner_ctx.transitions = std::mem::take(&mut region_trans);
        if !try_parse_statement(input, &mut inner_ctx, false)? && !input.is_empty() {
            // Char-safe advance; see parse_body().
            let mut chars = input.chars();
            chars.next();
            *input = chars.as_str();
        }
        region_children = std::mem::take(&mut inner_ctx.states);
        region_trans = std::mem::take(&mut inner_ctx.transitions);
    }

    // Restore shared collections to outer ctx
    outer_ctx.class_defs = std::mem::take(&mut inner_ctx.class_defs);
    outer_ctx.style_stmts = std::mem::take(&mut inner_ctx.style_stmts);

    let (children, trans, notes);
    if has_divider {
        regions.push(ConcurrentRegion {
            children: std::mem::take(&mut region_children),
            transitions: std::mem::take(&mut region_trans),
        });
        let mut c = Vec::new();
        let mut t = Vec::new();
        for r in &regions {
            c.extend(r.children.clone());
            t.extend(r.transitions.clone());
        }
        children = c;
        trans = t;
        notes = inner_ctx.notes;
    } else {
        children = region_children;
        trans = region_trans;
        notes = inner_ctx.notes;
    }

    let node = StateNode {
        label,
        ..StateNode::new(
            id,
            StateKind::Composite {
                direction: composite_direction,
                children,
                transitions: trans,
                notes,
                regions,
            },
        )
    };
    Ok(node)
}

/// Parse `state Name <<fork>>`, `state Name <<join>>`, `state Name <<choice>>`.
fn parse_state_decl(input: &mut &str) -> ModalResult<StateNode> {
    "state".parse_next(input)?;
    ws.parse_next(input)?;

    // Optional quoted label + "as"
    let (label, id) = if input.starts_with('"') {
        '"'.parse_next(input)?;
        let label = take_while(1.., |c: char| c != '"').parse_next(input)?;
        '"'.parse_next(input)?;
        ws.parse_next(input)?;
        "as".parse_next(input)?;
        ws.parse_next(input)?;
        let id = identifier.parse_next(input)?;
        (Some(unescape_unicode(label)), id.to_string())
    } else {
        let id = identifier.parse_next(input)?;
        (None, id.to_string())
    };

    ws.parse_next(input)?;

    let kind = alt((
        "<<fork>>".value(StateKind::Fork),
        "<<join>>".value(StateKind::Join),
        "<<choice>>".value(StateKind::Choice),
        "<<history>>".value(StateKind::History),
    ))
    .parse_next(input)?;

    let node = StateNode {
        label,
        ..StateNode::new(id, kind)
    };
    Ok(node)
}

/// Parse `Name : description`.
fn parse_state_label(input: &mut &str) -> ModalResult<StateNode> {
    let id = identifier.parse_next(input)?;
    ws.parse_next(input)?;
    ':'.parse_next(input)?;
    ws.parse_next(input)?;
    let label = take_while(1.., |c: char| c != '\n' && c != '\r').parse_next(input)?;
    Ok(StateNode::new(id, StateKind::Normal).with_label(unescape_unicode(label.trim())))
}

/// Parse `A --> B` or `A --> B : label`.
fn parse_transition(input: &mut &str) -> ModalResult<StateTransition> {
    let src = state_id.parse_next(input)?;
    ws.parse_next(input)?;
    "-->".parse_next(input)?;
    ws.parse_next(input)?;
    let dst = state_id.parse_next(input)?;

    let label = opt(parse_transition_label).parse_next(input)?;

    Ok(StateTransition {
        src: src.to_string(),
        dst: dst.to_string(),
        label: label.map(unescape_unicode),
    })
}

fn parse_transition_label<'i>(input: &mut &'i str) -> ModalResult<&'i str> {
    ws.parse_next(input)?;
    ':'.parse_next(input)?;
    ws.parse_next(input)?;
    let label = take_while(1.., |c: char| c != '\n' && c != '\r').parse_next(input)?;
    Ok(label.trim())
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod parser_tests;
