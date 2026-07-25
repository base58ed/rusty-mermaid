use super::*;

#[test]
fn parse_single_node() {
    let d = parse("graph TD\n    A[Only Node]").unwrap();
    assert_eq!(d.direction, Direction::TB);
    assert_eq!(d.vertices.len(), 1);
    assert_eq!(d.vertices[0].id, "A");
    assert_eq!(d.vertices[0].label, "Only Node");
    assert_eq!(d.vertices[0].shape, Shape::Rect);
}

#[test]
fn parse_linear_chain() {
    let d = parse("graph TD\n    A[Start] --> B[Middle] --> C[End]").unwrap();
    assert_eq!(d.vertices.len(), 3);
    assert_eq!(d.edges.len(), 2);
    assert_eq!(d.edges[0].src, "A");
    assert_eq!(d.edges[0].dst, "B");
    assert_eq!(d.edges[1].src, "B");
    assert_eq!(d.edges[1].dst, "C");
}

#[test]
fn parse_diamond() {
    let d = parse("graph TD\n    A{Decision?}").unwrap();
    assert_eq!(d.vertices[0].shape, Shape::Diamond);
    assert_eq!(d.vertices[0].label, "Decision?");
}

#[test]
fn parse_cylinder() {
    let d = parse("graph TD\n    A[(Database)]").unwrap();
    assert_eq!(d.vertices[0].shape, Shape::Cylinder);
}

#[test]
fn parse_edge_label_pipe() {
    let d = parse("graph TD\n    A -->|yes| B").unwrap();
    assert_eq!(d.edges[0].label.as_deref(), Some("yes"));
}

#[test]
fn parse_dotted_edge() {
    let d = parse("graph TD\n    A -.-> B").unwrap();
    assert_eq!(d.edges[0].stroke, StrokeType::Dotted);
    assert_eq!(d.edges[0].end_arrow, ArrowEnd::Arrow);
}

#[test]
fn parse_edge_minlen() {
    let d = parse("flowchart TD\n    A1 --> B1\n    A2 ---> B2\n    A3 ----> B3\n    A4 -----> B4")
        .unwrap();
    assert_eq!(d.edges[0].minlen, 1);
    assert_eq!(d.edges[1].minlen, 2);
    assert_eq!(d.edges[2].minlen, 3);
    assert_eq!(d.edges[3].minlen, 4);
}

#[test]
fn parse_edge_minlen_dotted() {
    let d = parse("flowchart TD\n    A -.-> B\n    C -..-> D\n    E -...-> F").unwrap();
    assert_eq!(d.edges[0].minlen, 1);
    assert_eq!(d.edges[1].minlen, 2);
    assert_eq!(d.edges[2].minlen, 3);
}

#[test]
fn parse_edge_minlen_thick() {
    let d = parse("flowchart TD\n    A ==> B\n    C ===> D\n    E ====> F").unwrap();
    assert_eq!(d.edges[0].minlen, 1);
    assert_eq!(d.edges[1].minlen, 2);
    assert_eq!(d.edges[2].minlen, 3);
}

#[test]
fn parse_flowchart_lr() {
    let d = parse("flowchart LR\n    A --> B").unwrap();
    assert_eq!(d.direction, Direction::LR);
}

#[test]
fn parse_quoted_label() {
    let d = parse("graph TD\n    A[\"<b>Bold</b>\"]").unwrap();
    assert_eq!(d.vertices[0].label, "<b>Bold</b>");
}

#[test]
fn parse_subgraph() {
    let d = parse("graph TD\n    subgraph cluster[Group]\n        A --> B\n    end\n    B --> C")
        .unwrap();
    assert_eq!(d.subgraphs.len(), 1);
    assert_eq!(d.subgraphs[0].id, "cluster");
    assert_eq!(d.subgraphs[0].label.as_deref(), Some("Group"));
    assert!(d.subgraphs[0].node_ids.contains(&"A".to_string()));
    assert!(d.subgraphs[0].node_ids.contains(&"B".to_string()));
}

/// A quoted subgraph title carries no quotes into the label. The bare-bracket
/// branch also matched `["`, so the quoted branch below it was unreachable and
/// the quote characters rode through into every renderer's output.
/// (Mutation: restore the old branch order → the label keeps its quotes.)
#[test]
fn parse_subgraph_quoted_title_drops_the_quotes() {
    let d =
        parse("graph TD\n    subgraph hdr[\"Header fields\"]\n        A --> B\n    end").unwrap();
    assert_eq!(d.subgraphs[0].id, "hdr");
    assert_eq!(d.subgraphs[0].label.as_deref(), Some("Header fields"));
}

/// An unquoted bracketed title still reaches the label verbatim, so the
/// reordering cannot swallow ordinary titles.
#[test]
fn parse_subgraph_bracketed_title_is_untouched() {
    let d = parse("graph TD\n    subgraph g[Header fields]\n        A --> B\n    end").unwrap();
    assert_eq!(d.subgraphs[0].label.as_deref(), Some("Header fields"));
}

