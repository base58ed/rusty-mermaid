use crate::TextStyle;

/// Measured text dimensions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextSize {
    pub width: f64,
    pub height: f64,
}

/// Measure text dimensions for layout purposes.
/// Generic parameter on dagre::layout — no vtable overhead.
pub trait TextMeasure {
    fn measure(&self, text: &str, style: &TextStyle) -> TextSize;
}

/// Monospace character width at the reference font size (14px).
/// Measured from Intel One Mono Regular: 8.596px at 14px.
/// Rounded up to ensure node boxes never undersize the text.
const MONOSPACE_CHAR_WIDTH_14PX: f64 = 8.6;
const MONOSPACE_LINE_HEIGHT_14PX: f64 = 16.8;

/// Monospace text measurer using fixed character advance width.
/// Uses accurate metrics for the default monospace font stack.
pub struct SimpleTextMeasure {
    pub avg_char_width: f64,
    pub line_height: f64,
}

impl Default for SimpleTextMeasure {
    fn default() -> Self {
        Self {
            avg_char_width: MONOSPACE_CHAR_WIDTH_14PX,
            line_height: MONOSPACE_LINE_HEIGHT_14PX,
        }
    }
}

impl SimpleTextMeasure {
    pub fn new(avg_char_width: f64, line_height: f64) -> Self {
        debug_assert!(avg_char_width > 0.0, "char width must be positive");
        debug_assert!(line_height > 0.0, "line height must be positive");
        Self {
            avg_char_width,
            line_height,
        }
    }
}

use crate::font_fallback::{FontSlot, font_for_char};

/// Width multiplier per FontSlot relative to Intel One Mono's advance.
/// These ratios account for the actual advance widths of each fallback font.
pub const fn char_width_ratio(ch: char) -> f64 {
    match font_for_char(ch) {
        FontSlot::Primary => 1.0,       // Intel One Mono: 0.6em monospace
        FontSlot::ExtendedText => 0.85, // Noto Sans: proportional, narrower than mono
        FontSlot::Monospace => 1.0,     // Noto Sans Mono: same width as primary
        FontSlot::Dingbats => 1.4,      // Noto Sans Symbols 2: wider symbols
        FontSlot::Arabic => 0.8,        // Noto Naskh Arabic: proportional, varies
        FontSlot::Cjk => 1.8,           // CJK: wide but proportional
        FontSlot::Emoji => 2.0,         // Color emoji: ~2x Latin mono width
    }
}

impl SimpleTextMeasure {
    /// Measure text width/height WITHOUT stripping HTML/markdown markup.
    /// Use for text that contains literal `<` `>` characters (e.g. `<<interface>>`).
    pub fn measure_raw(text: &str, style: &TextStyle) -> TextSize {
        let defaults = Self::default();
        let scale = style.font_size / crate::constants::REFERENCE_FONT_SIZE;
        let mut max_width: f64 = 0.0;
        let mut line_count: usize = 0;
        for line in text.split('\n') {
            line_count += 1;
            let w: f64 = line.chars().map(char_width_ratio).sum();
            max_width = max_width.max(w);
        }
        TextSize {
            width: max_width * defaults.avg_char_width * scale,
            height: line_count as f64 * defaults.line_height * scale,
        }
    }
}

impl TextMeasure for SimpleTextMeasure {
    fn measure(&self, text: &str, style: &TextStyle) -> TextSize {
        let scale = style.font_size / crate::constants::REFERENCE_FONT_SIZE;
        // Convert LaTeX commands to Unicode BEFORE stripping markdown markers
        // so that label sizing matches what the SVG renderer actually draws.
        // Example: `\mathbb{E}[T]` measures as 3 chars (𝔼[T]), not 14.
        let latex_stripped = crate::latex::strip_latex_to_unicode(text);
        let stripped = strip_markup(&latex_stripped);
        let mut max_width: f64 = 0.0;
        let mut line_count: usize = 0;
        for line in stripped.split('\n') {
            line_count += 1;
            let w: f64 = line.chars().map(char_width_ratio).sum();
            max_width = max_width.max(w);
        }

        TextSize {
            width: max_width * self.avg_char_width * scale,
            height: line_count as f64 * self.line_height * scale,
        }
    }
}

