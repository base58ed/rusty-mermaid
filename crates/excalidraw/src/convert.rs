//! Scene → Excalidraw element conversion.
//!
//! Two passes keep it O(n): pass one emits an element per primitive (minting a
//! clean, Excalidraw-safe id and indexing each source `ElementId` → its output
//! position); pass two resolves edge bindings through that index in O(1) each —
//! never a linear `Scene::find_by_id` per edge.

use std::collections::HashMap;

use rusty_mermaid_core::{
    EdgeBinding, ElementId, MarkerType, PathSegment, Point, Primitive, Scene, Style, TextAnchor,
    TextStyle, Theme,
};

use crate::color_hex;
use crate::element::{Binding, BoundElement, ElementKind, ExElement, Roundness};

/// Polyline steps when flattening a cubic/quadratic path segment. Fixed so the
/// output is deterministic (golden tests).
const CUBIC_STEPS: usize = 16;
/// Polyline steps approximating an `Arc`'s outer boundary.
const ARC_STEPS: usize = 24;

/// Convert a laid-out Scene into Excalidraw elements. Entry point. O(n + e).
pub fn scene_to_elements(scene: &Scene, theme: &Theme) -> Vec<ExElement> {
    let mut conv = Converter::new(theme);
    for el in scene.elements() {
        conv.emit(&el.primitive, el.id.as_ref());
    }
    conv.apply_bindings(scene.edge_bindings());
    conv.elements
}

struct Converter<'a> {
    theme: &'a Theme,
    elements: Vec<ExElement>,
    /// Source `ElementId` → index of its emitted element, for binding resolution.
    index: HashMap<ElementId, usize>,
    next: u32,
}

impl<'a> Converter<'a> {
    fn new(theme: &'a Theme) -> Self {
        Self {
            theme,
            elements: Vec::new(),
            index: HashMap::new(),
            next: 0,
        }
    }

    /// A fresh, Excalidraw- and diagrammy-safe element id (charset `[A-Za-z0-9-]`).
    /// Sequential, so the output is deterministic for golden tests.
    fn mint_id(&mut self) -> String {
        let id = format!("rm-{}", self.next);
        self.next += 1;
        id
    }

    /// Emit element(s) for one primitive. A `Group` flattens to its children
    /// sharing a `groupId`; everything else is one element. A `source` id (the
    /// node's primary primitive, or an edge's path) is indexed for binding.
    fn emit(&mut self, primitive: &Primitive, source: Option<&ElementId>) {
        if let Primitive::Group { children, .. } = primitive {
            let gid = self.mint_id();
            let group_start = self.elements.len();
            for child in children {
                let before = self.elements.len();
                self.emit(child, None);
                for el in &mut self.elements[before..] {
                    el.group_ids.push(gid.clone());
                }
            }
            // A group carrying a source id (a node rendered as a group) is bound
            // via its first emitted child, so edges to it still resolve.
            if let Some(src) = source
                && self.elements.len() > group_start
            {
                self.index.insert(src.clone(), group_start);
            }
            return;
        }
        let Some(el) = self.map_primitive(primitive) else {
            return;
        };
        let out = self.elements.len();
        self.elements.push(el);
        if let Some(src) = source {
            self.index.insert(src.clone(), out);
        }
    }

