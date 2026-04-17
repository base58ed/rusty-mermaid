use rusty_mermaid_core::{Direction, Shape};
use winnow::combinator::{alt, opt};
use winnow::prelude::*;
use winnow::token::{any, take_while};

use crate::common::error::{ParseError, ParseErrorKind};
use crate::common::styling::{class_apply_body, class_def_body, style_properties, style_stmt_body};
use crate::common::tokens::{
    direction, node_id, quoted_string, skip, style_class, text_until, unescape_unicode, ws,
};

use super::ir::*;

/// Parse a complete flowchart from mermaid text.
pub fn parse(input: &str) -> Result<FlowDiagram, ParseError> {
    let mut rest = input;
    parse_flowchart(&mut rest).map_err(|_| {
        let offset = input.len() - rest.len();
        ParseError::new(ParseErrorKind::UnexpectedToken, offset..offset, input)
    })
}

/// Top-level: `graph DIR` or `flowchart DIR`, then statements.
fn parse_flowchart(input: &mut &str) -> ModalResult<FlowDiagram> {
    skip.parse_next(input)?;
    let dir = header(input)?;
    let mut diagram = FlowDiagram::new(dir);

    skip.parse_next(input)?;
    parse_statements(input, &mut diagram, None)?;

    Ok(diagram)
}

/// Parse the header: `graph TD` or `flowchart LR`.
fn header(input: &mut &str) -> ModalResult<Direction> {
    let _keyword = alt(("flowchart", "graph")).parse_next(input)?;
    ws.parse_next(input)?;
    direction(input)
}

/// Parse statements until EOF or `end` keyword.
/// If `subgraph_id` is Some, we're inside a subgraph and stop at `end`.
fn parse_statements(
    input: &mut &str,
    diagram: &mut FlowDiagram,
    subgraph_id: Option<&str>,
) -> ModalResult<()> {
    loop {
        skip.parse_next(input)?;
        if input.is_empty() {
            return Ok(());
        }

        // Check for `end` keyword (closes subgraph)
        if subgraph_id.is_some() && input.starts_with("end") {
            let after = &input[3..];
            // `end` must be followed by whitespace, newline, EOF, or comment
            if after.is_empty()
                || after.starts_with(|c: char| c.is_ascii_whitespace())
                || after.starts_with("%%")
            {
                *input = after;
                return Ok(());
            }
        }

        // Try each statement type
        if input.starts_with("subgraph") {
            parse_subgraph(input, diagram, subgraph_id)?;
        } else if input.starts_with("classDef") {
            *input = &input[8..];
            let cd = class_def_body.parse_next(input)?;
            diagram.class_defs.push(cd);
        } else if input.starts_with("style ") {
            *input = &input[5..];
            let ss = style_stmt_body.parse_next(input)?;
            diagram.style_stmts.push(FlowStyleStmt {
                ids: ss.ids,
                styles: ss.styles,
            });
        } else if input.starts_with("class ") {
            *input = &input[5..];
            let ca = class_apply_body.parse_next(input)?;
            // Apply classes to vertices
            for id in &ca.ids {
                if let Some(v) = diagram.vertices.iter_mut().find(|v| v.id == *id) {
                    v.classes.push(ca.class_name.clone());
                }
            }
        } else if input.starts_with("linkStyle") {
            *input = &input[9..];
            ws.parse_next(input)?;
            let ls = parse_link_style_body(input)?;
            diagram.link_styles.push(ls);
        } else if input.starts_with("direction") {
            *input = &input[9..];
            ws.parse_next(input)?;
            let dir = direction(input)?;
            if let Some(sg_id) = subgraph_id
                && let Some(sg) = diagram.subgraphs.iter_mut().find(|s| s.id == sg_id)
            {
                sg.direction = Some(dir);
            }
        } else {
            // Must be a node/edge statement
            parse_node_edge_statement(input, diagram, subgraph_id)?;
        }
    }
}