/// Compute the baseline Y for vertically centered text.
///
/// Given a target center Y position, returns the Y coordinate for the
/// **first line's baseline** so that the text block is visually centered.
///
/// SVG does this with `dominant-baseline: central`. Non-SVG backends
/// (raster, gpui, vello) call this to compute the baseline position.
///
/// Usage: `baseline_y = center_y + text_baseline_y_offset(font_size, line_count)`
///
/// Based on Intel One Mono metrics: ascent ≈ 0.8em, descent ≈ 0.2em.
/// Visual center above baseline = (ascent - |descent|) / 2 ≈ 0.3em.
pub fn text_baseline_y_offset(font_size: f64, line_count: usize) -> f64 {
    // For a single line, baseline should be below center by 0.3 * font_size
    // because glyphs extend more above baseline (ascent) than below (descent).
    let baseline_from_center = font_size * crate::constants::BASELINE_ASCENT_RATIO;
    // For multi-line, shift up by half the total block height (from first to last baseline).
    let line_height = font_size * crate::constants::LINE_HEIGHT_MULTIPLIER;
    let block_offset = (line_count as f64 - 1.0) * line_height / 2.0;
    baseline_from_center - block_offset
}

/// A text span with inline markdown formatting.
#[derive(Debug, Clone, PartialEq)]
pub struct MdSpan {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
}

/// Parse inline markdown (`**bold**`, `*italic*`, `***both***`) into styled spans.
/// Returns `None` if the text contains no formatting markers.
pub fn parse_inline_markdown(text: &str) -> Option<Vec<MdSpan>> {
    if !text.contains('*') {
        return None;
    }

    let mut spans = Vec::new();
    let mut bold = false;
    let mut italic = false;
    let mut buf = String::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '*' && chars.peek() == Some(&'*') {
            chars.next();
            if !buf.is_empty() {
                spans.push(MdSpan {
                    text: std::mem::take(&mut buf),
                    bold,
                    italic,
                });
            }
            bold = !bold;
        } else if c == '*' {
            if !buf.is_empty() {
                spans.push(MdSpan {
                    text: std::mem::take(&mut buf),
                    bold,
                    italic,
                });
            }
            italic = !italic;
        } else {
            buf.push(c);
        }
    }
    if !buf.is_empty() {
        spans.push(MdSpan {
            text: buf,
            bold,
            italic,
        });
    }

    if spans.iter().any(|s| s.bold || s.italic) {
        Some(spans)
    } else {
        None
    }
}

/// Strip HTML tags from text in a single pass.
/// Replaces `<br/>` variants with newlines; strips other tags.
/// When `include_markdown` is true, also removes `**` and `*` markers.
fn strip_tags(text: &str, include_markdown: bool) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_tag = false;
    let mut tag_buf = String::with_capacity(8);
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_tag {
            if ch == '>' {
                in_tag = false;
                if tag_buf.eq_ignore_ascii_case("br")
                    || tag_buf.eq_ignore_ascii_case("br/")
                    || tag_buf.eq_ignore_ascii_case("br /")
                {
                    result.push('\n');
                }
            } else {
                tag_buf.push(ch);
            }
        } else if ch == '<' {
            in_tag = true;
            tag_buf.clear();
        } else if include_markdown && ch == '*' {
            if chars.peek() == Some(&'*') {
                chars.next();
            }
        } else {
            result.push(ch);
        }
    }
    result
}

/// Strip HTML tags and markdown markers (`*`, `**`).
fn strip_markup(text: &str) -> String {
    strip_tags(text, true)
}

