use rusty_mermaid_core::{
    Element, MarkerType, MdSpan, Primitive, TextAnchor, Transform, parse_inline_markdown,
};

use crate::SvgConfig;
use crate::document::{SvgDocument, fmt_f64, write_f64};
use crate::markers::marker_id;
use crate::path::segments_to_d;
use crate::style::{push_style_attrs, push_text_style_attrs};

/// Render a single Primitive into the SVG document.
pub fn render_primitive(doc: &mut SvgDocument, prim: &Primitive, config: &SvgConfig) {
    match prim {
        Primitive::Rect {
            bbox,
            rx,
            ry,
            style,
        } => render_rect(doc, bbox, *rx, *ry, style),
        Primitive::Circle {
            center,
            radius,
            style,
        } => render_circle(doc, center, *radius, style),
        Primitive::Ellipse {
            center,
            rx,
            ry,
            style,
        } => render_ellipse(doc, center, *rx, *ry, style),
        Primitive::Path {
            segments,
            style,
            marker_start,
            marker_end,
        } => {
            render_path(doc, segments, style, *marker_start, *marker_end, config);
        }
        Primitive::Text {
            position,
            content,
            anchor,
            style,
        } => {
            render_text(doc, position, content, *anchor, style);
        }
        Primitive::Polygon { points, style } => render_polygon(doc, points, style),
        Primitive::Group {
            transform,
            children,
        } => {
            render_group(doc, transform, children, config);
        }
        Primitive::Arc {
            center,
            inner_r,
            outer_r,
            start_angle,
            end_angle,
            style,
        } => {
            render_arc(
                doc,
                center,
                *inner_r,
                *outer_r,
                *start_angle,
                *end_angle,
                style,
                config,
            );
        }
    }
}

fn emit_tag(doc: &mut SvgDocument, tag: &str, attrs: &[(String, String)]) {
    let refs: Vec<(&str, &str)> = attrs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    doc.empty_tag(tag, &refs);
}

fn render_rect(
    doc: &mut SvgDocument,
    bbox: &rusty_mermaid_core::BBox,
    rx: f64,
    ry: f64,
    style: &rusty_mermaid_core::Style,
) {
    let mut attrs: Vec<(String, String)> = vec![
        ("x".into(), fmt_f64(bbox.x - bbox.width / 2.0)),
        ("y".into(), fmt_f64(bbox.y - bbox.height / 2.0)),
        ("width".into(), fmt_f64(bbox.width)),
        ("height".into(), fmt_f64(bbox.height)),
    ];
    if rx > 0.0 {
        attrs.push(("rx".into(), fmt_f64(rx)));
    }
    if ry > 0.0 {
        attrs.push(("ry".into(), fmt_f64(ry)));
    }
    push_style_attrs(&mut attrs, style);
    emit_tag(doc, "rect", &attrs);
}

fn render_circle(
    doc: &mut SvgDocument,
    center: &rusty_mermaid_core::Point,
    radius: f64,
    style: &rusty_mermaid_core::Style,
) {
    let mut attrs: Vec<(String, String)> = vec![
        ("cx".into(), fmt_f64(center.x)),
        ("cy".into(), fmt_f64(center.y)),
        ("r".into(), fmt_f64(radius)),
    ];
    push_style_attrs(&mut attrs, style);
    emit_tag(doc, "circle", &attrs);
}

fn render_ellipse(
    doc: &mut SvgDocument,
    center: &rusty_mermaid_core::Point,
    rx: f64,
    ry: f64,
    style: &rusty_mermaid_core::Style,
) {
    let mut attrs: Vec<(String, String)> = vec![
        ("cx".into(), fmt_f64(center.x)),
        ("cy".into(), fmt_f64(center.y)),
        ("rx".into(), fmt_f64(rx)),
        ("ry".into(), fmt_f64(ry)),
    ];
    push_style_attrs(&mut attrs, style);
    emit_tag(doc, "ellipse", &attrs);
}