/// Parse `subgraph id[Label] ... end` or `subgraph Title ... end`.
fn parse_subgraph(
    input: &mut &str,
    diagram: &mut FlowDiagram,
    parent_sg: Option<&str>,
) -> ModalResult<()> {
    // Consume "subgraph"
    "subgraph".parse_next(input)?;
    ws.parse_next(input)?;

    // Parse subgraph ID and optional label
    let (sg_id, sg_label) = parse_subgraph_header(input)?;

    let sg = FlowSubGraph {
        id: sg_id.clone(),
        label: sg_label,
        direction: None,
        node_ids: Vec::new(),
        subgraph_ids: Vec::new(),
    };
    diagram.subgraphs.push(sg);

    // Register this subgraph as a child of parent
    if let Some(parent) = parent_sg
        && let Some(p) = diagram.subgraphs.iter_mut().find(|s| s.id == parent)
    {
        p.subgraph_ids.push(sg_id.clone());
    }

    // Parse inner statements
    parse_statements(input, diagram, Some(&sg_id))?;

    Ok(())
}

/// Parse subgraph header: `id[Label]`, `id["Label"]`, or just `Title Text`.
fn parse_subgraph_header(input: &mut &str) -> ModalResult<(String, Option<String>)> {
    // Try: identifier followed by [label]
    let checkpoint = *input;
    if let Ok(id) = node_id.parse_next(input) {
        // Must check `["label"]` BEFORE `[label]`: the latter is a prefix of
        // the former, so writing the bare-bracket branch first makes the
        // quoted branch dead code and lets literal `"` leak into the label.
        if input.starts_with("[\"") {
            *input = &input[1..]; // consume `[`
            let label = quoted_string(input)?;
            ']'.parse_next(input)?;
            return Ok((id.to_string(), Some(label.to_string())));
        }
        if input.starts_with('[') {
            *input = &input[1..];
            let label = text_until(']', input)?;
            ']'.parse_next(input)?;
            // Belt-and-braces: if the user wrote `id[bare "text"]` — no outer
            // quoting bracket but the content is still `"..."` — strip it.
            return Ok((id.to_string(), Some(strip_outer_quotes(label))));
        }
        // No bracket — could be `subgraph Title Text`
        // Check if the rest of the line (before newline) is more text
        let remaining = take_while(0.., |c: char| c != '\n' && c != '\r').parse_next(input)?;
        let full_title = format!("{}{}", id, remaining).trim().to_string();
        // Use a sanitized version as ID
        let sg_id = full_title.replace(' ', "_");
        return Ok((sg_id, Some(full_title)));
    }

    // Fallback: quoted string as title
    *input = checkpoint;
    if let Ok(title) = quoted_string(input) {
        let sg_id = title.replace(' ', "_");
        return Ok((sg_id, Some(title.to_string())));
    }

    Err(winnow::error::ErrMode::Backtrack(
        winnow::error::ContextError::new(),
    ))
}

/// Parse a node/edge statement. Supports:
///   - single edge: `A --> B`
///   - chained edges: `A --> B --> C`
///   - group sources / targets via `&`: `A & B --> C & D` (emits A→C, A→D,
///     B→C, B→D). Chains retain their "current group" so you can write
///     `A & B --> C --> D & E`.
fn parse_node_edge_statement(
    input: &mut &str,
    diagram: &mut FlowDiagram,
    subgraph_id: Option<&str>,
) -> ModalResult<()> {
    // Parse a group of nodes joined by `&` (at least one).
    let mut prev_group = parse_node_group(input, diagram, subgraph_id)?;

    loop {
        ws.parse_next(input)?;
        let checkpoint = *input;
        if let Ok((label, stroke, start_arrow, end_arrow, minlen)) = parse_edge_operator(input) {
            ws.parse_next(input)?;
            let next_group = parse_node_group(input, diagram, subgraph_id)?;
            // Cartesian product: every src in prev_group connects to every
            // dst in next_group.
            for src in &prev_group {
                for dst in &next_group {
                    diagram.edges.push(FlowEdge {
                        src: src.clone(),
                        dst: dst.clone(),
                        label: label.clone(),
                        stroke,
                        start_arrow,
                        end_arrow,
                        minlen,
                    });
                }
            }
            prev_group = next_group;
        } else {
            *input = checkpoint;
            break;
        }
    }

    Ok(())
}

