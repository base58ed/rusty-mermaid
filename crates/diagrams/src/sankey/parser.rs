use super::ir::{SankeyDiagram, SankeyLink};
use crate::common::error::{ParseError, ParseErrorKind};

/// Parse a sankey diagram from mermaid syntax.
///
/// ```text
/// sankey-beta
/// source,target,value
/// "quoted source","target",123.4
/// ```
pub fn parse(input: &str) -> Result<SankeyDiagram, ParseError> {
    let mut links = Vec::new();
    let mut header_found = false;

    for (line_no, raw_line) in input.lines().enumerate() {
        let line = raw_line.trim();

        if line.is_empty() || line.starts_with("%%") {
            continue;
        }

        if !header_found {
            if line.starts_with("sankey") {
                header_found = true;
                continue;
            }
            return Err(ParseError::new(
                ParseErrorKind::UnexpectedToken,
                0..line.len().min(10),
                input,
            ));
        }

        // Parse CSV line: source, target, value
        let fields = parse_csv_line(line);
        if fields.len() != 3 {
            return Err(ParseError::new(
                ParseErrorKind::UnexpectedToken,
                byte_offset(input, line_no)..byte_offset(input, line_no) + line.len(),
                input,
            ));
        }

        let value: f64 = fields[2].parse().map_err(|_| {
            ParseError::new(
                ParseErrorKind::UnexpectedToken,
                byte_offset(input, line_no)..byte_offset(input, line_no) + line.len(),
                input,
            )
        })?;

        if value < 0.0 {
            return Err(ParseError::new(
                ParseErrorKind::UnexpectedToken,
                byte_offset(input, line_no)..byte_offset(input, line_no) + line.len(),
                input,
            ));
        }

        links.push(SankeyLink {
            source: fields[0].clone(),
            target: fields[1].clone(),
            value,
        });
    }

    if !header_found {
        return Err(ParseError::new(
            ParseErrorKind::UnexpectedToken,
            0..input.len().min(10),
            input,
        ));
    }

    if links.is_empty() {
        return Err(ParseError::new(
            ParseErrorKind::UnexpectedEof,
            input.len()..input.len(),
            input,
        ));
    }

    Ok(SankeyDiagram { links })
}

/// Parse a single CSV line respecting quoted fields.
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut chars = line.chars().peekable();
    let mut field = String::new();
    let mut in_quotes = false;

    while let Some(&ch) = chars.peek() {
        if in_quotes {
            chars.next();
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(ch);
            }
        } else {
            match ch {
                ',' => {
                    chars.next();
                    fields.push(field.trim().to_string());
                    field = String::new();
                }
                '"' => {
                    chars.next();
                    in_quotes = true;
                }
                _ => {
                    chars.next();
                    field.push(ch);
                }
            }
        }
    }

    fields.push(field.trim().to_string());
    fields
}

fn byte_offset(input: &str, line_no: usize) -> usize {
    input.lines().take(line_no).map(|l| l.len() + 1).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic() {
        let d = parse("sankey-beta\nA,B,10\nA,C,5").unwrap();
        assert_eq!(d.links.len(), 2);
        assert_eq!(d.links[0].source, "A");
        assert_eq!(d.links[0].target, "B");
        assert!((d.links[0].value - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_sankey_header() {
        let d = parse("sankey\nX,Y,100").unwrap();
        assert_eq!(d.links.len(), 1);
    }

    #[test]
    fn parse_quoted_fields() {
        let d = parse("sankey-beta\n\"Source, A\",\"Target B\",42.5").unwrap();
        assert_eq!(d.links[0].source, "Source, A");
        assert_eq!(d.links[0].target, "Target B");
    }

    #[test]
    fn parse_escaped_quotes() {
        let d = parse("sankey-beta\n\"He said \"\"hi\"\"\",B,1").unwrap();
        assert_eq!(d.links[0].source, "He said \"hi\"");
    }

    #[test]
    fn parse_comments_and_blanks() {
        let d = parse("sankey-beta\n%% comment\n\nA,B,10\n\n%% another\nC,D,5").unwrap();
        assert_eq!(d.links.len(), 2);
    }

    // 3.75 rather than 3.14: the value is arbitrary sample data, and a literal
    // approximating a math constant trips `clippy::approx_constant`.
    #[test]
    fn parse_float_values() {
        let d = parse("sankey-beta\nA,B,3.75\nC,D,0.001").unwrap();
        assert!((d.links[0].value - 3.75).abs() < 1e-10);
        assert!((d.links[1].value - 0.001).abs() < 1e-10);
    }

    #[test]
    fn reject_no_header() {
        assert!(parse("A,B,10").is_err());
    }

    #[test]
    fn reject_no_links() {
        assert!(parse("sankey-beta\n%% only comments").is_err());
    }

    #[test]
    fn reject_wrong_field_count() {
        assert!(parse("sankey-beta\nA,B").is_err());
    }

    #[test]
    fn reject_non_numeric_value() {
        assert!(parse("sankey-beta\nA,B,abc").is_err());
    }

    #[test]
    fn reject_negative_value() {
        assert!(parse("sankey-beta\nA,B,-5").is_err());
    }

    #[test]
    fn csv_line_simple() {
        assert_eq!(parse_csv_line("a,b,c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn csv_line_quoted() {
        assert_eq!(parse_csv_line("\"a,b\",c,d"), vec!["a,b", "c", "d"]);
    }

    #[test]
    fn csv_line_whitespace() {
        assert_eq!(parse_csv_line("  a , b , c  "), vec!["a", "b", "c"]);
    }
}