fn render_path(
    doc: &mut SvgDocument,
    segments: &[rusty_mermaid_core::PathSegment],
    style: &rusty_mermaid_core::Style,
    marker_start: Option<MarkerType>,
    marker_end: Option<MarkerType>,
    config: &SvgConfig,
) {
    let d = segments_to_d(segments);
    let mut attrs: Vec<(String, String)> = vec![("d".into(), d)];

    if style.fill.is_none() {
        attrs.push(("fill".into(), "none".into()));
    }
    if style.stroke.is_none() && style.stroke_width.is_none() {
        attrs.push(("stroke".into(), config.default_stroke.to_string()));
        attrs.push(("stroke-width".into(), fmt_f64(config.default_stroke_width)));
    }
    push_style_attrs(&mut attrs, style);

    let stroke_color = style.stroke.unwrap_or(config.default_stroke).to_string();
    if let Some(m) = marker_start {
        attrs.push((
            "marker-start".into(),
            format!("url(#{})", marker_id(m, &stroke_color)),
        ));
    }
    if let Some(m) = marker_end {
        attrs.push((
            "marker-end".into(),
            format!("url(#{})", marker_id(m, &stroke_color)),
        ));
    }
    emit_tag(doc, "path", &attrs);
}

fn render_text(
    doc: &mut SvgDocument,
    position: &rusty_mermaid_core::Point,
    content: &str,
    anchor: TextAnchor,
    style: &rusty_mermaid_core::TextStyle,
) {
    let anchor_str = match anchor {
        TextAnchor::Start => "start",
        TextAnchor::Middle => "middle",
        TextAnchor::End => "end",
    };
    let content = normalize_line_breaks(content);
    let content: &str = &content;
    let lines: Vec<&str> = content.split('\n').collect();

    if lines.len() <= 1 {
        let mut attrs: Vec<(String, String)> = vec![
            ("x".into(), fmt_f64(position.x)),
            ("y".into(), fmt_f64(position.y)),
            ("text-anchor".into(), anchor_str.into()),
            ("dominant-baseline".into(), "central".into()),
        ];
        push_text_style_attrs(&mut attrs, style);
        let refs: Vec<(&str, &str)> = attrs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        if let Some(spans) = parse_inline_markdown(content) {
            doc.open_tag("text", &refs);
            render_md_spans(doc, &spans);
            doc.close_tag("text");
        } else {
            doc.text_element("text", &refs, &xml_escape(content));
        }
        return;
    }

    // Multi-line — tspan elements with dy offsets
    let line_height = style.font_size * rusty_mermaid_core::constants::LINE_HEIGHT_MULTIPLIER;
    let total_h = line_height * (lines.len() - 1) as f64;
    let start_y = position.y - total_h / 2.0;

    let mut attrs: Vec<(String, String)> = vec![
        ("x".into(), fmt_f64(position.x)),
        ("y".into(), fmt_f64(start_y)),
        ("text-anchor".into(), anchor_str.into()),
        ("dominant-baseline".into(), "central".into()),
    ];
    push_text_style_attrs(&mut attrs, style);
    let refs: Vec<(&str, &str)> = attrs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    doc.open_tag("text", &refs);

    let x_str = fmt_f64(position.x);
    let dy_str = fmt_f64(line_height);
    for (i, line) in lines.iter().enumerate() {
        let dy: &str = if i == 0 { "0" } else { &dy_str };
        let tspan_attrs: Vec<(&str, &str)> = vec![("x", &x_str), ("dy", dy)];
        if let Some(spans) = parse_inline_markdown(line) {
            doc.open_tag("tspan", &tspan_attrs);
            render_md_spans(doc, &spans);
            doc.close_tag("tspan");
        } else {
            doc.text_element("tspan", &tspan_attrs, &xml_escape(line));
        }
    }
    doc.close_tag("text");
}