#[test]
fn parse_subgraph_unbracketed_title() {
    let d = parse("graph TD\n    subgraph Frontend\n        A --> B\n    end").unwrap();
    assert_eq!(d.subgraphs[0].label.as_deref(), Some("Frontend"));
}

#[test]
fn parse_nested_subgraphs() {
    let input = "\
graph TD
    subgraph outer[Outer]
        subgraph inner[Inner]
            A --> B
        end
        C
    end";
    let d = parse(input).unwrap();
    assert_eq!(d.subgraphs.len(), 2);
    let outer = d.subgraphs.iter().find(|s| s.id == "outer").unwrap();
    assert!(outer.subgraph_ids.contains(&"inner".to_string()));
}

#[test]
fn parse_comments_ignored() {
    let d = parse("graph TD\n    %% This is a comment\n    A --> B").unwrap();
    assert_eq!(d.vertices.len(), 2);
    assert_eq!(d.edges.len(), 1);
}

#[test]
fn parse_node_reuse_without_shape() {
    let d = parse("graph TD\n    A[Start] --> B\n    B --> C[End]").unwrap();
    assert_eq!(d.vertices.len(), 3);
    // B should exist with default shape
    let b = d.vertex("B").unwrap();
    assert_eq!(b.shape, Shape::Rect);
}

#[test]
fn parse_self_loop() {
    let d = parse("graph TD\n    A[Node] --> B\n    A --> A").unwrap();
    let self_edge = d.edges.iter().find(|e| e.src == "A" && e.dst == "A");
    assert!(self_edge.is_some());
}

#[test]
fn parse_realistic_flowchart() {
    let input = "\
graph TD
    start[Start] --> input[Get Input]
    input --> validate{Valid?}
    validate -->|No| error[Show Error]
    validate -->|Yes| process[Process Data]
    error --> input
    process --> decide{Choose Path}
    decide -->|A| optA[Option A]
    decide -->|B| optB[Option B]
    optA --> merge[Merge]
    optB --> merge
    merge --> output[Output Result]
    output --> done[End]";
    let d = parse(input).unwrap();
    assert_eq!(d.vertices.len(), 11);
    assert_eq!(d.edges.len(), 12);
    assert_eq!(d.vertex("validate").unwrap().shape, Shape::Diamond);
}

#[test]
fn parse_all_directions() {
    for (keyword, expected) in [
        ("graph TB", Direction::TB),
        ("graph TD", Direction::TB),
        ("graph BT", Direction::BT),
        ("graph LR", Direction::LR),
        ("graph RL", Direction::RL),
    ] {
        let d = parse(&format!("{keyword}\n    A --> B")).unwrap();
        assert_eq!(d.direction, expected, "failed for {keyword}");
    }
}

#[test]
fn parse_link_style_by_index() {
    let d = parse("flowchart TD\n    A --> B --> C\n    linkStyle 0 stroke:#f00,stroke-width:3px")
        .unwrap();
    assert_eq!(d.link_styles.len(), 1);
    assert!(!d.link_styles[0].is_default);
    assert_eq!(d.link_styles[0].indices, vec![0]);
    assert_eq!(d.link_styles[0].styles[0].key, "stroke");
    assert_eq!(d.link_styles[0].styles[0].value, "#f00");
}

#[test]
fn parse_link_style_multiple_indices() {
    let d = parse("flowchart TD\n    A --> B --> C\n    linkStyle 0,1 stroke:red").unwrap();
    assert_eq!(d.link_styles[0].indices, vec![0, 1]);
}

#[test]
fn parse_link_style_default() {
    let d = parse("flowchart TD\n    A --> B\n    linkStyle default stroke:green").unwrap();
    assert!(d.link_styles[0].is_default);
    assert!(d.link_styles[0].indices.is_empty());
}

#[test]
fn parse_subgraph_direction() {
    let d = parse(
        "flowchart TD\n    subgraph sub1[Group]\n        direction LR\n        A --> B\n    end",
    )
    .unwrap();
    assert_eq!(d.subgraphs[0].direction, Some(Direction::LR));
}

#[test]
fn parse_subgraph_no_direction() {
    let d = parse("flowchart TD\n    subgraph sub1[Group]\n        A --> B\n    end").unwrap();
    assert_eq!(d.subgraphs[0].direction, None);
}

// ── Negative / error-path tests ──────────────────────────────────

#[test]
fn reject_empty_input() {
    assert!(parse("").is_err(), "empty input must fail (no header)");
}

#[test]
fn reject_whitespace_only() {
    assert!(
        parse("   \n\n  ").is_err(),
        "whitespace-only input must fail"
    );
}

#[test]
fn reject_no_diagram_header() {
    assert!(
        parse("A --> B").is_err(),
        "input without graph/flowchart header must fail"
    );
}

#[test]
fn reject_unknown_keyword_header() {
    assert!(
        parse("diagram TD\n    A --> B").is_err(),
        "unknown header keyword must fail"
    );
}

#[test]
fn reject_missing_direction_after_graph() {
    // `graph` followed by a newline — no valid direction token.
    // The parser requires a direction after the keyword.
    assert!(
        parse("graph\n    A --> B").is_err(),
        "missing direction after 'graph' must fail"
    );
}

