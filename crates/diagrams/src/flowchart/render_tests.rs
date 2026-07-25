use super::*;
use crate::common::rendering::{MARKER_INSET_VB, marker_inset_px, prev_endpoint};
use crate::common::test_helpers::test_helpers::*;

#[test]
fn simple_flowchart_to_scene() {
    let d = crate::flowchart::parser::parse("graph TD\n    A --> B").unwrap();
    let layout = crate::flowchart::bridge::layout(&d);
    let scene = to_scene(&layout, &Theme::default());

    assert_scene_valid(&scene);

    let elems = scene.elements();
    // At minimum: 2 nodes (Rect + Text each) + 1 edge (Path)
    assert!(
        elems.len() >= 5,
        "expected at least 5 primitives, got {}",
        elems.len()
    );

    assert!(
        has_rect(&scene),
        "scene should contain Rect primitives for nodes"
    );
    assert!(
        has_path(&scene),
        "scene should contain Path primitives for edges"
    );
    assert!(
        count_texts(&scene) > 0,
        "scene should contain Text primitives for labels"
    );
}

#[test]
fn nodes_and_edges_carry_ids_and_bindings() {
    let d = crate::flowchart::parser::parse("graph TD\n    A --> B").unwrap();
    let layout = crate::flowchart::bridge::layout(&d);
    let scene = to_scene(&layout, &Theme::default());

    // The edge binding resolves the A-->B edge to its endpoint nodes.
    let bindings = scene.edge_bindings();
    assert_eq!(bindings.len(), 1, "one edge binding for A-->B");
    assert_eq!(bindings[0].src, ElementId::node("A"));
    assert_eq!(bindings[0].dst, ElementId::node("B"));

    // Both endpoint nodes resolve to real scene elements, and so does the edge —
    // so the Excalidraw backend can bind the arrow to its source/target.
    assert!(
        scene.find_by_id(&ElementId::node("A")).is_some(),
        "node A element exists"
    );
    assert!(
        scene.find_by_id(&ElementId::node("B")).is_some(),
        "node B element exists"
    );
    assert!(
        scene.find_by_id(&bindings[0].edge).is_some(),
        "the bound edge element exists"
    );
}

#[test]
fn diamond_node_produces_polygon() {
    let d = crate::flowchart::parser::parse("flowchart TD\n    A{Decision}").unwrap();
    let layout = crate::flowchart::bridge::layout(&d);
    let scene = to_scene(&layout, &Theme::default());

    assert!(
        count_polygons(&scene) > 0,
        "diamond shape should produce at least one Polygon primitive"
    );
    // Diamond has exactly 4 points
    let polygons: Vec<_> = scene
        .elements()
        .iter()
        .filter(|e| matches!(e.primitive, Primitive::Polygon { .. }))
        .collect();
    if let Primitive::Polygon { points, .. } = &polygons[0].primitive {
        assert_eq!(points.len(), 4, "diamond polygon should have 4 vertices");
    }
}

#[test]
fn circle_node_produces_circle() {
    let d = crate::flowchart::parser::parse("flowchart TD\n    A((Round))").unwrap();
    let layout = crate::flowchart::bridge::layout(&d);
    let scene = to_scene(&layout, &Theme::default());

    assert!(
        has_circle(&scene),
        "circle shape should produce Circle primitive"
    );
    let circles: Vec<_> = scene
        .elements()
        .iter()
        .filter(|e| matches!(e.primitive, Primitive::Circle { .. }))
        .collect();
    if let Primitive::Circle { radius, .. } = &circles[0].primitive {
        assert!(*radius > 0.0);
    }
}

#[test]
fn edges_produce_paths_with_markers() {
    let d = crate::flowchart::parser::parse("flowchart TD\n    A --> B").unwrap();
    let layout = crate::flowchart::bridge::layout(&d);
    let scene = to_scene(&layout, &Theme::default());

    let edge_paths: Vec<_> = scene
        .elements()
        .iter()
        .filter(|e| {
            matches!(
                e.primitive,
                Primitive::Path {
                    marker_end: Some(MarkerType::ArrowPoint),
                    ..
                }
            )
        })
        .collect();
    assert_eq!(
        edge_paths.len(),
        1,
        "one edge should produce one Path with ArrowPoint marker"
    );
}