/// Parse a `&`-joined group of node refs: `A`, `A & B`, `A & B & C`.
fn parse_node_group(
    input: &mut &str,
    diagram: &mut FlowDiagram,
    subgraph_id: Option<&str>,
) -> ModalResult<Vec<String>> {
    let first = parse_node_ref(input, diagram, subgraph_id)?;
    let mut group = vec![first];
    loop {
        let checkpoint = *input;
        // Leading whitespace before `&` is optional; mermaid allows both
        // `A&B` and `A & B`.
        let _ = ws.parse_next(input);
        if input.starts_with('&') {
            *input = &input[1..];
            let _ = ws.parse_next(input);
            match parse_node_ref(input, diagram, subgraph_id) {
                Ok(id) => group.push(id),
                Err(_) => {
                    *input = checkpoint;
                    break;
                }
            }
        } else {
            *input = checkpoint;
            break;
        }
    }
    Ok(group)
}

/// Parse a node reference: `A`, `A[Label]`, `A{Label}`, `A[(Label)]`, etc.
/// Adds/updates the vertex in the diagram and returns the node ID.
fn parse_node_ref(
    input: &mut &str,
    diagram: &mut FlowDiagram,
    subgraph_id: Option<&str>,
) -> ModalResult<String> {
    let id = node_id(input)?;
    let id_str = id.to_string();

    // Try to parse a shape+label
    let shape_label = parse_node_shape(input);

    // Parse optional :::className
    let class = opt(style_class).parse_next(input)?;

    if let Ok((shape, raw_label)) = shape_label {
        let label = unescape_unicode(&raw_label);
        // Add or update vertex
        if let Some(v) = diagram.vertices.iter_mut().find(|v| v.id == id_str) {
            // Update label/shape if redefined
            v.label = label.clone();
            v.shape = shape;
        } else {
            let mut v = FlowVertex::new(&id_str, &label, shape);
            if let Some(c) = class {
                v.classes.push(c.to_string());
            }
            diagram.vertices.push(v);
        }
    } else if diagram.vertex(&id_str).is_none() {
        // Node referenced without shape — default to Rect with ID as label
        let mut v = FlowVertex::new(&id_str, &id_str, Shape::Rect);
        if let Some(c) = class {
            v.classes.push(c.to_string());
        }
        diagram.vertices.push(v);
    }

    // Register in subgraph
    if let Some(sg_id) = subgraph_id
        && let Some(sg) = diagram.subgraphs.iter_mut().find(|s| s.id == sg_id)
        && !sg.node_ids.contains(&id_str)
    {
        sg.node_ids.push(id_str.clone());
    }

    Ok(id_str)
}

/// Parse node shape delimiter and label text. Returns (Shape, label).
fn parse_node_shape(input: &mut &str) -> ModalResult<(Shape, String)> {
    let c = input
        .chars()
        .next()
        .ok_or_else(|| winnow::error::ErrMode::Backtrack(winnow::error::ContextError::new()))?;

    match c {
        '[' => {
            *input = &input[1..];
            // Check for special multi-char openers: [( )] cylinder, [[ ]] subroutine
            if input.starts_with('(') {
                *input = &input[1..];
                let label = text_until(')', input)?;
                ")]".parse_next(input)?;
                Ok((Shape::Cylinder, strip_outer_quotes(label)))
            } else if input.starts_with('[') {
                *input = &input[1..];
                let label = text_until(']', input)?;
                "]]".parse_next(input)?;
                Ok((Shape::Subroutine, strip_outer_quotes(label)))
            } else if input.starts_with('/') {
                // Trapezoid [/text\] or lean right [/text/]
                *input = &input[1..];
                let label = text_until_trap(input)?;
                Ok((Shape::Trapezoid, strip_outer_quotes(&label)))
            } else if input.starts_with('\\') {
                // Inv trapezoid [\text/] or lean left [\text\]
                *input = &input[1..];
                let label = text_until_trap(input)?;
                Ok((Shape::TrapezoidAlt, strip_outer_quotes(&label)))
            } else {
                // Regular rect [text] or quoted ["text"]
                let label = if input.starts_with('"') {
                    let s = quoted_string(input)?;
                    ']'.parse_next(input)?;
                    s.to_string()
                } else {
                    let s = text_until(']', input)?;
                    ']'.parse_next(input)?;
                    s.to_string()
                };
                Ok((Shape::Rect, label))
            }
        }
        '(' => {
            *input = &input[1..];
            if input.starts_with('[') {
                // Stadium ([text])
                *input = &input[1..];
                let label = text_until(']', input)?;
                "])".parse_next(input)?;
                Ok((Shape::Stadium, strip_outer_quotes(label)))
            } else if input.starts_with('(') {
                // Circle ((text)) or double circle (((text)))
                *input = &input[1..];
                if input.starts_with('(') {
                    *input = &input[1..];
                    let label = text_until(')', input)?;
                    ")))".parse_next(input)?;
                    Ok((Shape::DoubleCircle, strip_outer_quotes(label)))
                } else {
                    let label = text_until(')', input)?;
                    "))".parse_next(input)?;
                    Ok((Shape::Circle, strip_outer_quotes(label)))
                }
            } else {
                // Rounded rect (text)
                let label = text_until(')', input)?;
                ')'.parse_next(input)?;
                Ok((Shape::RoundedRect, strip_outer_quotes(label)))
            }
        }
        '{' => {
            *input = &input[1..];
            if input.starts_with('{') {
                // Hexagon {{text}}
                *input = &input[1..];
                let label = text_until('}', input)?;
                "}}".parse_next(input)?;
                Ok((Shape::Hexagon, strip_outer_quotes(label)))
            } else {
                // Diamond {text}
                let label = text_until('}', input)?;
                '}'.parse_next(input)?;
                Ok((Shape::Diamond, strip_outer_quotes(label)))
            }
        }
        '>' => {
            // Odd shape >text]
            *input = &input[1..];
            let label = text_until(']', input)?;
            ']'.parse_next(input)?;
            Ok((Shape::Asymmetric, strip_outer_quotes(label)))
        }
        _ => Err(winnow::error::ErrMode::Backtrack(
            winnow::error::ContextError::new(),
        )),
    }
}