fn render_polygon(
    doc: &mut SvgDocument,
    points: &[rusty_mermaid_core::Point],
    style: &rusty_mermaid_core::Style,
) {
    let mut pts = String::with_capacity(points.len() * 16);
    for (i, p) in points.iter().enumerate() {
        if i > 0 {
            pts.push(' ');
        }
        write_f64(&mut pts, p.x);
        pts.push(',');
        write_f64(&mut pts, p.y);
    }
    let mut attrs: Vec<(String, String)> = vec![("points".into(), pts)];
    push_style_attrs(&mut attrs, style);
    emit_tag(doc, "polygon", &attrs);
}

fn render_group(
    doc: &mut SvgDocument,
    transform: &Transform,
    children: &[Primitive],
    config: &SvgConfig,
) {
    let transform_str = transform_to_attr(transform);
    let mut attrs: Vec<(String, String)> = Vec::new();
    if !transform_str.is_empty() {
        attrs.push(("transform".into(), transform_str));
    }
    let refs: Vec<(&str, &str)> = attrs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    doc.open_tag("g", &refs);
    for child in children {
        render_primitive(doc, child, config);
    }
    doc.close_tag("g");
}

/// Collect all (MarkerType, color_string) pairs from scene elements.
pub fn collect_marker_colors(
    elements: &[Element],
    config: &SvgConfig,
) -> Vec<(MarkerType, String)> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for elem in elements {
        collect_markers_prim(&elem.primitive, config, &mut seen, &mut result);
    }
    result
}

fn collect_markers_prim(
    prim: &Primitive,
    config: &SvgConfig,
    seen: &mut std::collections::HashSet<(MarkerType, String)>,
    out: &mut Vec<(MarkerType, String)>,
) {
    match prim {
        Primitive::Path {
            style,
            marker_start,
            marker_end,
            ..
        } => {
            let color = style.stroke.unwrap_or(config.default_stroke).to_string();
            for marker in [marker_start, marker_end].into_iter().flatten() {
                let pair = (*marker, color.clone());
                if seen.insert(pair.clone()) {
                    out.push(pair);
                }
            }
        }
        Primitive::Group { children, .. } => {
            for child in children {
                collect_markers_prim(child, config, seen, out);
            }
        }
        _ => {}
    }
}