/// A subgraph box carries its own `ElementId`, so an edge whose endpoint IS the
/// subgraph can resolve that endpoint and bind to it. Unidentified, the box is
/// invisible to binding resolution: the arrow lowers unbound, no consumer
/// treats it as a graph edge, and it keeps stale coordinates while the nodes
/// around it move — the disconnected tails seen in rendered output.
/// (Mutation: push the rect without its id → this goes red.)
#[test]
fn subgraph_box_is_identified_so_edges_can_bind_to_it() {
    let d = crate::flowchart::parser::parse(
        "flowchart TD\n    A --> G\n    subgraph G[Group]\n        B --> C\n    end",
    )
    .unwrap();
    let layout = crate::flowchart::bridge::layout(&d);
    let scene = to_scene(&layout, &Theme::default());

    let boxed = scene
        .elements()
        .iter()
        .find(|e| e.id.as_ref() == Some(&ElementId::node("G")));
    assert!(
        boxed.is_some(),
        "the subgraph box must carry ElementId::node(\"G\") for binding resolution",
    );
    assert!(
        matches!(boxed.map(|e| &e.primitive), Some(Primitive::Rect { .. })),
        "the identified subgraph element is its boundary rect",
    );
}

#[test]
fn subgraph_produces_background_rect() {
    let mmd = "flowchart TD\n    subgraph sg[My Group]\n        A --> B\n    end";
    let d = crate::flowchart::parser::parse(mmd).unwrap();
    let layout = crate::flowchart::bridge::layout(&d);
    let scene = to_scene(&layout, &Theme::default());

    // Subgraph rect is rendered first, before node rects.
    // Count all rects: should be at least 3 (1 subgraph + 2 nodes).
    assert!(
        count_rects(&scene) >= 3,
        "expected at least 3 Rects (1 subgraph bg + 2 nodes), got {}",
        count_rects(&scene)
    );

    // First rect should be the subgraph background (rendered before nodes)
    let rects: Vec<_> = scene
        .elements()
        .iter()
        .filter(|e| matches!(e.primitive, Primitive::Rect { .. }))
        .collect();
    if let Primitive::Rect { rx, ry, .. } = &rects[0].primitive {
        assert!(
            (*rx - 5.0).abs() < f64::EPSILON,
            "subgraph rect should have rx=5"
        );
        assert!(
            (*ry - 5.0).abs() < f64::EPSILON,
            "subgraph rect should have ry=5"
        );
    }

    // Subgraph label text should appear
    assert!(
        has_text(&scene, "My Group"),
        "subgraph label text should be in scene"
    );
}

#[test]
fn nested_subgraphs_render_outermost_first() {
    let mmd = "flowchart TD\n    subgraph outer[Outer]\n        subgraph middle[Middle]\n            subgraph inner[Inner]\n                A --> B\n            end\n        end\n    end";
    let d = crate::flowchart::parser::parse(mmd).unwrap();
    let layout = crate::flowchart::bridge::layout(&d);
    let scene = to_scene(&layout, &Theme::default());

    // Collect subgraph rect areas in scene order (subgraphs render before nodes)
    let subgraph_areas: Vec<f64> = scene
        .elements()
        .iter()
        .filter_map(|e| {
            if let Primitive::Rect { bbox, rx, .. } = &e.primitive {
                // Subgraph rects use rx=5 and are larger than leaf nodes
                if *rx == 5.0 && bbox.width * bbox.height > 3000.0 {
                    return Some(bbox.width * bbox.height);
                }
            }
            None
        })
        .collect();

    assert!(
        subgraph_areas.len() >= 3,
        "expected at least 3 subgraph rects, got {}",
        subgraph_areas.len()
    );
    for w in subgraph_areas.windows(2) {
        assert!(
            w[0] >= w[1],
            "subgraph rects must be ordered largest-first for correct z-ordering: {:.0} < {:.0}",
            w[0],
            w[1]
        );
    }
}

#[test]
fn empty_layout_produces_empty_scene() {
    let layout = crate::flowchart::bridge::LayoutResult {
        nodes: vec![],
        edges: vec![],
        subgraphs: vec![],
        width: 0.0,
        height: 0.0,
    };
    let scene = to_scene(&layout, &Theme::default());
    assert!(scene.is_empty());
    assert!((scene.width - 0.0).abs() < f64::EPSILON);
    assert!((scene.height - 0.0).abs() < f64::EPSILON);
}

#[test]
fn themed_scene_has_edge_paths() {
    let d = crate::flowchart::parser::parse("graph TD\n    A --> B").unwrap();
    let layout = crate::flowchart::bridge::layout(&d);
    let theme = Theme::default();
    let scene = to_scene(&layout, &theme);

    let has_edge_path = scene.elements().iter().any(|e| {
        matches!(
            e.primitive,
            Primitive::Path {
                marker_end: Some(_),
                ..
            }
        )
    });
    assert!(
        has_edge_path,
        "themed scene should have edge paths with markers"
    );
}

