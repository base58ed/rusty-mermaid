use winnow::combinator::{alt, opt};
use winnow::prelude::*;
use winnow::token::take_while;

use rusty_mermaid_core::Direction;

use crate::common::error::{ParseError, ParseErrorKind};
use crate::common::styling::{class_apply_body, class_def_body, style_stmt_body};
use crate::common::tokens::{identifier, skip, ws};

use super::ir::*;
use super::parse_relations::parse_relationship;

/// Parse a mermaid class diagram string into IR.
pub fn parse(input: &str) -> Result<ClassDiagram, ParseError> {
    let mut rest = input;
    parse_class_diagram(&mut rest).map_err(|_| {
        let offset = input.len() - rest.len();
        ParseError::new(ParseErrorKind::UnexpectedToken, offset..offset, input)
    })
}

fn parse_class_diagram(input: &mut &str) -> ModalResult<ClassDiagram> {
    skip.parse_next(input)?;
    header(input)?;
    skip.parse_next(input)?;

    let mut diagram = ClassDiagram::new(Direction::TB);
    parse_statements(input, &mut diagram, None)?;
    Ok(diagram)
}

fn header(input: &mut &str) -> ModalResult<()> {
    alt(("classDiagram-v2", "classDiagram")).parse_next(input)?;
    Ok(())
}

fn parse_statements(
    input: &mut &str,
    diagram: &mut ClassDiagram,
    namespace: Option<&str>,
) -> ModalResult<()> {
    loop {
        skip.parse_next(input)?;
        if input.is_empty() || input.starts_with('}') {
            break;
        }
        if !try_parse_statement(input, diagram, namespace)? && !input.is_empty() {
            // Skip one char (not byte) — `&input[1..]` panics on multi-byte
            // UTF-8 starts (CJK, `▋`, emoji, etc.).
            let mut chars = input.chars();
            chars.next();
            *input = chars.as_str();
        }
    }
    Ok(())
}

fn try_parse_statement(
    input: &mut &str,
    diagram: &mut ClassDiagram,
    namespace: Option<&str>,
) -> ModalResult<bool> {
    // classDef
    if input.starts_with("classDef") {
        let cp = *input;
        *input = &input[8..];
        if let Ok(cd) = class_def_body.parse_next(input) {
            diagram.class_defs.push(cd);
            return Ok(true);
        }
        *input = cp;
    }

    // style statement
    if input.starts_with("style ") {
        let cp = *input;
        *input = &input[5..];
        if let Ok(ss) = style_stmt_body.parse_next(input) {
            diagram.style_stmts.push(ss);
            return Ok(true);
        }
        *input = cp;
    }

    // class apply (cssClass)
    if input.starts_with("class ") && !peek_class_declaration(input) {
        let cp = *input;
        *input = &input[5..];
        if let Ok(ca) = class_apply_body.parse_next(input) {
            for id in &ca.ids {
                ensure_class(&mut diagram.classes, id, namespace);
                if let Some(c) = diagram.classes.iter_mut().find(|c| c.id == *id) {
                    c.css_classes.push(ca.class_name.clone());
                }
            }
            return Ok(true);
        }
        *input = cp;
    }

    // direction
    if input.starts_with("direction") {
        let cp = *input;
        *input = &input[9..];
        if ws.parse_next(input).is_ok()
            && let Ok(dir) = parse_direction(input)
        {
            diagram.direction = dir;
            return Ok(true);
        }
        *input = cp;
    }

    // namespace
    if input.starts_with("namespace") {
        let cp = *input;
        if parse_namespace(input, diagram).is_ok() {
            return Ok(true);
        }
        *input = cp;
    }

    // note
    if input.starts_with("note") {
        let cp = *input;
        if let Ok(note) = parse_note(input) {
            diagram.notes.push(note);
            return Ok(true);
        }
        *input = cp;
    }

    // annotation: <<interface>> ClassName
    if input.starts_with("<<") {
        let cp = *input;
        if let Ok((annotation, class_id)) = parse_annotation(input) {
            ensure_class(&mut diagram.classes, &class_id, namespace);
            if let Some(c) = diagram.classes.iter_mut().find(|c| c.id == class_id) {
                c.annotations.push(annotation);
            }
            return Ok(true);
        }
        *input = cp;
    }

    // class declaration: `class Name ...`
    if input.starts_with("class ") {
        let cp = *input;
        if let Ok(mut node) = parse_class_declaration(input) {
            node.namespace = namespace.map(String::from);
            upsert_class(&mut diagram.classes, node);
            return Ok(true);
        }
        *input = cp;
    }

    // relationship: A <|-- B : label
    {
        let cp = *input;
        if let Ok(rel) = parse_relationship(input) {
            ensure_class(&mut diagram.classes, &rel.from_id, namespace);
            ensure_class(&mut diagram.classes, &rel.to_id, namespace);
            diagram.relationships.push(rel);
            return Ok(true);
        }
        *input = cp;
    }

    // colon member: ClassName : member
    {
        let cp = *input;
        if let Ok((class_id, member)) = parse_colon_member(input) {
            ensure_class(&mut diagram.classes, &class_id, namespace);
            if let Some(c) = diagram.classes.iter_mut().find(|c| c.id == class_id) {
                if member.is_method() {
                    c.methods.push(member);
                } else {
                    c.members.push(member);
                }
            }
            return Ok(true);
        }
        *input = cp;
    }

    Ok(false)
}

