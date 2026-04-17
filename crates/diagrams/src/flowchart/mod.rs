pub mod bridge;
pub mod ir;
pub mod parser;

use rusty_mermaid_core::{
    BBox, CurveType, MarkerType, PathSegment, Point, Primitive, Scene, Shape, Style, TextAnchor,
    TextStyle, Theme, interpolate,
};

const SUBGRAPH_LABEL_OFFSET_X: f64 = 8.0;
const SUBGRAPH_LABEL_OFFSET_Y: f64 = 12.0;

use crate::common::layout::NodeLayout;
use bridge::LayoutResult;
use ir::{ArrowEnd, StrokeType};

use crate::common::rendering::{
    contrasting_label_style, merge_custom_style, overlay_style, render_edge_label,
    shorten_path_for_markers,
};

fn edge_style(stroke: StrokeType, theme: &Theme) -> Style {
    let sw = theme.default_stroke_width;
    Style {
        stroke: Some(theme.edge_stroke),
        stroke_width: Some(match stroke {
            StrokeType::Thick => sw * 2.0 + 0.5,
            _ => sw,
        }),
        stroke_dasharray: match stroke {
            StrokeType::Dotted => Some(vec![3.0, 3.0]),
            _ => None,
        },
        ..Default::default()
    }
}

/// Convert a flowchart layout result into a Scene of drawing primitives.
pub fn to_scene(layout: &LayoutResult, theme: &Theme) -> Scene {
    let mut scene = Scene::new(layout.width, layout.height);
    layout_to_scene(layout, &mut scene, theme);
    scene
}

fn subgraph_style(theme: &Theme) -> Style {
    Style {
        fill: Some(theme.subgraph_fill),
        stroke: Some(theme.subgraph_stroke),
        stroke_width: Some(1.0),
        ..Default::default()
    }
}

fn subgraph_label_style(theme: &Theme) -> TextStyle {
    TextStyle {
        // Subgraph labels are the spatial "index" of the diagram — "前端层",
        // "数据层". They deserve to read larger than node body text. +3pt on
        // top of font_size_label (13 → 16) makes them scannable without
        // overwhelming the node fonts.
        font_size: theme.font_size_label + 3.0,
        fill: Some(theme.subgraph_label),
        font_weight: rusty_mermaid_core::FontWeight::Bold,
        ..Default::default()
    }
}