#[test]
fn unclosed_bracket_in_node_shape_is_lenient() {
    // `A[text` has no closing `]`. text_until fails and backtracks, so
    // parse_node_shape returns Err. parse_node_ref then creates `A` as a
    // bare node (default Rect with ID as label). The remaining `[text` is
    // consumed character-by-character by subsequent parse_node_edge_statement
    // attempts which also backtrack. The parser is lenient — it does NOT
    // return an error.
    let d = parse("graph TD\n    A[text").unwrap();
    let a = d.vertex("A").unwrap();
    assert_eq!(
        a.shape,
        Shape::Rect,
        "unclosed bracket falls back to bare node"
    );
    assert_eq!(
        a.label, "A",
        "label defaults to node ID when shape parse fails"
    );
}

#[test]
fn incomplete_arrow_no_head() {
    // `A -- B` is not a valid edge — `--` is the start of normal stroke but
    // the next token is ` B` (space + identifier), which the parser treats
    // as an inline label. Then it expects more dashes to close the label
    // section. This should either error or produce unexpected results.
    let result = parse("graph TD\n    A -- B");
    // The parser interprets `B` as an inline edge label text (between
    // dashes). With no closing dashes, it consumes to EOF. Since there is
    // no destination node, this produces an error.
    assert!(
        result.is_err(),
        "incomplete arrow 'A -- B' (no >) must fail"
    );
}

#[test]
fn invalid_chars_in_node_id() {
    // node_id requires first char to be alphanumeric or underscore.
    // `@node` starts with `@` which fails the node_id parser.
    let result = parse("graph TD\n    @node --> B");
    assert!(result.is_err(), "node ID starting with '@' must fail");
}

#[test]
fn invalid_special_chars_in_node_id() {
    // `$A` starts with `$` which is not alphanumeric/underscore.
    let result = parse("graph TD\n    $A --> B");
    assert!(result.is_err(), "node ID starting with '$' must fail");
}

#[test]
fn unclosed_subgraph_at_eof() {
    // Subgraph without `end` — parse_statements runs until EOF and returns
    // Ok. The parser is lenient: it treats EOF as closing the scope.
    let result = parse("graph TD\n    subgraph cluster[Group]\n        A --> B");
    assert!(
        result.is_ok(),
        "unclosed subgraph is tolerated (EOF closes scope)"
    );
    let d = result.unwrap();
    assert_eq!(d.subgraphs.len(), 1);
    assert_eq!(d.subgraphs[0].id, "cluster");
}

#[test]
fn duplicate_node_declarations_last_shape_wins() {
    // Defining the same node with different shapes: second definition updates.
    let d = parse("graph TD\n    A((Circle))\n    A{Diamond}").unwrap();
    let a = d.vertex("A").unwrap();
    // parse_node_ref updates existing vertex in-place with new shape/label.
    assert_eq!(a.shape, Shape::Diamond, "last shape declaration wins");
    assert_eq!(a.label, "Diamond", "last label declaration wins");
}

#[test]
fn duplicate_node_keeps_single_vertex() {
    let d = parse("graph TD\n    A[First]\n    A[Second]\n    A --> B").unwrap();
    // Should have exactly 2 vertices (A and B), not 3.
    assert_eq!(d.vertices.len(), 2);
    assert_eq!(d.vertex("A").unwrap().label, "Second");
}

#[test]
fn edge_with_no_destination() {
    // `A -->` followed by EOF — no destination node.
    let result = parse("graph TD\n    A -->");
    assert!(result.is_err(), "edge with no destination must fail");
}

#[test]
fn edge_with_no_source() {
    // `--> B` — the `-->` is not a valid node ID, so it fails.
    let result = parse("graph TD\n    --> B");
    assert!(result.is_err(), "edge with no source must fail");
}

#[test]
fn unclosed_parenthesis_in_node_is_lenient() {
    // Same lenient behavior as unclosed bracket: parse_node_shape fails,
    // `A` becomes a bare node, remaining `(text` is skipped.
    let d = parse("graph TD\n    A(text").unwrap();
    let a = d.vertex("A").unwrap();
    assert_eq!(
        a.shape,
        Shape::Rect,
        "unclosed paren falls back to bare node"
    );
    assert_eq!(a.label, "A");
}

#[test]
fn unclosed_curly_in_node_is_lenient() {
    // Same lenient behavior: parse_node_shape fails on unclosed `{`,
    // `A` becomes a bare node.
    let d = parse("graph TD\n    A{text").unwrap();
    let a = d.vertex("A").unwrap();
    assert_eq!(
        a.shape,
        Shape::Rect,
        "unclosed curly falls back to bare node"
    );
    assert_eq!(a.label, "A");
}

#[test]
fn only_header_no_statements() {
    // Just the header with no nodes/edges — should parse to an empty diagram.
    let d = parse("graph TD").unwrap();
    assert!(d.vertices.is_empty());
    assert!(d.edges.is_empty());
}

#[test]
fn invalid_direction_keyword() {
    let result = parse("graph XY\n    A --> B");
    assert!(result.is_err(), "invalid direction 'XY' must fail");
}