    /// Map a single (non-Group) primitive to one Excalidraw element.
    fn map_primitive(&mut self, primitive: &Primitive) -> Option<ExElement> {
        match primitive {
            Primitive::Rect {
                bbox, rx, style, ..
            } => {
                let roundness = (*rx > 0.0).then_some(Roundness { kind: 3 });
                Some(self.shape(
                    bbox.left(),
                    bbox.top(),
                    bbox.width,
                    bbox.height,
                    style,
                    ElementKind::Rectangle,
                    roundness,
                ))
            }
            Primitive::Circle {
                center,
                radius,
                style,
            } => Some(self.shape(
                center.x - radius,
                center.y - radius,
                radius * 2.0,
                radius * 2.0,
                style,
                ElementKind::Ellipse,
                None,
            )),
            Primitive::Ellipse {
                center,
                rx,
                ry,
                style,
            } => Some(self.shape(
                center.x - rx,
                center.y - ry,
                rx * 2.0,
                ry * 2.0,
                style,
                ElementKind::Ellipse,
                None,
            )),
            Primitive::Polygon { points, style } => {
                let mut pts: Vec<[f64; 2]> = points.iter().map(|p| [p.x, p.y]).collect();
                if pts.first() != pts.last()
                    && let Some(first) = pts.first().copied()
                {
                    pts.push(first); // close the polygon
                }
                (pts.len() >= 2).then(|| self.polyline(&pts, style, None, None))
            }
            Primitive::Path {
                segments,
                style,
                marker_start,
                marker_end,
            } => {
                let pts = flatten(segments);
                (pts.len() >= 2).then(|| self.polyline(&pts, style, *marker_start, *marker_end))
            }
            Primitive::Arc {
                center,
                outer_r,
                start_angle,
                end_angle,
                style,
                ..
            } => {
                let pts = arc_points(*center, *outer_r, *start_angle, *end_angle);
                (pts.len() >= 2).then(|| self.polyline(&pts, style, None, None))
            }
            Primitive::Text {
                position,
                content,
                anchor,
                style,
            } => Some(self.text(*position, content, *anchor, style)),
            Primitive::Group { .. } => None, // handled in emit()
        }
    }

    /// Build a box-shaped element (rectangle / ellipse / diamond).
    #[allow(clippy::too_many_arguments)]
    fn shape(
        &mut self,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        style: &Style,
        kind: ElementKind,
        roundness: Option<Roundness>,
    ) -> ExElement {
        let mut el = self.base(x, y, w, h, style, kind);
        el.roundness = roundness;
        el
    }

    /// Build a connector element from absolute points. With a marker it becomes
    /// an `arrow` (bindable); otherwise a plain `line`. Points are stored
    /// relative to the element's `(x, y)` (the first point), per Excalidraw.
    fn polyline(
        &mut self,
        points: &[[f64; 2]],
        style: &Style,
        marker_start: Option<MarkerType>,
        marker_end: Option<MarkerType>,
    ) -> ExElement {
        let (ox, oy) = (points[0][0], points[0][1]);
        let rel: Vec<[f64; 2]> = points.iter().map(|p| [p[0] - ox, p[1] - oy]).collect();
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
        for p in &rel {
            min_x = min_x.min(p[0]);
            max_x = max_x.max(p[0]);
            min_y = min_y.min(p[1]);
            max_y = max_y.max(p[1]);
        }
        let kind = if marker_start.is_some() || marker_end.is_some() {
            ElementKind::Arrow {
                points: rel,
                last_committed_point: None,
                start_binding: None,
                end_binding: None,
                start_arrowhead: marker_start.map(arrowhead),
                end_arrowhead: marker_end.map(arrowhead),
            }
        } else {
            ElementKind::Line {
                points: rel,
                last_committed_point: None,
            }
        };
        self.base(ox, oy, max_x - min_x, max_y - min_y, style, kind)
    }

    /// Build a text element. Excalidraw anchors text at top-left, so the source
    /// anchor point shifts by the estimated extent.
    fn text(
        &mut self,
        pos: Point,
        content: &str,
        anchor: TextAnchor,
        style: &TextStyle,
    ) -> ExElement {
        let font_size = style.font_size;
        let line_height = 1.25;
        let width = content.chars().count() as f64 * font_size * 0.55;
        let height = font_size * line_height;
        let x = match anchor {
            TextAnchor::Start => pos.x,
            TextAnchor::Middle => pos.x - width / 2.0,
            TextAnchor::End => pos.x - width,
        };
        let y = pos.y - height / 2.0; // vertical-center anchor → top
        let text_align = match anchor {
            TextAnchor::Start => "left",
            TextAnchor::Middle => "center",
            TextAnchor::End => "right",
        };
        // The shared Style fill is the text color; reuse the box builder, then
        // overwrite stroke with the text color (Excalidraw text uses strokeColor).
        let pseudo = Style {
            stroke: style.fill,
            ..Default::default()
        };
        self.base(
            x,
            y,
            width,
            height,
            &pseudo,
            ElementKind::Text {
                text: content.to_string(),
                font_size,
                font_family: font_family(&style.font_family),
                text_align: text_align.to_string(),
                vertical_align: "middle".to_string(),
                container_id: None,
                line_height,
                original_text: content.to_string(),
            },
        )
    }