#[test]
fn marker_inset_all_markers_have_positive_inset() {
    // Every marker gets a uniform positive inset so the stroke butt-cap
    // hides behind the marker body.
    for m in [
        MarkerType::ArrowPoint,
        MarkerType::ArrowBarb,
        MarkerType::ArrowOpen,
        MarkerType::Cross,
        MarkerType::Circle,
        MarkerType::Aggregation,
        MarkerType::Composition,
        MarkerType::Dependency,
    ] {
        assert!(MARKER_INSET_VB > 0.0, "viewBox inset must be positive");
        assert!(
            marker_inset_px(m, 1.5) > 0.0,
            "{m:?} must produce positive pixel inset at normal stroke"
        );
        assert!(
            marker_inset_px(m, 3.5) > 0.0,
            "{m:?} must produce positive pixel inset at thick stroke"
        );
    }
}

#[test]
fn marker_inset_px_scales_with_stroke_width() {
    let normal = marker_inset_px(MarkerType::ArrowPoint, 1.5);
    let thick = marker_inset_px(MarkerType::ArrowPoint, 3.5);
    assert!(thick > normal, "thick stroke should produce larger inset");
    // inset = vb * 0.8 * sw → thick/normal = 3.5/1.5
    let ratio = thick / normal;
    assert!(
        (ratio - 3.5 / 1.5).abs() < 0.01,
        "inset ratio should match stroke ratio"
    );
}

#[test]
fn edge_path_shortened_for_arrow_marker() {
    // An edge with ArrowPoint marker should have its endpoint pulled back.
    let d = crate::flowchart::parser::parse("graph TD\n    A --> B").unwrap();
    let layout = crate::flowchart::bridge::layout(&d);
    let scene = to_scene(&layout, &Theme::default());

    // Find the target node's top boundary and the edge endpoint
    let target_node = &layout.nodes.iter().find(|n| n.label == "B").unwrap();
    let node_top = target_node.y - target_node.height / 2.0;

    // The edge path endpoint should be ABOVE the node boundary (shortened)
    for e in scene.elements() {
        if let Primitive::Path {
            segments,
            marker_end: Some(MarkerType::ArrowPoint),
            style,
            ..
        } = &e.primitive
        {
            let endpoint = prev_endpoint(segments).unwrap();
            let sw = style.stroke_width.unwrap_or(1.5);
            let expected_inset = marker_inset_px(MarkerType::ArrowPoint, sw);
            let gap = node_top - endpoint.y;
            assert!(
                gap > 0.0,
                "edge endpoint ({:.1}) should be above node boundary ({:.1})",
                endpoint.y,
                node_top
            );
            assert!(
                (gap - expected_inset).abs() < 1.0,
                "gap ({gap:.1}) should be ~{expected_inset:.1}px"
            );
        }
    }
}

#[test]
fn edge_path_shortened_for_circle_marker() {
    // All markers use the same uniform inset.
    let d = crate::flowchart::parser::parse("graph TD\n    A --o B").unwrap();
    let layout = crate::flowchart::bridge::layout(&d);
    let scene = to_scene(&layout, &Theme::default());

    let target_node = &layout.nodes.iter().find(|n| n.label == "B").unwrap();
    let node_top = target_node.y - target_node.height / 2.0;

    for e in scene.elements() {
        if let Primitive::Path {
            segments,
            marker_end: Some(MarkerType::Circle),
            style,
            ..
        } = &e.primitive
        {
            let endpoint = prev_endpoint(segments).unwrap();
            let sw = style.stroke_width.unwrap_or(1.5);
            let expected_inset = marker_inset_px(MarkerType::Circle, sw);
            let gap = node_top - endpoint.y;
            assert!(
                gap > 0.0,
                "edge endpoint ({:.1}) should be above node boundary ({:.1})",
                endpoint.y,
                node_top
            );
            assert!(
                (gap - expected_inset).abs() < 1.0,
                "gap ({gap:.1}) should be ~{expected_inset:.1}px"
            );
        }
    }
}