/// Strip HTML tags only (no markdown removal).
#[cfg(test)]
fn strip_html_tags(text: &str) -> String {
    strip_tags(text, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    const W: f64 = MONOSPACE_CHAR_WIDTH_14PX;
    const LH: f64 = MONOSPACE_LINE_HEIGHT_14PX;

    fn default_style() -> TextStyle {
        TextStyle::default()
    }

    #[test]
    fn simple_measure_basic() {
        let m = SimpleTextMeasure::default();
        let s = m.measure("hello", &default_style());
        assert!((s.width - 5.0 * W).abs() < f64::EPSILON);
        assert!((s.height - LH).abs() < f64::EPSILON);
    }

    #[test]
    fn simple_measure_empty() {
        let m = SimpleTextMeasure::default();
        let s = m.measure("", &default_style());
        assert!((s.width - 0.0).abs() < f64::EPSILON);
        assert!((s.height - LH).abs() < f64::EPSILON);
    }

    #[test]
    fn simple_measure_strips_html() {
        let m = SimpleTextMeasure::default();
        let s = m.measure("<b>bold</b>", &default_style());
        assert!((s.width - 4.0 * W).abs() < f64::EPSILON);
    }

    #[test]
    fn simple_measure_br_adds_lines() {
        let m = SimpleTextMeasure::default();
        let s = m.measure("line1<br/>line2", &default_style());
        assert!((s.height - 2.0 * LH).abs() < f64::EPSILON);
    }

    #[test]
    fn simple_measure_font_size_scales() {
        let m = SimpleTextMeasure::default();
        let mut style = default_style();
        style.font_size = 28.0; // 2x default
        let s = m.measure("ab", &style);
        assert!((s.width - 2.0 * W * 2.0).abs() < f64::EPSILON);
        assert!((s.height - LH * 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn simple_measure_custom_char_width() {
        let m = SimpleTextMeasure::new(10.0, 20.0);
        let s = m.measure("abc", &default_style());
        assert!((s.width - 30.0).abs() < f64::EPSILON);
        assert!((s.height - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn measure_strips_markdown() {
        let m = SimpleTextMeasure::default();
        let w_plain = m.measure("bold", &default_style()).width;
        let w_md = m.measure("**bold**", &default_style()).width;
        assert!(
            (w_plain - w_md).abs() < f64::EPSILON,
            "markdown markers should be stripped: plain={w_plain} md={w_md}"
        );
    }

    #[test]
    fn strip_html_basic() {
        assert_eq!(strip_html_tags("<b>bold</b>"), "bold");
        assert_eq!(strip_html_tags("<i>italic</i>"), "italic");
        assert_eq!(strip_html_tags("no tags"), "no tags");
    }

    #[test]
    fn strip_html_br_to_newline() {
        assert_eq!(strip_html_tags("a<br/>b"), "a\nb");
        assert_eq!(strip_html_tags("a<br>b"), "a\nb");
        assert_eq!(strip_html_tags("a<br />b"), "a\nb");
    }

    #[test]
    fn strip_html_nested() {
        assert_eq!(strip_html_tags("<b><i>text</i></b>"), "text");
    }

    #[test]
    fn default_trait() {
        let m = SimpleTextMeasure::default();
        assert!((m.avg_char_width - W).abs() < f64::EPSILON);
        assert!((m.line_height - LH).abs() < f64::EPSILON);
    }

    #[test]
    fn cjk_chars_wider_than_latin() {
        let m = SimpleTextMeasure::default();
        let w = m.measure("你好世界", &default_style()).width;
        assert!((w - 4.0 * 1.8 * W).abs() < 1e-10);
    }

    #[test]
    fn japanese_kana_wider_than_latin() {
        let m = SimpleTextMeasure::default();
        let w = m.measure("こんにちは世界", &default_style()).width;
        assert!((w - 7.0 * 1.8 * W).abs() < 1e-10);
    }

    #[test]
    fn mixed_latin_cjk() {
        let m = SimpleTextMeasure::default();
        let w = m.measure("Hi你好", &default_style()).width;
        assert!((w - (2.0 + 2.0 * 1.8) * W).abs() < 1e-10);
    }

    #[test]
    fn latin_and_cyrillic_widths() {
        let m = SimpleTextMeasure::default();
        let w_latin = m.measure("hello", &default_style()).width;
        let w_cyrillic = m.measure("приве", &default_style()).width;
        assert!((w_latin - 5.0 * W).abs() < 1e-10);
        assert!((w_cyrillic - 5.0 * 0.85 * W).abs() < 1e-10);
    }

    #[test]
    fn char_width_ratios() {
        assert!((char_width_ratio('A') - 1.0).abs() < f64::EPSILON); // Primary
        assert!((char_width_ratio('你') - 1.8).abs() < f64::EPSILON); // CJK
        assert!((char_width_ratio('α') - 0.85).abs() < f64::EPSILON); // ExtendedText
        assert!((char_width_ratio('★') - 1.4).abs() < f64::EPSILON); // Dingbats
        assert!((char_width_ratio('→') - 1.0).abs() < f64::EPSILON); // Monospace
        assert!((char_width_ratio('م') - 0.8).abs() < f64::EPSILON); // Arabic
    }

    // ── Text measurement property tests (13.10) ──

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn char_width_ratio_always_positive(c in proptest::char::any()) {
            let r = char_width_ratio(c);
            prop_assert!(r > 0.0, "char_width_ratio({c:?}) = {r}, must be > 0");
        }

        #[test]
        fn measure_width_positive_for_nonempty(
            text in "[a-zA-Z0-9]{1,20}",
        ) {
            let m = SimpleTextMeasure::default();
            let s = m.measure(&text, &default_style());
            prop_assert!(s.width > 0.0, "width must be > 0 for non-empty text, got {}", s.width);
            prop_assert!(s.height > 0.0, "height must be > 0, got {}", s.height);
        }

        #[test]
        fn measure_scales_linearly_with_font_size(
            text in "[a-z]{1,10}",
            scale in 0.5..4.0f64,
        ) {
            let m = SimpleTextMeasure::default();
            let base_style = default_style();
            let mut scaled_style = base_style.clone();
            scaled_style.font_size = base_style.font_size * scale;

            let s1 = m.measure(&text, &base_style);
            let s2 = m.measure(&text, &scaled_style);

            prop_assert!((s2.width / s1.width - scale).abs() < 1e-10,
                "width should scale by {scale}: w1={}, w2={}", s1.width, s2.width);
            prop_assert!((s2.height / s1.height - scale).abs() < 1e-10,
                "height should scale by {scale}: h1={}, h2={}", s1.height, s2.height);
        }
    }
}
