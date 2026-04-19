use winnow::combinator::{alt, opt};
use winnow::prelude::*;
use winnow::token::take_while;

use rusty_mermaid_core::Direction;

use crate::common::error::{ParseError, ParseErrorKind};
use crate::common::styling::{class_def_body, style_stmt_body};
use crate::common::tokens::{identifier, skip, ws};

use super::ir::*;

/// Parse a mermaid ER diagram string into IR.
pub fn parse(input: &str) -> Result<ErDiagram, ParseError> {
    let mut rest = input;
    parse_er_diagram(&mut rest).map_err(|_| {
        let offset = input.len() - rest.len();
        ParseError::new(ParseErrorKind::UnexpectedToken, offset..offset, input)
    })
}

fn parse_er_diagram(input: &mut &str) -> ModalResult<ErDiagram> {
    skip.parse_next(input)?;
    "erDiagram".parse_next(input)?;
    skip.parse_next(input)?;

    let mut diagram = ErDiagram::new(Direction::TB);
    parse_statements(input, &mut diagram)?;
    Ok(diagram)
}

fn parse_statements(input: &mut &str, diagram: &mut ErDiagram) -> ModalResult<()> {
    loop {
        skip.parse_next(input)?;
        if input.is_empty() {
            break;
        }
        if !try_parse_statement(input, diagram)? && !input.is_empty() {
            // Char-safe advance — skip one Unicode scalar, not one byte.
            let mut chars = input.chars();
            chars.next();
            *input = chars.as_str();
        }
    }
    Ok(())
}

fn try_parse_statement(input: &mut &str, diagram: &mut ErDiagram) -> ModalResult<bool> {
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

    // Try relationship first (entity CARD -- CARD entity : label)
    {
        let cp = *input;
        if let Ok(rel) = parse_relationship(input) {
            ensure_entity(&mut diagram.entities, &rel.entity_a);
            ensure_entity(&mut diagram.entities, &rel.entity_b);
            diagram.relationships.push(rel);
            return Ok(true);
        }
        *input = cp;
    }

    // Entity with attributes: ENTITY { ... }
    {
        let cp = *input;
        if let Ok(entity) = parse_entity_block(input) {
            upsert_entity(&mut diagram.entities, entity);
            return Ok(true);
        }
        *input = cp;
    }

    // Bare entity name (just declares it exists)
    {
        let cp = *input;
        if let Ok(id) = entity_identifier(input) {
            // Check it's followed by newline/EOF (not a relationship)
            let rest_trimmed = input.trim_start_matches([' ', '\t']);
            if rest_trimmed.is_empty()
                || rest_trimmed.starts_with('\n')
                || rest_trimmed.starts_with("%%")
            {
                ensure_entity(&mut diagram.entities, id);
                return Ok(true);
            }
        }
        *input = cp;
    }

    Ok(false)
}

// ── Entity block ──

fn parse_entity_block(input: &mut &str) -> ModalResult<Entity> {
    let id = entity_identifier(input)?;
    let mut entity = Entity::new(id);

    // Optional alias: [Display Name]
    skip_horizontal_ws(input);
    if input.starts_with('[') {
        *input = &input[1..];
        let alias = take_while(0.., |c: char| c != ']').parse_next(input)?;
        ']'.parse_next(input)?;
        entity.alias = Some(alias.to_string());
    }

    // Optional :::cssClass
    skip_horizontal_ws(input);
    if input.starts_with(":::") {
        *input = &input[3..];
        let cls = identifier.parse_next(input)?;
        entity.css_classes.push(cls.to_string());
    }

    skip_horizontal_ws(input);
    '{'.parse_next(input)?;

    // Parse attributes
    loop {
        skip.parse_next(input)?;
        if input.is_empty() || input.starts_with('}') {
            break;
        }
        if let Some(attr) = parse_attribute(input) {
            entity.attributes.push(attr);
        } else {
            skip_to_eol(input);
        }
    }
    opt('}').parse_next(input)?;

    Ok(entity)
}

fn parse_attribute(input: &mut &str) -> Option<Attribute> {
    let line = take_line(input);
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let mut parts = line.splitn(2, [' ', '\t']);
    let attr_type = parts.next()?.trim().to_string();
    let rest = parts.next().unwrap_or("").trim();

    if rest.is_empty() {
        return Some(Attribute {
            attr_type,
            name: String::new(),
            keys: Vec::new(),
            comment: None,
        });
    }

    // Parse: name [PK[,FK[,UK]]] ["comment"]
    let mut tokens: Vec<&str> = Vec::new();
    // Extract quoted comment first
    let (main_part, cmt) = if let Some(q_start) = rest.find('"') {
        let after = &rest[q_start + 1..];
        if let Some(q_end) = after.find('"') {
            let comment = Some(after[..q_end].to_string());
            (rest[..q_start].trim(), comment)
        } else {
            (rest, None)
        }
    } else {
        (rest, None)
    };

    tokens.extend(main_part.split_whitespace());

    let name = tokens.first().map(|s| s.to_string()).unwrap_or_default();

    // Parse keys from remaining tokens
    let mut keys = Vec::new();
    for token in &tokens[1..] {
        for part in token.split(',') {
            let part = part.trim();
            match part {
                "PK" => keys.push(KeyType::PrimaryKey),
                "FK" => keys.push(KeyType::ForeignKey),
                "UK" => keys.push(KeyType::UniqueKey),
                _ => {}
            }
        }
    }

    Some(Attribute {
        attr_type,
        name,
        keys,
        comment: cmt,
    })
}

// ── Relationship ──