#[test]
fn edge_path_shortened_for_cross_marker() {
    let d = crate::flowchart::parser::parse("graph TD\n    A --x B").unwrap();
    let layout = crate::flowchart::bridge::layout(&d);
    let scene = to_scene(&layout, &Theme::default());

    let target_node = &layout.nodes.iter().find(|n| n.label == "B").unwrap();
    let node_top = target_node.y - target_node.height / 2.0;

    for e in scene.elements() {
        if let Primitive::Path {
            segments,
            marker_end: Some(MarkerType::Cross),
            style,
            ..
        } = &e.primitive
        {
            let endpoint = prev_endpoint(segments).unwrap();
            let sw = style.stroke_width.unwrap_or(1.5);
            let expected_inset = marker_inset_px(MarkerType::Cross, sw);
            let gap = node_top - endpoint.y;
            assert!(
                gap > 0.0,
                "edge endpoint ({:.1}) should be above node boundary ({:.1})",
                endpoint.y,
                node_top
            );
            assert!(
                (gap - expected_inset).abs() < 1.0,
                "gap ({gap:.1}) should be ~{expected_inset:.1}px"
            );
        }
    }
}

#[test]
fn subroutine_edge_terminates_at_visual_boundary() {
    // The subroutine shape has 8px decorative bars inset from each side.
    // Edge endpoints must land at the outer rect boundary, not at the
    // inner bar position. Regression: arrows were clipped behind the node
    // when dagre used the wrong (smaller) width for intersection.
    let d = crate::flowchart::parser::parse("flowchart LR\n    A --> B[[Process]]\n    B --> C")
        .unwrap();
    let layout = crate::flowchart::bridge::layout(&d);
    let scene = to_scene(&layout, &Theme::default());

    // Find the subroutine rect (the one with rx=0, ry=0 whose width is larger)
    let sub_rects: Vec<_> = scene
        .elements()
        .iter()
        .filter_map(|e| {
            if let Primitive::Rect { bbox, rx, .. } = &e.primitive {
                if *rx == 0.0 { Some(bbox) } else { None }
            } else {
                None
            }
        })
        .collect();
    assert!(!sub_rects.is_empty(), "should have subroutine rect");
    let sub = sub_rects
        .iter()
        .max_by(|a, b| a.width.partial_cmp(&b.width).unwrap())
        .unwrap();
    let sub_left = sub.x - sub.width / 2.0;
    let sub_right = sub.x + sub.width / 2.0;

    // Find edge paths with markers
    let edge_paths: Vec<_> = scene
        .elements()
        .iter()
        .filter_map(|e| {
            if let Primitive::Path {
                segments,
                marker_end: Some(_),
                ..
            } = &e.primitive
            {
                Some(segments)
            } else {
                None
            }
        })
        .collect();
    assert!(edge_paths.len() >= 2, "should have at least 2 edge paths");

    // Inbound edge: endpoint should be at or just outside sub_left
    let inbound_end = prev_endpoint(&edge_paths[0]).unwrap();
    assert!(
        inbound_end.x <= sub_left + 1.0,
        "inbound edge endpoint ({:.1}) should be at or outside subroutine left ({:.1})",
        inbound_end.x,
        sub_left
    );

    // Outbound edge: start should be at or just outside sub_right
    let outbound_start = match edge_paths[1][0] {
        PathSegment::MoveTo(p) => p,
        _ => panic!("expected MoveTo"),
    };
    assert!(
        outbound_start.x >= sub_right - 1.0,
        "outbound edge start ({:.1}) should be at or outside subroutine right ({:.1})",
        outbound_start.x,
        sub_right
    );
}

#[test]
fn edges_render_behind_nodes() {
    // Edges must render BEHIND nodes so node shapes cleanly cover any
    // marker overshoot. This matches state diagram and mermaid.js z-order.
    let d = crate::flowchart::parser::parse("flowchart TD\n    A --o B\n    A --x C").unwrap();
    let layout = crate::flowchart::bridge::layout(&d);
    let scene = to_scene(&layout, &Theme::default());

    let last_edge_idx = scene
        .elements()
        .iter()
        .rposition(|e| {
            matches!(
                e.primitive,
                Primitive::Path {
                    marker_end: Some(_),
                    ..
                }
            )
        })
        .expect("should have edge paths with markers");
    let first_node_after_edges = scene.elements().iter().enumerate().position(|(i, e)| {
        i > last_edge_idx
            && matches!(
                e.primitive,
                Primitive::Rect { .. } | Primitive::Circle { .. } | Primitive::Polygon { .. }
            )
    });
    assert!(
        first_node_after_edges.is_some(),
        "node shapes must render after edge paths (last edge idx {last_edge_idx}) \
             so nodes cover edges"
    );
}