fn transform_to_attr(t: &Transform) -> String {
    match t {
        Transform::Identity => String::new(),
        Transform::Translate(x, y) => format!("translate({}, {})", fmt_f64(*x), fmt_f64(*y)),
        Transform::Scale(sx, sy) => format!("scale({}, {})", fmt_f64(*sx), fmt_f64(*sy)),
        Transform::Rotate { degrees, cx, cy } => {
            format!(
                "rotate({}, {}, {})",
                fmt_f64(*degrees),
                fmt_f64(*cx),
                fmt_f64(*cy)
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_arc(
    doc: &mut SvgDocument,
    center: &rusty_mermaid_core::Point,
    inner_r: f64,
    outer_r: f64,
    start_angle: f64,
    end_angle: f64,
    style: &rusty_mermaid_core::Style,
    config: &SvgConfig,
) {
    debug_assert!(
        inner_r == 0.0,
        "SVG arc rendering does not support inner radius yet"
    );
    let x1 = center.x + outer_r * start_angle.cos();
    let y1 = center.y + outer_r * start_angle.sin();
    let x2 = center.x + outer_r * end_angle.cos();
    let y2 = center.y + outer_r * end_angle.sin();
    let large_arc = if (end_angle - start_angle).abs() > std::f64::consts::PI {
        1
    } else {
        0
    };

    let d = format!(
        "M{} {} A{} {} 0 {} 1 {} {}",
        fmt_f64(x1),
        fmt_f64(y1),
        fmt_f64(outer_r),
        fmt_f64(outer_r),
        large_arc,
        fmt_f64(x2),
        fmt_f64(y2),
    );

    let mut attrs: Vec<(String, String)> = vec![("d".into(), d)];
    push_style_attrs(&mut attrs, style);
    if style.fill.is_none() {
        attrs.push(("fill".into(), "none".into()));
    }
    if style.stroke.is_none() {
        attrs.push(("stroke".into(), config.default_stroke.to_string()));
    }
    let refs: Vec<(&str, &str)> = attrs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    doc.empty_tag("path", &refs);
}

/// Convert `<br/>` variants to `\n` for SVG multi-line rendering.
///
/// NOTE on UTF-8: the earlier implementation did `result.push(bytes[i] as char);
/// i += 1;` which only worked for ASCII. For any multi-byte codepoint (CJK,
/// emoji, combining marks, etc.) it pushed each raw byte as a separate
/// Latin-1 char, producing mojibake (`异` = `E5 BC 82` became three garbage
/// chars). We now copy whole codepoints by looking up their UTF-8 byte
/// length from the lead byte and slicing the ORIGINAL `&str` (which keeps
/// the bytes as a valid string).
///
/// `<br>` detection itself is still byte-driven because `<`, `b`/`B`,
/// `r`/`R`, `/`, ` `, `>` are all ASCII — single-byte match is correct
/// there and cheaper than a char scan.
fn normalize_line_breaks(s: &str) -> std::borrow::Cow<'_, str> {
    if !s.contains("<br") {
        return std::borrow::Cow::Borrowed(s);
    }
    let mut result = String::with_capacity(s.len());
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        // ASCII-fast <br> match. All bytes involved are ASCII, so the byte
        // index arithmetic never lands inside a multi-byte codepoint.
        if bytes[i] == b'<'
            && i + 2 < bytes.len()
            && bytes[i + 1].eq_ignore_ascii_case(&b'b')
            && bytes[i + 2].eq_ignore_ascii_case(&b'r')
        {
            let mut j = i + 3;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'/' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'>' {
                result.push('\n');
                i = j + 1;
                continue;
            }
        }
        // Copy a whole UTF-8 codepoint (1-4 bytes) by slicing the source
        // string at codepoint boundaries.
        let ch_len = utf8_codepoint_len(bytes[i]);
        let end = (i + ch_len).min(bytes.len());
        result.push_str(&s[i..end]);
        i = end;
    }
    std::borrow::Cow::Owned(result)
}

/// Given the first byte of a UTF-8 codepoint, return the codepoint's total
/// byte length (1-4). A continuation byte (0x80..0xC0) is treated as length
/// 1 to avoid getting stuck on malformed input.
#[inline]
fn utf8_codepoint_len(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first < 0xC0 {
        1 // stray continuation byte — skip it
    } else if first < 0xE0 {
        2
    } else if first < 0xF0 {
        3
    } else {
        4
    }
}

fn xml_escape(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            _ => result.push(c),
        }
    }
    result
}

/// Render a line of text with inline markdown as styled tspans.
fn render_md_spans(doc: &mut SvgDocument, spans: &[MdSpan]) {
    for span in spans {
        let escaped = xml_escape(&span.text);
        let mut attrs: Vec<(&str, &str)> = Vec::new();
        if span.bold {
            attrs.push(("font-weight", "bold"));
        }
        if span.italic {
            attrs.push(("font-style", "italic"));
        }
        if attrs.is_empty() {
            // Plain text — no wrapping tspan needed, but we use one for consistency.
            doc.text_element("tspan", &[], &escaped);
        } else {
            doc.text_element("tspan", &attrs, &escaped);
        }
    }
}

#[cfg(test)]
mod tests {
    use rusty_mermaid_core::*;

    use super::*;

    fn render_one(prim: &Primitive) -> String {
        let mut doc = SvgDocument::new(200.0, 200.0);
        render_primitive(&mut doc, prim, &SvgConfig::default());
        doc.finish()
    }