fn parse_relationship(input: &mut &str) -> ModalResult<ErRelation> {
    let entity_a = entity_identifier(input)?;
    skip_horizontal_ws(input);

    let cardinality_a = parse_cardinality_left(input)?;
    let identification = parse_identification(input)?;
    let cardinality_b = parse_cardinality_right(input)?;

    skip_horizontal_ws(input);
    let entity_b = entity_identifier(input)?;

    // Optional label after colon
    skip_horizontal_ws(input);
    let label = if input.starts_with(':') {
        *input = &input[1..];
        skip_horizontal_ws(input);
        let text = take_to_eol(input);
        if text.is_empty() {
            None
        } else {
            Some(strip_quotes(text))
        }
    } else {
        None
    };

    Ok(ErRelation {
        entity_a: entity_a.to_string(),
        entity_b: entity_b.to_string(),
        cardinality_a,
        cardinality_b,
        identification,
        label,
    })
}

fn parse_cardinality_left(input: &mut &str) -> ModalResult<Cardinality> {
    // Text aliases (must check before symbols)
    if let Some(card) = try_text_cardinality(input) {
        return Ok(card);
    }
    // Symbol syntax (left side): }|, }o, ||, o|
    if input.starts_with("}|") {
        *input = &input[2..];
        Ok(Cardinality::OneOrMore)
    } else if input.starts_with("}o") {
        *input = &input[2..];
        Ok(Cardinality::ZeroOrMore)
    } else if input.starts_with("||") {
        *input = &input[2..];
        Ok(Cardinality::ExactlyOne)
    } else if input.starts_with("o|") {
        *input = &input[2..];
        Ok(Cardinality::ZeroOrOne)
    } else {
        Err(winnow::error::ErrMode::Backtrack(
            winnow::error::ContextError::new(),
        ))
    }
}

fn parse_cardinality_right(input: &mut &str) -> ModalResult<Cardinality> {
    // Text aliases
    if let Some(card) = try_text_cardinality(input) {
        return Ok(card);
    }
    // Symbol syntax (right side): |{, o{, ||, o|
    if input.starts_with("|{") {
        *input = &input[2..];
        Ok(Cardinality::OneOrMore)
    } else if input.starts_with("o{") {
        *input = &input[2..];
        Ok(Cardinality::ZeroOrMore)
    } else if input.starts_with("||") {
        *input = &input[2..];
        Ok(Cardinality::ExactlyOne)
    } else if input.starts_with("o|") {
        *input = &input[2..];
        Ok(Cardinality::ZeroOrOne)
    } else {
        Err(winnow::error::ErrMode::Backtrack(
            winnow::error::ContextError::new(),
        ))
    }
}

fn try_text_cardinality(input: &mut &str) -> Option<Cardinality> {
    let trimmed = input.trim_start();
    for (prefix, card) in [
        ("one or zero", Cardinality::ZeroOrOne),
        ("zero or one", Cardinality::ZeroOrOne),
        ("one or more", Cardinality::OneOrMore),
        ("one or many", Cardinality::OneOrMore),
        ("zero or more", Cardinality::ZeroOrMore),
        ("zero or many", Cardinality::ZeroOrMore),
        ("only one", Cardinality::ExactlyOne),
        ("many(1)", Cardinality::OneOrMore),
        ("many(0)", Cardinality::ZeroOrMore),
        ("many", Cardinality::ZeroOrMore),
        ("1+", Cardinality::OneOrMore),
        ("0+", Cardinality::ZeroOrMore),
    ] {
        if let Some(after) = trimmed.strip_prefix(prefix)
            && (after.is_empty() || after.starts_with([' ', '\t', '-', '.']))
        {
            let consumed = input.len() - trimmed.len() + prefix.len();
            *input = &input[consumed..];
            return Some(card);
        }
    }
    None
}

fn parse_identification(input: &mut &str) -> ModalResult<Identification> {
    // Text aliases
    let trimmed = input.trim_start();
    if trimmed.starts_with("optionally to") {
        let consumed = input.len() - trimmed.len() + "optionally to".len();
        *input = &input[consumed..];
        return Ok(Identification::NonIdentifying);
    }
    if trimmed.starts_with("to") {
        let consumed = input.len() - trimmed.len() + "to".len();
        *input = &input[consumed..];
        return Ok(Identification::Identifying);
    }
    // Symbol syntax
    if input.starts_with("--") {
        *input = &input[2..];
        Ok(Identification::Identifying)
    } else if input.starts_with("..") || input.starts_with(".-") || input.starts_with("-.") {
        *input = &input[2..];
        Ok(Identification::NonIdentifying)
    } else {
        Err(winnow::error::ErrMode::Backtrack(
            winnow::error::ContextError::new(),
        ))
    }
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

fn entity_identifier<'i>(input: &mut &'i str) -> ModalResult<&'i str> {
    take_while(1.., |c: char| c.is_alphanumeric() || c == '_' || c == '-').parse_next(input)
}

fn skip_horizontal_ws(input: &mut &str) {
    *input = input.trim_start_matches([' ', '\t']);
}

fn take_to_eol<'i>(input: &mut &'i str) -> &'i str {
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

fn strip_quotes(s: &str) -> String {
    s.trim_matches('"').to_string()
}

fn ensure_entity(entities: &mut Vec<Entity>, id: &str) {
    if !entities.iter().any(|e| e.id == id) {
        entities.push(Entity::new(id));
    }
}

fn upsert_entity(entities: &mut Vec<Entity>, entity: Entity) {
    if let Some(existing) = entities.iter_mut().find(|e| e.id == entity.id) {
        if entity.alias.is_some() {
            existing.alias = entity.alias;
        }
        if !entity.css_classes.is_empty() {
            existing.css_classes.extend(entity.css_classes);
        }
        existing.attributes.extend(entity.attributes);
    } else {
        entities.push(entity);
    }
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod parser_tests;