/// Peek: is this `class Name` (declaration) vs `class id1,id2 className` (apply)?
fn peek_class_declaration(input: &str) -> bool {
    let rest = input.strip_prefix("class ").unwrap_or("");
    let rest = rest.trim_start();
    // Declaration starts with identifier optionally followed by ~, [, {, <<, or newline
    // Apply starts with identifier(s) followed by a class name (no special chars)
    // Heuristic: if after first identifier there's ~, [, {, <<, newline, or EOF → declaration
    let end = rest
        .find(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .unwrap_or(rest.len());
    if end >= rest.len() {
        return true;
    }
    let next = rest[end..].trim_start();
    next.is_empty()
        || next.starts_with('~')
        || next.starts_with('[')
        || next.starts_with('{')
        || next.starts_with("<<")
        || next.starts_with('\n')
        || next.starts_with('\r')
        || next.starts_with(":::")
}

// ── Class declaration ──

fn parse_class_declaration(input: &mut &str) -> ModalResult<ClassNode> {
    "class".parse_next(input)?;
    ws.parse_next(input)?;

    let id = class_identifier(input)?;
    let mut node = ClassNode::new(id);

    // Optional generic: ~T~ or ~KeyType, ValueType~
    if input.starts_with('~') {
        '~'.parse_next(input)?;
        let generic = take_while(1.., |c: char| c != '~').parse_next(input)?;
        '~'.parse_next(input)?;
        node.generic_type = Some(generic.to_string());
    }

    // Optional label: ["Display Label"]
    skip_horizontal_ws(input);
    if input.starts_with("[\"") {
        "[\"".parse_next(input)?;
        let label = take_while(0.., |c: char| c != '"').parse_next(input)?;
        "\"]".parse_next(input)?;
        node.label = Some(label.to_string());
    }

    // Optional inline annotation: <<interface>>
    skip_horizontal_ws(input);
    if input.starts_with("<<") {
        "<<".parse_next(input)?;
        let ann = take_while(1.., |c: char| c != '>').parse_next(input)?;
        ">>".parse_next(input)?;
        node.annotations.push(ann.to_string());
    }

    // Optional :::cssClass
    skip_horizontal_ws(input);
    if input.starts_with(":::") {
        ":::".parse_next(input)?;
        let cls = identifier.parse_next(input)?;
        node.css_classes.push(cls.to_string());
    }

    // Optional body block: { ... }
    skip_horizontal_ws(input);
    if input.starts_with('{') {
        '{'.parse_next(input)?;
        parse_class_body(input, &mut node)?;
        skip.parse_next(input)?;
        opt('}').parse_next(input)?;
    }

    Ok(node)
}

fn parse_class_body(input: &mut &str, node: &mut ClassNode) -> ModalResult<()> {
    loop {
        skip.parse_next(input)?;
        if input.is_empty() || input.starts_with('}') {
            break;
        }

        // Annotation inside body
        if input.starts_with("<<") {
            "<<".parse_next(input)?;
            let ann = take_while(1.., |c: char| c != '>').parse_next(input)?;
            ">>".parse_next(input)?;
            node.annotations.push(ann.to_string());
            continue;
        }

        // Separator lines: --, .., ==, __
        if is_separator_line(input) {
            skip_to_eol(input);
            continue;
        }

        // Member line
        let line = take_line(input);
        if line.is_empty() {
            continue;
        }
        let member = parse_member_string(line);
        if member.is_method() {
            node.methods.push(member);
        } else {
            node.members.push(member);
        }
    }
    Ok(())
}

fn is_separator_line(input: &str) -> bool {
    input.starts_with("--") && !input.starts_with("-->")
        || input.starts_with("..")
        || input.starts_with("==")
        || input.starts_with("__")
}

// ── Member parsing ──

/// Parse a member string like `+getName(id int) String` or `-count int`.
pub fn parse_member_string(raw: &str) -> ClassMember {
    let s = raw.trim();
    if s.is_empty() {
        return ClassMember::attribute("");
    }

    let mut chars = s.chars().peekable();

    // Visibility prefix
    let visibility = chars
        .peek()
        .and_then(|&c| Visibility::from_char(c))
        .unwrap_or(Visibility::None);
    if visibility != Visibility::None {
        chars.next();
    }

    let rest: String = chars.collect();
    let rest = rest.trim();

    // Classifier suffix
    let (rest, classifier) = if let Some(stripped) = rest.strip_suffix('$') {
        (stripped, Classifier::Static)
    } else if let Some(stripped) = rest.strip_suffix('*') {
        (stripped, Classifier::Abstract)
    } else {
        (rest, Classifier::None)
    };

    // Method: has parentheses
    if let Some(paren_start) = rest.find('(')
        && let Some(paren_end) = rest[paren_start..].find(')')
    {
        let name = rest[..paren_start].trim().to_string();
        let params = rest[paren_start + 1..paren_start + paren_end]
            .trim()
            .to_string();
        let after = rest[paren_start + paren_end + 1..].trim();
        let return_type = if after.is_empty() {
            None
        } else {
            Some(after.to_string())
        };
        return ClassMember {
            name,
            visibility,
            classifier,
            return_type,
            parameters: Some(params),
        };
    }

    // Attribute: may have type before or after name
    // Patterns: `type name`, `name`, `name type`
    // Mermaid convention: last word-like token is the name if there are spaces
    let name = rest.trim().to_string();
    ClassMember {
        name,
        visibility,
        classifier,
        return_type: None,
        parameters: None,
    }
}

// ── Colon member ──

fn parse_colon_member(input: &mut &str) -> ModalResult<(String, ClassMember)> {
    let id = class_identifier(input)?;
    skip_horizontal_ws(input);
    ':'.parse_next(input)?;
    skip_horizontal_ws(input);
    let line = take_to_eol(input);
    if line.is_empty() {
        return Err(winnow::error::ErrMode::Backtrack(
            winnow::error::ContextError::new(),
        ));
    }
    let member = parse_member_string(line);
    Ok((id.to_string(), member))
}

// ── Annotation ──

fn parse_annotation(input: &mut &str) -> ModalResult<(String, String)> {
    "<<".parse_next(input)?;
    let ann = take_while(1.., |c: char| c != '>').parse_next(input)?;
    ">>".parse_next(input)?;
    skip_horizontal_ws(input);
    let id = class_identifier(input)?;
    Ok((ann.to_string(), id.to_string()))
}

// ── Namespace ──

fn parse_namespace(input: &mut &str, diagram: &mut ClassDiagram) -> ModalResult<()> {
    "namespace".parse_next(input)?;
    ws.parse_next(input)?;
    let ns_id = take_while(1.., |c: char| {
        c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
    })
    .parse_next(input)?;
    let ns_id = ns_id.to_string();
    skip.parse_next(input)?;
    '{'.parse_next(input)?;

    let class_count_before = diagram.classes.len();
    parse_statements(input, diagram, Some(&ns_id))?;
    skip.parse_next(input)?;
    opt('}').parse_next(input)?;

    // Collect class IDs that were added inside this namespace
    let class_ids: Vec<String> = diagram.classes[class_count_before..]
        .iter()
        .map(|c| c.id.clone())
        .collect();

    diagram.namespaces.push(Namespace {
        id: ns_id,
        parent: None,
        class_ids,
    });

    Ok(())
}

// ── Note ──

fn parse_note(input: &mut &str) -> ModalResult<ClassNote> {
    "note".parse_next(input)?;
    ws.parse_next(input)?;

    let class_id = if input.starts_with("for ") {
        "for".parse_next(input)?;
        ws.parse_next(input)?;
        let id = class_identifier(input)?;
        skip_horizontal_ws(input);
        Some(id.to_string())
    } else {
        None
    };

    '"'.parse_next(input)?;
    let text = take_while(0.., |c: char| c != '"').parse_next(input)?;
    '"'.parse_next(input)?;

    Ok(ClassNote {
        text: text.to_string(),
        class_id,
    })
}

// ── Direction ──

fn parse_direction(input: &mut &str) -> ModalResult<Direction> {
    alt((
        "TB".value(Direction::TB),
        "TD".value(Direction::TB),
        "BT".value(Direction::BT),
        "LR".value(Direction::LR),
        "RL".value(Direction::RL),
    ))
    .parse_next(input)
}

// ── Helpers ──

pub(super) fn class_identifier<'i>(input: &mut &'i str) -> ModalResult<&'i str> {
    take_while(1.., |c: char| c.is_alphanumeric() || c == '_' || c == '-').parse_next(input)
}