    #[test]
    fn render_rect() {
        let svg = render_one(&Primitive::Rect {
            bbox: BBox::new(50.0, 50.0, 80.0, 40.0),
            rx: 5.0,
            ry: 5.0,
            style: Style {
                fill: Some(Color::WHITE),
                stroke: Some(Color::BLACK),
                ..Default::default()
            },
        });
        assert!(svg.contains("<rect"));
        assert!(svg.contains(r#"x="10""#));
        assert!(svg.contains(r#"y="30""#));
        assert!(svg.contains(r#"width="80""#));
        assert!(svg.contains(r#"height="40""#));
        assert!(svg.contains(r#"rx="5""#));
        assert!(svg.contains(r##"fill="#ffffff""##));
    }

    #[test]
    fn render_circle() {
        let svg = render_one(&Primitive::Circle {
            center: Point::new(100.0, 100.0),
            radius: 30.0,
            style: Style::default(),
        });
        assert!(svg.contains("<circle"));
        assert!(svg.contains(r#"cx="100""#));
        assert!(svg.contains(r#"r="30""#));
    }

    #[test]
    fn render_path_with_marker() {
        let svg = render_one(&Primitive::Path {
            segments: vec![
                PathSegment::MoveTo(Point::new(0.0, 0.0)),
                PathSegment::LineTo(Point::new(100.0, 100.0)),
            ],
            style: Style {
                stroke: Some(Color::rgb(51, 51, 51)),
                ..Default::default()
            },
            marker_start: None,
            marker_end: Some(MarkerType::ArrowPoint),
        });
        assert!(svg.contains(r#"d="M0 0 L100 100""#));
        assert!(svg.contains("marker-end"));
        assert!(svg.contains("url(#arrow-point-333333)"));
    }

    #[test]
    fn render_text() {
        let svg = render_one(&Primitive::Text {
            position: Point::new(50.0, 50.0),
            content: "Hello <World>".into(),
            anchor: TextAnchor::Middle,
            style: TextStyle::default(),
        });
        assert!(svg.contains("<text"));
        assert!(svg.contains("Hello &lt;World&gt;"));
        assert!(svg.contains(r#"text-anchor="middle""#));
    }

    #[test]
    fn render_polygon() {
        let svg = render_one(&Primitive::Polygon {
            points: vec![
                Point::new(0.0, 0.0),
                Point::new(100.0, 0.0),
                Point::new(50.0, 86.6),
            ],
            style: Style::default(),
        });
        assert!(svg.contains("<polygon"));
        assert!(svg.contains("points="));
    }

    #[test]
    fn render_group() {
        let svg = render_one(&Primitive::Group {
            transform: Transform::Translate(10.0, 20.0),
            children: vec![Primitive::Circle {
                center: Point::new(0.0, 0.0),
                radius: 5.0,
                style: Style::default(),
            }],
        });
        assert!(svg.contains("<g"));
        assert!(svg.contains("translate(10, 20)"));
        assert!(svg.contains("<circle"));
        assert!(svg.contains("</g>"));
    }

    #[test]
    fn collect_marker_colors_from_paths() {
        let elems = vec![
            Element {
                primitive: Primitive::Path {
                    segments: vec![],
                    style: Style {
                        stroke: Some(Color::rgb(255, 0, 0)),
                        ..Default::default()
                    },
                    marker_start: None,
                    marker_end: Some(MarkerType::ArrowPoint),
                },
                id: None,
            },
            Element {
                primitive: Primitive::Path {
                    segments: vec![],
                    style: Style {
                        stroke: Some(Color::rgb(0, 128, 0)),
                        ..Default::default()
                    },
                    marker_start: Some(MarkerType::Circle),
                    marker_end: Some(MarkerType::ArrowPoint),
                },
                id: None,
            },
        ];
        let mc = collect_marker_colors(&elems, &SvgConfig::default());
        assert_eq!(mc.len(), 3);
        assert!(mc.contains(&(MarkerType::ArrowPoint, "#ff0000".into())));
        assert!(mc.contains(&(MarkerType::Circle, "#008000".into())));
        assert!(mc.contains(&(MarkerType::ArrowPoint, "#008000".into())));
    }

    #[test]
    fn path_default_uses_config_stroke() {
        let config = SvgConfig {
            default_stroke: Color::rgb(0, 0, 255),
            default_stroke_width: 2.0,
            ..Default::default()
        };
        let mut doc = SvgDocument::new(200.0, 200.0);
        render_primitive(
            &mut doc,
            &Primitive::Path {
                segments: vec![
                    PathSegment::MoveTo(Point::new(0.0, 0.0)),
                    PathSegment::LineTo(Point::new(100.0, 0.0)),
                ],
                style: Style::default(),
                marker_start: None,
                marker_end: None,
            },
            &config,
        );
        let svg = doc.finish();
        assert!(
            svg.contains(r##"stroke="#0000ff""##),
            "should use config default_stroke"
        );
        assert!(
            svg.contains(r#"stroke-width="2""#),
            "should use config default_stroke_width"
        );
    }

    #[test]
    fn xml_escape_special_chars() {
        assert_eq!(xml_escape("a < b & c > d"), "a &lt; b &amp; c &gt; d");
        assert_eq!(xml_escape(r#"say "hello""#), "say &quot;hello&quot;");
    }

    #[test]
    fn parse_md_bold() {
        let spans = parse_inline_markdown("hello **world**").unwrap();
        assert_eq!(spans.len(), 2);
        assert_eq!(
            spans[0],
            MdSpan {
                text: "hello ".into(),
                bold: false,
                italic: false
            }
        );
        assert_eq!(
            spans[1],
            MdSpan {
                text: "world".into(),
                bold: true,
                italic: false
            }
        );
    }

    #[test]
    fn parse_md_italic() {
        let spans = parse_inline_markdown("hello *world*").unwrap();
        assert_eq!(spans.len(), 2);
        assert_eq!(
            spans[0],
            MdSpan {
                text: "hello ".into(),
                bold: false,
                italic: false
            }
        );
        assert_eq!(
            spans[1],
            MdSpan {
                text: "world".into(),
                bold: false,
                italic: true
            }
        );
    }

    #[test]
    fn parse_md_bold_italic() {
        let spans = parse_inline_markdown("***both***").unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(
            spans[0],
            MdSpan {
                text: "both".into(),
                bold: true,
                italic: true
            }
        );
    }

    #[test]
    fn parse_md_no_markers() {
        assert!(parse_inline_markdown("plain text").is_none());
    }

    #[test]
    fn render_text_bold() {
        let svg = render_one(&Primitive::Text {
            position: Point::new(50.0, 50.0),
            content: "hello **world**".into(),
            anchor: TextAnchor::Middle,
            style: TextStyle::default(),
        });
        assert!(svg.contains(r#"font-weight="bold"#));
        assert!(svg.contains(">world</tspan>"));
        assert!(svg.contains(">hello </tspan>"));
    }

    #[test]
    fn render_text_italic() {
        let svg = render_one(&Primitive::Text {
            position: Point::new(50.0, 50.0),
            content: "hello *world*".into(),
            anchor: TextAnchor::Middle,
            style: TextStyle::default(),
        });
        assert!(svg.contains(r#"font-style="italic"#));
        assert!(svg.contains(">world</tspan>"));
    }

    #[test]
    fn render_text_br_to_multiline() {
        let svg = render_one(&Primitive::Text {
            position: Point::new(50.0, 50.0),
            content: "line1<br/>line2".into(),
            anchor: TextAnchor::Middle,
            style: TextStyle::default(),
        });
        // <br/> should produce multi-line tspans
        assert!(svg.contains(">line1</tspan>"));
        assert!(svg.contains(">line2</tspan>"));
    }

    #[test]
    fn render_text_br_variant() {
        let svg = render_one(&Primitive::Text {
            position: Point::new(50.0, 50.0),
            content: "a<br>b<br />c".into(),
            anchor: TextAnchor::Middle,
            style: TextStyle::default(),
        });
        assert!(svg.contains(">a</tspan>"));
        assert!(svg.contains(">b</tspan>"));
        assert!(svg.contains(">c</tspan>"));
    }

    #[test]
    fn render_multiline_with_markdown() {
        let svg = render_one(&Primitive::Text {
            position: Point::new(50.0, 50.0),
            content: "**bold line**\nplain line".into(),
            anchor: TextAnchor::Middle,
            style: TextStyle::default(),
        });
        assert!(svg.contains(r#"font-weight="bold"#));
        assert!(svg.contains(">bold line</tspan>"));
        assert!(svg.contains(">plain line</tspan>"));
    }
}