/// Strip a single pair of surrounding double quotes from a label body.
///
/// Mermaid allows any shape to wrap its label in `"..."` as a quoting
/// mechanism (required when the label contains syntax characters like `]`,
/// `)`, `}`, `,`). The `Rect` branch above handles this explicitly via
/// [`quoted_string`], but every other shape (`Cylinder`, `Stadium`, `Circle`,
/// `Diamond`, `Hexagon`, `RoundedRect`, `Subroutine`, `Asymmetric`, the
/// trapezoids, and the double-circle) just runs `text_until` and so keeps
/// the literal `"` characters in the label — which then get rendered as
/// visible quotes on the node. This helper is applied uniformly at each
/// call site so the behaviour matches `Rect`.
fn strip_outer_quotes(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// Parse trapezoid label text until `\]` or `/]`.
fn text_until_trap(input: &mut &str) -> ModalResult<String> {
    let content = take_while(0.., |c: char| c != '\\' && c != '/' && c != ']').parse_next(input)?;
    // Consume closing: `\]` or `/]`
    any.parse_next(input)?; // `\` or `/`
    ']'.parse_next(input)?;
    Ok(content.to_string())
}

/// Parse an edge operator and optional label.
/// Returns (label, stroke, start_arrow, end_arrow, minlen).
fn parse_edge_operator(
    input: &mut &str,
) -> ModalResult<(Option<String>, StrokeType, ArrowEnd, ArrowEnd, i32)> {
    // Detect start arrow: `<`, `o`, `x`
    let start_arrow = parse_start_arrow(input);

    // Detect stroke type from first chars and compute minlen.
    // Extra dashes/dots/equals beyond the base arrow increase minlen:
    //   Normal: `-->` = 1, `--->` = 2, `---->` = 3
    //   Thick:  `==>` = 1, `===>` = 2, `====>` = 3
    //   Dotted: `-.->` = 1, `-..->` = 2, `-...->` = 3
    let (stroke, label, minlen) = if input.starts_with("-.") {
        // Dotted: `-.->` or `-. text .->`
        *input = &input[2..];
        let label = parse_inline_edge_label(input, ".-")?;
        let tail: &str = take_while(0.., |c: char| c == '.' || c == '-').parse_next(input)?;
        let extra_dots = tail.chars().filter(|&c| c == '.').count() as i32;
        let minlen = if label.is_some() {
            // Labeled: closing `.-` has 1 dot, extras beyond that add length
            1 + extra_dots.saturating_sub(1)
        } else {
            // No label: prefix `-.` has 1 dot, extras in tail add length
            1 + extra_dots
        };
        (StrokeType::Dotted, label, minlen)
    } else if input.starts_with("==") {
        // Thick: `==>` or `== text ==>`
        *input = &input[2..];
        let label = parse_inline_edge_label(input, "=")?;
        let tail: &str = take_while(0.., |c: char| c == '=').parse_next(input)?;
        let minlen = if label.is_some() {
            1 + (tail.len() as i32 - 2).max(0)
        } else {
            1 + tail.len() as i32
        };
        (StrokeType::Thick, label, minlen)
    } else if input.starts_with("--") {
        // Normal: `-->` or `-- text -->` or `---`
        *input = &input[2..];
        let label = parse_inline_edge_label(input, "-")?;
        let tail: &str = take_while(0.., |c: char| c == '-').parse_next(input)?;
        let minlen = if label.is_some() {
            // Labeled: closing `--` is 2 chars, extras add length
            1 + (tail.len() as i32 - 2).max(0)
        } else {
            1 + tail.len() as i32
        };
        (StrokeType::Normal, label, minlen)
    } else {
        return Err(winnow::error::ErrMode::Backtrack(
            winnow::error::ContextError::new(),
        ));
    };

    // End arrow
    let end_arrow = parse_end_arrow(input);

    // Pipe-delimited label: `-->|text|`
    let label = if label.is_some() {
        label
    } else if input.starts_with('|') {
        *input = &input[1..];
        let text = text_until('|', input)?;
        '|'.parse_next(input)?;
        Some(text.to_string())
    } else {
        None
    };

    let label = label.map(|l| unescape_unicode(&l));

    Ok((label, stroke, start_arrow, end_arrow, minlen))
}

fn parse_start_arrow(input: &mut &str) -> ArrowEnd {
    if input.starts_with('<') {
        *input = &input[1..];
        ArrowEnd::Arrow
    } else if input.starts_with("o-") || input.starts_with("o=") || input.starts_with("o.") {
        *input = &input[1..];
        ArrowEnd::Circle
    } else if input.starts_with("x-") || input.starts_with("x=") || input.starts_with("x.") {
        *input = &input[1..];
        ArrowEnd::Cross
    } else {
        ArrowEnd::None
    }
}

fn parse_end_arrow(input: &mut &str) -> ArrowEnd {
    if input.starts_with('>') {
        *input = &input[1..];
        ArrowEnd::Arrow
    } else if input.starts_with('x') {
        *input = &input[1..];
        ArrowEnd::Cross
    } else if input.starts_with('o') {
        *input = &input[1..];
        ArrowEnd::Circle
    } else {
        ArrowEnd::None
    }
}

/// Try to parse an inline edge label: `-- text -->` (text between dashes).
/// Returns None if there's no label (just more dashes/dots/equals).
/// `stop_chars` contains characters that signal "no label, continue to arrow".
fn parse_inline_edge_label(input: &mut &str, stop_chars: &str) -> ModalResult<Option<String>> {
    if input.is_empty() {
        return Ok(None);
    }
    let Some(next) = input.chars().next() else {
        return Ok(None);
    };
    // If next char is a stop char or arrow endpoint, no label
    if stop_chars.contains(next) || next == '>' || next == 'x' || next == 'o' {
        return Ok(None);
    }

    // There's a label: consume until we hit a stop char
    ws.parse_next(input)?;
    let mut label = String::new();
    while !input.is_empty() {
        let Some(c) = input.chars().next() else { break };
        if stop_chars.contains(c) {
            break;
        }
        label.push(c);
        *input = &input[c.len_utf8()..];
    }
    let label = label.trim().to_string();
    if label.is_empty() {
        Ok(None)
    } else {
        Ok(Some(label))
    }
}

/// Parse `linkStyle` body: `default stroke:green` or `0,1,2 stroke:#f00`.
/// The `linkStyle` keyword and leading whitespace are already consumed.
fn parse_link_style_body(input: &mut &str) -> ModalResult<FlowLinkStyle> {
    if input.starts_with("default") {
        *input = &input[7..];
        ws.parse_next(input)?;
        let styles = style_properties(input)?;
        Ok(FlowLinkStyle {
            indices: Vec::new(),
            is_default: true,
            styles,
        })
    } else {
        // Parse comma-separated indices
        let idx_str: &str =
            take_while(1.., |c: char| c.is_ascii_digit() || c == ',').parse_next(input)?;
        let indices: Vec<usize> = idx_str
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        ws.parse_next(input)?;
        let styles = style_properties(input)?;
        Ok(FlowLinkStyle {
            indices,
            is_default: false,
            styles,
        })
    }
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod parser_tests;