pub(super) fn skip_horizontal_ws(input: &mut &str) {
    *input = input.trim_start_matches([' ', '\t']);
}

pub(super) fn take_to_eol<'i>(input: &mut &'i str) -> &'i str {
    let end = input.find('\n').unwrap_or(input.len());
    let line = &input[..end];
    *input = &input[end..];
    line.trim()
}

fn take_line<'i>(input: &mut &'i str) -> &'i str {
    let end = input.find('\n').unwrap_or(input.len());
    let line = input[..end].trim();
    *input = if end < input.len() {
        &input[end + 1..]
    } else {
        ""
    };
    line
}

fn skip_to_eol(input: &mut &str) {
    let end = input.find('\n').unwrap_or(input.len());
    *input = if end < input.len() {
        &input[end + 1..]
    } else {
        ""
    };
}

fn ensure_class(classes: &mut Vec<ClassNode>, id: &str, namespace: Option<&str>) {
    if !classes.iter().any(|c| c.id == id) {
        let mut node = ClassNode::new(id);
        node.namespace = namespace.map(String::from);
        classes.push(node);
    }
}

fn upsert_class(classes: &mut Vec<ClassNode>, node: ClassNode) {
    if let Some(existing) = classes.iter_mut().find(|c| c.id == node.id) {
        if node.label.is_some() {
            existing.label = node.label;
        }
        if node.generic_type.is_some() {
            existing.generic_type = node.generic_type;
        }
        if !node.annotations.is_empty() {
            existing.annotations.extend(node.annotations);
        }
        if !node.css_classes.is_empty() {
            existing.css_classes.extend(node.css_classes);
        }
        if node.namespace.is_some() {
            existing.namespace = node.namespace;
        }
        existing.members.extend(node.members);
        existing.methods.extend(node.methods);
    } else {
        classes.push(node);
    }
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod parser_tests;
