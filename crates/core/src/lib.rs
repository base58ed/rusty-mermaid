//! Foundation crate for rusty-mermaid: primitives, Scene, Theme, geometry, and text measurement.
//!
//! This crate defines the universal intermediate representation that all diagram
//! types produce and all rendering backends consume. The central type is [`Scene`],
//! a collection of [`Primitive`] drawing elements (rects, circles, paths, text, etc.)
//! that is completely backend-agnostic.
//!
//! # Key types
//!
//! - [`Scene`] / [`Primitive`] -- the contract between layout and rendering
//! - [`Theme`] / [`Style`] / [`TextStyle`] -- visual configuration
//! - [`Color`] / [`Point`] / [`BBox`] -- geometric primitives
//! - [`Shape`] -- node shape catalog (rect, diamond, circle, etc.)
//! - [`Direction`] -- layout flow direction (TB, BT, LR, RL)
//!
//! # Key traits
//!
//! - [`Renderer`] -- backends implement this to consume a [`Scene`]
//! - [`TextMeasure`] -- text dimension measurement for layout
//!
//! # Examples
//!
//! ```
//! use rusty_mermaid_core::{
//!     Scene, Primitive, Style, Color, Point, BBox, TextStyle, TextAnchor,
//! };
//!
//! let mut scene = Scene::new(200.0, 100.0);
//!
//! // Add a filled rectangle
//! scene.push(Primitive::Rect {
//!     bbox: BBox::new(100.0, 50.0, 120.0, 40.0),
//!     rx: 4.0,
//!     ry: 4.0,
//!     style: Style {
//!         fill: Some(Color::rgb(236, 236, 255)),
//!         stroke: Some(Color::rgb(147, 112, 219)),
//!         stroke_width: Some(2.0),
//!         ..Style::default()
//!     },
//! });
//!
//! // Add a text label
//! scene.push(Primitive::Text {
//!     position: Point::new(100.0, 50.0),
//!     content: "Hello".into(),
//!     anchor: TextAnchor::Middle,
//!     style: TextStyle::default(),
//! });
//!
//! assert_eq!(scene.len(), 2);
//! ```

pub mod constants;
pub mod curve;
pub mod font_fallback;
pub mod force_layout;
pub mod geometry;
pub mod latex;
pub mod marker_shapes;
pub mod renderer;
pub mod scene;
pub mod shape;
pub mod style;
pub mod text;
pub mod types;

pub use latex::strip_latex_to_unicode;

pub use curve::{CurveType, interpolate};
pub use geometry::{
    arc_sector_segments, intersect_circle, intersect_ellipse, intersect_line_circle,
    intersect_line_ellipse, intersect_polygon, intersect_rect,
};
pub use marker_shapes::{
    MarkerGeometry, MarkerPath, MarkerShape, marker_geometry, marker_path, transform_marker_circle,
    transform_marker_curves, transform_marker_points,
};
pub use renderer::Renderer;
pub use scene::{
    Element, ElementId, ElementKind, MarkerType, PathSegment, Primitive, Scene, TextAnchor,
    Transform, path_end_tangent, path_start_tangent,
};
pub use shape::Shape;
pub use style::{FontWeight, Style, TextStyle, Theme};
pub use text::{
    MdSpan, SimpleTextMeasure, TextMeasure, TextSize, parse_inline_markdown, text_baseline_y_offset,
};
pub use types::{BBox, Color, Direction, Point};