    /// Common element fields with Excalidraw defaults + translated style.
    fn base(
        &mut self,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        style: &Style,
        kind: ElementKind,
    ) -> ExElement {
        let n = self.next;
        let id = self.mint_id();
        ExElement {
            id,
            x,
            y,
            width: w,
            height: h,
            angle: 0.0,
            stroke_color: style
                .stroke
                .map_or_else(|| color_hex(self.theme.node_stroke), color_hex),
            background_color: style
                .fill
                .map_or_else(|| "transparent".to_string(), color_hex),
            fill_style: "solid".to_string(),
            stroke_width: style
                .stroke_width
                .unwrap_or(self.theme.default_stroke_width),
            stroke_style: if style.stroke_dasharray.is_some() {
                "dashed".to_string()
            } else {
                "solid".to_string()
            },
            roughness: 0, // clean lines — mermaid geometry is precise
            opacity: opacity_pct(style.opacity),
            group_ids: Vec::new(),
            roundness: None,
            // Deterministic per element so output is reproducible and
            // golden-testable. A consumer merging several diagrams onto one
            // canvas should re-stamp seed/versionNonce to keep them unique.
            seed: n.wrapping_mul(2_654_435_761).wrapping_add(1),
            version: 1,
            version_nonce: n.wrapping_mul(40_503).wrapping_add(7),
            is_deleted: false,
            bound_elements: Vec::new(),
            updated: 0,
            locked: false,
            kind,
        }
    }

    /// Pass 2: turn each `EdgeBinding` into a real arrow start/end binding plus
    /// the shapes' `boundElements`. O(1) per binding via the id index.
    fn apply_bindings(&mut self, bindings: &[EdgeBinding]) {
        for b in bindings {
            let (Some(&ei), Some(&si), Some(&di)) = (
                self.index.get(&b.edge),
                self.index.get(&b.src),
                self.index.get(&b.dst),
            ) else {
                continue; // an endpoint or edge that didn't emit an element
            };
            // Bind each end ONLY to a bindable host. Excalidraw refuses a binding
            // to a `line` (a diamond/polygon node lowers to one) and drops it on
            // import — so emitting one would be a dead binding the canvas silently
            // discards. Per-endpoint binding keeps start/end + boundElements
            // symmetric for whichever ends actually bind.
            let bind_start = is_bindable(&self.elements[si].kind);
            let bind_end = is_bindable(&self.elements[di].kind);
            if !bind_start && !bind_end {
                continue; // neither endpoint can bind — leave the edge as it lowered
            }
            // A bound edge must be an Excalidraw `arrow` to carry the binding. A
            // markerless connector (an ER relationship, a mindmap/treeview link, a
            // plain class association) lowered to a `line`; upgrade it in place to
            // a headless arrow — visually identical, but now bindable. Anything
            // that still isn't an arrow (a stray shape id) is skipped.
            upgrade_line_to_arrow(&mut self.elements[ei].kind);
            if !matches!(self.elements[ei].kind, ElementKind::Arrow { .. }) {
                continue;
            }
            let arrow_id = self.elements[ei].id.clone();
            let src_id = self.elements[si].id.clone();
            let dst_id = self.elements[di].id.clone();
            if let ElementKind::Arrow {
                start_binding,
                end_binding,
                ..
            } = &mut self.elements[ei].kind
            {
                if bind_start {
                    *start_binding = Some(Binding {
                        element_id: src_id,
                        focus: 0.0,
                        gap: 4.0,
                    });
                }
                if bind_end {
                    *end_binding = Some(Binding {
                        element_id: dst_id,
                        focus: 0.0,
                        gap: 4.0,
                    });
                }
            }
            if bind_start {
                self.elements[si].bound_elements.push(BoundElement {
                    id: arrow_id.clone(),
                    kind: "arrow".to_string(),
                });
            }
            // Self-loop dedup: the same shape on both ends lists the arrow once.
            if bind_end && di != si {
                self.elements[di].bound_elements.push(BoundElement {
                    id: arrow_id,
                    kind: "arrow".to_string(),
                });
            }
        }
    }
}