fn layout_to_scene(layout: &LayoutResult, scene: &mut Scene, theme: &Theme) {
    // Draw subgraph boundaries first (behind nodes), largest first so
    // nested subgraphs render on top of their parents.
    let mut subgraphs: Vec<&_> = layout.subgraphs.iter().collect();
    subgraphs.sort_by(|a, b| {
        let area_a = a.width * a.height;
        let area_b = b.width * b.height;
        area_b
            .partial_cmp(&area_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for sg in &subgraphs {
        let bbox = BBox::new(sg.x, sg.y, sg.width, sg.height);
        scene.push(Primitive::Rect {
            bbox,
            rx: 5.0,
            ry: 5.0,
            style: subgraph_style(theme),
        });
        if let Some(label) = &sg.label {
            let top_y = sg.y - sg.height / 2.0;
            let left_x = sg.x - sg.width / 2.0;
            scene.push(Primitive::Text {
                position: Point::new(
                    left_x + SUBGRAPH_LABEL_OFFSET_X,
                    top_y + SUBGRAPH_LABEL_OFFSET_Y,
                ),
                content: label.clone(),
                anchor: TextAnchor::Start,
                style: subgraph_label_style(theme),
            });
        }
    }

    // Z-order: subgraphs (background) → edges + markers → nodes (foreground).
    for edge in &layout.edges {
        if edge.points.len() >= 2 {
            let mut segments = interpolate(&edge.points, CurveType::Basis);

            let marker_end = match edge.end_arrow {
                ArrowEnd::Arrow => Some(MarkerType::ArrowPoint),
                ArrowEnd::Circle => Some(MarkerType::Circle),
                ArrowEnd::Cross => Some(MarkerType::Cross),
                ArrowEnd::None => None,
            };
            let marker_start = match edge.start_arrow {
                ArrowEnd::Arrow => Some(MarkerType::ArrowPoint),
                ArrowEnd::Circle => Some(MarkerType::Circle),
                ArrowEnd::Cross => Some(MarkerType::Cross),
                ArrowEnd::None => None,
            };

            let mut estyle = edge_style(edge.stroke, theme);
            if let Some(custom) = &edge.custom_style {
                overlay_style(&mut estyle, custom);
            }

            let sw = estyle.stroke_width.unwrap_or(theme.default_stroke_width);
            shorten_path_for_markers(&mut segments, marker_start, marker_end, sw);

            scene.push(Primitive::Path {
                segments,
                style: estyle,
                marker_start,
                marker_end,
            });
            if let Some(label) = &edge.label {
                let mid = edge.points[edge.points.len() / 2];
                render_edge_label(scene, mid, label, edge.label_size, theme);
            }
        }
    }

    for node in &layout.nodes {
        render_node(node, scene, theme);
    }
}

// ---------------------------------------------------------------------------
// Shape rendering — formulas match mermaid.js
// ---------------------------------------------------------------------------

fn render_node(node: &NodeLayout, scene: &mut Scene, theme: &Theme) {
    let style = merge_custom_style(node.custom_style.as_ref(), theme);
    let node_fill = style.fill;
    let cx = node.x;
    let cy = node.y;
    let w = node.width;
    let h = node.height;

    let bbox = BBox::new(cx, cy, w, h);

    match node.shape {
        Shape::Rect => {
            scene.push(Primitive::Rect {
                bbox,
                rx: 0.0,
                ry: 0.0,
                style,
            });
        }
        Shape::RoundedRect => {
            scene.push(Primitive::Rect {
                bbox,
                rx: 5.0,
                ry: 5.0,
                style,
            });
        }
        Shape::Stadium => {
            let r = h / 2.0;
            scene.push(Primitive::Rect {
                bbox,
                rx: r,
                ry: r,
                style,
            });
        }
        Shape::Diamond => render_diamond(bbox, style, scene),
        Shape::Circle => {
            let r = w.max(h) / 2.0;
            scene.push(Primitive::Circle {
                center: Point::new(cx, cy),
                radius: r,
                style,
            });
        }
        Shape::DoubleCircle => {
            let gap = 5.0;
            let outer_r = w.max(h) / 2.0;
            let inner_r = outer_r - gap;
            scene.push(Primitive::Circle {
                center: Point::new(cx, cy),
                radius: outer_r,
                style: style.clone(),
            });
            scene.push(Primitive::Circle {
                center: Point::new(cx, cy),
                radius: inner_r,
                style,
            });
        }
        Shape::Hexagon => render_hexagon(bbox, style, scene),
        Shape::Parallelogram => render_parallelogram(bbox, style, scene),
        Shape::ParallelogramAlt => render_parallelogram_alt(bbox, style, scene),
        Shape::Trapezoid => render_trapezoid(bbox, style, scene),
        Shape::TrapezoidAlt => render_trapezoid_alt(bbox, style, scene),
        Shape::Cylinder => render_cylinder(bbox, style, scene),
        Shape::Subroutine => render_subroutine(bbox, style, scene),
        Shape::Asymmetric => render_asymmetric(bbox, style, scene),
        _ => {
            scene.push(Primitive::Rect {
                bbox,
                rx: 3.0,
                ry: 3.0,
                style,
            });
        }
    }

    // Cylinder: center text on the wall below the top elliptical cap.
    // Wall runs from (top + ry) to bottom; its center is cy + ry/2.
    let label_y = if node.shape == Shape::Cylinder {
        let rx = w / 2.0;
        let ry = rx / (2.5 + w / 50.0);
        cy + ry / 2.0
    } else {
        cy
    };

    scene.push(Primitive::Text {
        position: Point::new(cx, label_y),
        content: node.label.clone(),
        anchor: TextAnchor::Middle,
        style: contrasting_label_style(node_fill, theme),
    });
}

/// Diamond: 4-point rhombus. Mermaid computes s = w + h, then draws a
/// diamond of that size. We use w and h directly to fit dagre's box.
fn render_diamond(bbox: BBox, style: Style, scene: &mut Scene) {
    let (cx, cy) = (bbox.x, bbox.y);
    let (hw, hh) = (bbox.width / 2.0, bbox.height / 2.0);
    scene.push(Primitive::Polygon {
        points: vec![
            Point::new(cx, cy - hh),
            Point::new(cx + hw, cy),
            Point::new(cx, cy + hh),
            Point::new(cx - hw, cy),
        ],
        style,
    });
}

/// Hexagon: 6 points. Cut amount m = h/4 on each side (matches mermaid f=4).
fn render_hexagon(bbox: BBox, style: Style, scene: &mut Scene) {
    let (cx, cy) = (bbox.x, bbox.y);
    let (hw, hh) = (bbox.width / 2.0, bbox.height / 2.0);
    let m = bbox.height / 4.0;
    scene.push(Primitive::Polygon {
        points: vec![
            Point::new(cx - hw + m, cy - hh), // top-left
            Point::new(cx + hw - m, cy - hh), // top-right
            Point::new(cx + hw, cy),          // right
            Point::new(cx + hw - m, cy + hh), // bottom-right
            Point::new(cx - hw + m, cy + hh), // bottom-left
            Point::new(cx - hw, cy),          // left
        ],
        style,
    });
}

/// Parallelogram (lean right): skew = h/2 (mermaid uses 3*h/6 = h/2).
fn render_parallelogram(bbox: BBox, style: Style, scene: &mut Scene) {
    let (cx, cy) = (bbox.x, bbox.y);
    let (hw, hh) = (bbox.width / 2.0, bbox.height / 2.0);
    let skew = bbox.height / 2.0;
    scene.push(Primitive::Polygon {
        points: vec![
            Point::new(cx - hw + skew, cy - hh), // top-left
            Point::new(cx + hw + skew, cy - hh), // top-right
            Point::new(cx + hw - skew, cy + hh), // bottom-right
            Point::new(cx - hw - skew, cy + hh), // bottom-left
        ],
        style,
    });
}

/// Parallelogram alt (lean left): opposite skew direction.
fn render_parallelogram_alt(bbox: BBox, style: Style, scene: &mut Scene) {
    let (cx, cy) = (bbox.x, bbox.y);
    let (hw, hh) = (bbox.width / 2.0, bbox.height / 2.0);
    let skew = bbox.height / 2.0;
    scene.push(Primitive::Polygon {
        points: vec![
            Point::new(cx - hw - skew, cy - hh), // top-left
            Point::new(cx + hw - skew, cy - hh), // top-right
            Point::new(cx + hw + skew, cy + hh), // bottom-right
            Point::new(cx - hw + skew, cy + hh), // bottom-left
        ],
        style,
    });
}

/// Trapezoid: top narrower than bottom. Offset = h/2 on each side.
fn render_trapezoid(bbox: BBox, style: Style, scene: &mut Scene) {
    let (cx, cy) = (bbox.x, bbox.y);
    let (hw, hh) = (bbox.width / 2.0, bbox.height / 2.0);
    let offset = bbox.height / 2.0;
    scene.push(Primitive::Polygon {
        points: vec![
            Point::new(cx - hw, cy - hh),          // top-left
            Point::new(cx + hw, cy - hh),          // top-right
            Point::new(cx + hw + offset, cy + hh), // bottom-right (wider)
            Point::new(cx - hw - offset, cy + hh), // bottom-left (wider)
        ],
        style,
    });
}

/// Trapezoid alt (inverted): bottom narrower than top.
fn render_trapezoid_alt(bbox: BBox, style: Style, scene: &mut Scene) {
    let (cx, cy) = (bbox.x, bbox.y);
    let (hw, hh) = (bbox.width / 2.0, bbox.height / 2.0);
    let offset = bbox.height / 2.0;
    scene.push(Primitive::Polygon {
        points: vec![
            Point::new(cx - hw - offset, cy - hh), // top-left (wider)
            Point::new(cx + hw + offset, cy - hh), // top-right (wider)
            Point::new(cx + hw, cy + hh),          // bottom-right
            Point::new(cx - hw, cy + hh),          // bottom-left
        ],
        style,
    });
}

/// Cylinder: rect body + elliptical top/bottom caps.
/// Mermaid: ry = rx / (2.5 + w/50).
///
/// Drawn as two paths: (1) body with bottom arc, (2) top ellipse on top.
/// A single combined path causes fill artifacts where the top cap's
/// interior remains unfilled due to winding/even-odd fill rules.
fn render_cylinder(bbox: BBox, style: Style, scene: &mut Scene) {
    let (cx, cy, w, h) = (bbox.x, bbox.y, bbox.width, bbox.height);
    let hw = w / 2.0;
    let rx = hw;
    let ry = rx / (2.5 + w / 50.0);
    let body_h = h - ry;
    let top = cy - body_h / 2.0;
    let bottom = cy + body_h / 2.0;

    // Body: left side down → bottom arc → right side up → top back-arc to close
    let body = vec![
        PathSegment::MoveTo(Point::new(cx - hw, top)),
        PathSegment::LineTo(Point::new(cx - hw, bottom)),
        PathSegment::ArcTo {
            rx,
            ry,
            rotation: 0.0,
            large_arc: false,
            sweep: false,
            to: Point::new(cx + hw, bottom),
        },
        PathSegment::LineTo(Point::new(cx + hw, top)),
        // Close with back-arc so the top edge is smooth
        PathSegment::ArcTo {
            rx,
            ry,
            rotation: 0.0,
            large_arc: false,
            sweep: true,
            to: Point::new(cx - hw, top),
        },
    ];
    scene.push(Primitive::Path {
        segments: body,
        style: style.clone(),
        marker_start: None,
        marker_end: None,
    });

    // Top ellipse (front arc only — drawn on top of body)
    let top_cap = vec![
        PathSegment::MoveTo(Point::new(cx - hw, top)),
        PathSegment::ArcTo {
            rx,
            ry,
            rotation: 0.0,
            large_arc: false,
            sweep: true,
            to: Point::new(cx + hw, top),
        },
        PathSegment::ArcTo {
            rx,
            ry,
            rotation: 0.0,
            large_arc: false,
            sweep: true,
            to: Point::new(cx - hw, top),
        },
    ];
    scene.push(Primitive::Path {
        segments: top_cap,
        style,
        marker_start: None,
        marker_end: None,
    });
}

/// Asymmetric (flag/pennant): rectangle with a V-notch on the left.
/// Mermaid `>text]` shape. Notch indentation = h/4.
fn render_asymmetric(bbox: BBox, style: Style, scene: &mut Scene) {
    let (cx, cy) = (bbox.x, bbox.y);
    let (hw, hh) = (bbox.width / 2.0, bbox.height / 2.0);
    let notch = (bbox.height / 4.0).min(hw * 0.8);
    scene.push(Primitive::Polygon {
        points: vec![
            Point::new(cx - hw, cy - hh),    // top-left
            Point::new(cx + hw, cy - hh),    // top-right
            Point::new(cx + hw, cy + hh),    // bottom-right
            Point::new(cx - hw, cy + hh),    // bottom-left
            Point::new(cx - hw + notch, cy), // left V-notch (pointing right)
        ],
        style,
    });
}

/// Subroutine: rect with double vertical bars (8px inset each side).
/// The bbox already includes the bar width (set in bridge sizing).
fn render_subroutine(bbox: BBox, style: Style, scene: &mut Scene) {
    let (cx, cy, w, h) = (bbox.x, bbox.y, bbox.width, bbox.height);
    let bar_inset = 8.0;
    scene.push(Primitive::Rect {
        bbox,
        rx: 0.0,
        ry: 0.0,
        style: style.clone(),
    });
    let top = cy - h / 2.0;
    let bottom = cy + h / 2.0;
    // Left inner vertical bar
    let left_bar = cx - w / 2.0 + bar_inset;
    scene.push(Primitive::Path {
        segments: vec![
            PathSegment::MoveTo(Point::new(left_bar, top)),
            PathSegment::LineTo(Point::new(left_bar, bottom)),
        ],
        style: Style {
            fill: None,
            stroke: style.stroke,
            stroke_width: style.stroke_width,
            ..Default::default()
        },
        marker_start: None,
        marker_end: None,
    });
    // Right inner vertical bar
    let right_bar = cx + w / 2.0 - bar_inset;
    scene.push(Primitive::Path {
        segments: vec![
            PathSegment::MoveTo(Point::new(right_bar, top)),
            PathSegment::LineTo(Point::new(right_bar, bottom)),
        ],
        style: Style {
            fill: None,
            stroke: style.stroke,
            stroke_width: style.stroke_width,
            ..Default::default()
        },
        marker_start: None,
        marker_end: None,
    });
}

#[cfg(test)]
mod render_tests;