/// Upgrade a markerless connector `Line` to a headless `Arrow` in place, so a
/// recorded edge binding can attach to it. A no-op for any other kind (already
/// an arrow, or a shape). The geometry is preserved; only bindings (which an
/// Excalidraw `line` cannot hold) become possible.
fn upgrade_line_to_arrow(kind: &mut ElementKind) {
    let ElementKind::Line {
        points,
        last_committed_point,
    } = kind
    else {
        return;
    };
    *kind = ElementKind::Arrow {
        points: std::mem::take(points),
        last_committed_point: last_committed_point.take(),
        start_binding: None,
        end_binding: None,
        start_arrowhead: None,
        end_arrowhead: None,
    };
}

/// Whether an Excalidraw element kind can host an arrow binding. Excalidraw
/// rejects bindings to `line`/`arrow`; a diamond/polygon node lowers to `line`.
fn is_bindable(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::Rectangle
            | ElementKind::Ellipse
            | ElementKind::Diamond
            | ElementKind::Text { .. }
    )
}

/// Flatten path segments to an absolute polyline (cubics/quads → `CUBIC_STEPS`
/// line steps, arcs → endpoint). O(segments · steps).
fn flatten(segments: &[PathSegment]) -> Vec<[f64; 2]> {
    let mut pts: Vec<[f64; 2]> = Vec::new();
    let mut cur = Point::new(0.0, 0.0);
    let push = |p: Point, pts: &mut Vec<[f64; 2]>| {
        if pts.last() != Some(&[p.x, p.y]) {
            pts.push([p.x, p.y]);
        }
    };
    for seg in segments {
        match seg {
            PathSegment::MoveTo(p) | PathSegment::LineTo(p) => {
                cur = *p;
                push(*p, &mut pts);
            }
            PathSegment::CubicTo { cp1, cp2, to } => {
                for i in 1..=CUBIC_STEPS {
                    let t = i as f64 / CUBIC_STEPS as f64;
                    push(cubic(cur, *cp1, *cp2, *to, t), &mut pts);
                }
                cur = *to;
            }
            PathSegment::QuadTo { cp, to } => {
                for i in 1..=CUBIC_STEPS {
                    let t = i as f64 / CUBIC_STEPS as f64;
                    push(quad(cur, *cp, *to, t), &mut pts);
                }
                cur = *to;
            }
            PathSegment::ArcTo { to, .. } => {
                cur = *to;
                push(*to, &mut pts);
            }
            PathSegment::Close => {
                if let Some(first) = pts.first().copied() {
                    push(Point::new(first[0], first[1]), &mut pts);
                }
            }
        }
    }
    pts
}

fn cubic(p0: Point, c1: Point, c2: Point, p1: Point, t: f64) -> Point {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Point::new(
        a * p0.x + b * c1.x + c * c2.x + d * p1.x,
        a * p0.y + b * c1.y + c * c2.y + d * p1.y,
    )
}

fn quad(p0: Point, c: Point, p1: Point, t: f64) -> Point {
    let u = 1.0 - t;
    Point::new(
        u * u * p0.x + 2.0 * u * t * c.x + t * t * p1.x,
        u * u * p0.y + 2.0 * u * t * c.y + t * t * p1.y,
    )
}

/// Sample an arc's outer boundary as a polyline.
fn arc_points(center: Point, r: f64, start: f64, end: f64) -> Vec<[f64; 2]> {
    (0..=ARC_STEPS)
        .map(|i| {
            let a = start + (end - start) * (i as f64 / ARC_STEPS as f64);
            [center.x + r * a.cos(), center.y + r * a.sin()]
        })
        .collect()
}

/// Map a rusty-mermaid marker to an Excalidraw arrowhead name.
///
/// Excalidraw's `triangle` is a SOLID filled head and `arrow` is an open stick,
/// so a filled marker must map to `triangle` and an open one to `arrow`. These
/// two were transposed, which inverted the very distinction the notation
/// carries: mermaid's `->>` (filled) drew a thin stick while `->` (open) drew a
/// solid head. Every arm stays inside the `arrow|triangle|dot|bar|diamond`
/// vocabulary that downstream consumers can decode.
fn arrowhead(m: MarkerType) -> String {
    match m {
        // Filled heads: the emphatic marker (mermaid `->>`, UML generalization).
        MarkerType::ArrowPoint | MarkerType::Extension => "triangle",
        // Open heads: a plain stick (mermaid `->` / `-)`, UML dependency).
        MarkerType::ArrowBarb | MarkerType::ArrowOpen | MarkerType::Dependency => "arrow",
        MarkerType::Circle | MarkerType::Aggregation => "dot",
        MarkerType::Cross => "bar",
        MarkerType::Composition => "diamond",
        _ => "arrow", // any other crow's-foot / future marker → a plain arrowhead
    }
    .to_string()
}

/// Map a font-family name to Excalidraw's integer family (1 hand-drawn, 2
/// normal, 3 code).
fn font_family(name: &str) -> u8 {
    if name.contains("mono") || name.contains("code") {
        3
    } else {
        2
    }
}

/// Style opacity (0.0–1.0) → Excalidraw percentage (0–100). Default 100.
fn opacity_pct(opacity: Option<f64>) -> u8 {
    opacity.map_or(100, |o| (o.clamp(0.0, 1.0) * 100.0).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A filled marker draws Excalidraw's SOLID `triangle` and an open one the
    /// stick `arrow`. The two were transposed, so mermaid's `->>` rendered thin
    /// and `->` rendered solid — the notation's meaning inverted. (Mutation:
    /// swap the two arms back → the first two assertions go red.)
    #[test]
    fn arrowhead_maps_filled_to_solid_and_open_to_stick() {
        assert_eq!(arrowhead(MarkerType::ArrowPoint), "triangle");
        assert_eq!(arrowhead(MarkerType::ArrowOpen), "arrow");
        assert_eq!(arrowhead(MarkerType::ArrowBarb), "arrow");
        assert_eq!(arrowhead(MarkerType::Dependency), "arrow");
        assert_eq!(arrowhead(MarkerType::Composition), "diamond");
        assert_eq!(arrowhead(MarkerType::Cross), "bar");
    }

    /// Every marker maps into the arrowhead vocabulary Excalidraw and its
    /// consumers can decode. A name outside this set (say `triangle_outline`)
    /// deserializes to nothing downstream and breaks the whole import, so the
    /// set is a hard boundary rather than a style preference.
    #[test]
    fn arrowhead_never_leaves_the_decodable_vocabulary() {
        for m in [
            MarkerType::ArrowPoint,
            MarkerType::ArrowBarb,
            MarkerType::ArrowOpen,
            MarkerType::Circle,
            MarkerType::Cross,
            MarkerType::Aggregation,
            MarkerType::Composition,
            MarkerType::Dependency,
            MarkerType::Extension,
        ] {
            let name = arrowhead(m);
            assert!(
                matches!(
                    name.as_str(),
                    "arrow" | "triangle" | "dot" | "bar" | "diamond"
                ),
                "{m:?} produced {name}, outside the decodable vocabulary",
            );
        }
    }
}
