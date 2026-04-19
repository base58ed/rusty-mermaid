//! LaTeX-to-Unicode simplification for diagram label text.
//!
//! LLM-generated mermaid sources sometimes embed LaTeX commands inside node
//! and edge labels — e.g. `\mathbb{E}[T]`, `t_{transit}`, `\text{状态}`,
//! `3\sigma`. Mermaid.js itself does not render LaTeX inside labels
//! natively (an experimental KaTeX plugin exists upstream but is not a
//! default), and rusty-mermaid's SVG backend otherwise emits the raw
//! backslash-escaped source into `<text>` elements, which is unreadable.
//!
//! This module provides a best-effort pre-processor that walks a label
//! string and substitutes common LaTeX commands with Unicode equivalents
//! or plain-text simplifications. It is **not** a KaTeX replacement —
//! it does not produce proper 2D layout for fractions, matrices, or
//! integrals. Its job is strictly to make the most common LLM patterns
//! legible. If you need real math rendering, wire in a KaTeX-equivalent
//! at the renderer layer instead.
//!
//! Coverage (observed from Kimi / DeepSeek / Claude outputs 2026-04):
//!   - `\text{X}`     → bare `X`
//!   - `\mathbb{X}`   → Unicode blackboard bold (ℝ, ℕ, ℤ, ℚ, ℂ, 𝔼, 𝔽, …)
//!   - `\mathbf{X}`   → `X` (bold would need separate markup; drop wrapper)
//!   - `\mathit{X}`   → `X` (same)
//!   - `\frac{a}{b}`  → `a/b`
//!   - `\sqrt{x}`     → `√x`
//!   - Greek letters (`\alpha`..`\omega`, `\Alpha`..`\Omega`)
//!   - Common math operators: `\sum` `\int` `\partial` `\nabla` `\infty`
//!     `\in` `\notin` `\forall` `\exists` `\leq` `\geq` `\neq` `\approx`
//!     `\cdot` `\times` `\pm` `\to` `\rightarrow` `\leftarrow` `\Rightarrow`
//!   - Subscripts `_x` / `_{xxx}` → Unicode subscript where each char has
//!     a Unicode subscript form, else `_xxx`
//!   - Superscripts `^x` / `^{xxx}` → same logic
//!   - Inline `$…$` and display `$$…$$` delimiters stripped
//!
//! Unknown `\cmd{…}` wrappers drop the `\cmd` prefix and keep `{…}` as
//! plain braces — a safer default than leaving raw backslashes.

use std::borrow::Cow;

/// Convert common LaTeX-in-label constructs to Unicode / plain-text.
/// Idempotent on inputs with no LaTeX markers.
#[must_use]
pub fn strip_latex_to_unicode(input: &str) -> Cow<'_, str> {
    // Fast path: no LaTeX-signal characters at all → borrow.
    let has_signal = input.as_bytes().iter().any(|b| {
        matches!(b, b'\\' | b'$' | b'_' | b'^')
    });
    if !has_signal {
        return Cow::Borrowed(input);
    }
    Cow::Owned(strip_pass(input))
}

/// Single-pass char walker. Not optimal but clear.
fn strip_pass(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\\' => handle_backslash(&mut chars, &mut out),
            '$' => {
                // Strip `$$…$$` and `$…$` delimiters. Consume a second `$` if present.
                if chars.peek() == Some(&'$') {
                    chars.next();
                }
                // Don't push the `$` — it's a delimiter, and everything inside is already
                // processed by this same loop because we don't switch modes.
            }
            '_' => handle_sub_sup(&mut chars, &mut out, '_'),
            '^' => handle_sub_sup(&mut chars, &mut out, '^'),
            c => out.push(c),
        }
    }

    out
}

/// Called after we consumed a `\`. Look at the following chars to decide
/// what to emit.
fn handle_backslash(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, out: &mut String) {
    // Read the command name: [a-zA-Z]+
    let mut name = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphabetic() {
            name.push(c);
            chars.next();
        } else {
            break;
        }
    }

    if name.is_empty() {
        // Escaped non-alphabetic char — emit as-is.
        if let Some(c) = chars.next() {
            out.push(c);
        }
        return;
    }

    // Is there a `{...}` argument?
    let arg = if chars.peek() == Some(&'{') {
        chars.next(); // consume '{'
        Some(read_balanced_braces(chars))
    } else {
        None
    };

    match (name.as_str(), arg) {
        // Wrappers: drop the command, keep the inner content processed.
        ("text", Some(inner)) | ("mathbf", Some(inner)) | ("mathit", Some(inner))
        | ("mathrm", Some(inner)) | ("mathsf", Some(inner)) | ("mathtt", Some(inner)) => {
            out.push_str(&strip_pass(&inner));
        }
        // Blackboard bold.
        ("mathbb", Some(inner)) => {
            out.push_str(&mathbb_transform(&inner));
        }
        // Calligraphic → leave as-is (Unicode coverage is partial; fallback to inner).
        ("mathcal", Some(inner)) => {
            out.push_str(&strip_pass(&inner));
        }
        // Fraction: a/b.
        ("frac", Some(inner)) => {
            if chars.peek() == Some(&'{') {
                chars.next();
                let denom = read_balanced_braces(chars);
                out.push_str(&strip_pass(&inner));
                out.push('/');
                out.push_str(&strip_pass(&denom));
            } else {
                out.push_str(&strip_pass(&inner));
            }
        }
        // Square root.
        ("sqrt", Some(inner)) => {
            out.push('√');
            out.push_str(&strip_pass(&inner));
        }
        // Greek + operators without arg.
        (name, None) => {
            if let Some(sym) = lookup_symbol(name) {
                out.push_str(sym);
            } else {
                // Unknown command; drop the backslash prefix, keep the name.
                out.push_str(name);
            }
        }
        // Unknown command WITH arg: drop the backslash and the wrapping, keep inner.
        (_, Some(inner)) => {
            out.push_str(&strip_pass(&inner));
        }
    }
}

/// Called after we consumed `_` or `^`. Look at next token — could be a
/// single char or `{…}` group.
fn handle_sub_sup(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    out: &mut String,
    marker: char,
) {
    let group = if chars.peek() == Some(&'{') {
        chars.next();
        read_balanced_braces(chars)
    } else if let Some(&c) = chars.peek() {
        chars.next();
        c.to_string()
    } else {
        return;
    };

    let processed = strip_pass(&group);
    let converted = if marker == '_' {
        convert_subscript(&processed)
    } else {
        convert_superscript(&processed)
    };

    if let Some(s) = converted {
        out.push_str(&s);
    } else {
        out.push(marker);
        out.push_str(&processed);
    }
}

/// Read chars until the matching `}` (handling nesting). Consumes the
/// closing `}`. Returns the inner body without braces.
fn read_balanced_braces(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut depth = 1usize;
    let mut body = String::new();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                depth += 1;
                body.push(c);
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return body;
                }
                body.push(c);
            }
            _ => body.push(c),
        }
    }
    body
}

/// Map `\mathbb{X}` to Unicode blackboard bold where a codepoint exists.
/// Falls back to plain X for letters without a Unicode BB form.
fn mathbb_transform(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 4);
    for c in s.chars() {
        let mapped = match c {
            'A' => "𝔸", 'B' => "𝔹", 'C' => "ℂ", 'D' => "𝔻", 'E' => "𝔼",
            'F' => "𝔽", 'G' => "𝔾", 'H' => "ℍ", 'I' => "𝕀", 'J' => "𝕁",
            'K' => "𝕂", 'L' => "𝕃", 'M' => "𝕄", 'N' => "ℕ", 'O' => "𝕆",
            'P' => "ℙ", 'Q' => "ℚ", 'R' => "ℝ", 'S' => "𝕊", 'T' => "𝕋",
            'U' => "𝕌", 'V' => "𝕍", 'W' => "𝕎", 'X' => "𝕏", 'Y' => "𝕐",
            'Z' => "ℤ",
            'a' => "𝕒", 'b' => "𝕓", 'c' => "𝕔", 'd' => "𝕕", 'e' => "𝕖",
            'f' => "𝕗", 'g' => "𝕘", 'h' => "𝕙", 'i' => "𝕚", 'j' => "𝕛",
            'k' => "𝕜", 'l' => "𝕝", 'm' => "𝕞", 'n' => "𝕟", 'o' => "𝕠",
            'p' => "𝕡", 'q' => "𝕢", 'r' => "𝕣", 's' => "𝕤", 't' => "𝕥",
            'u' => "𝕦", 'v' => "𝕧", 'w' => "𝕨", 'x' => "𝕩", 'y' => "𝕪",
            'z' => "𝕫",
            '0' => "𝟘", '1' => "𝟙", '2' => "𝟚", '3' => "𝟛", '4' => "𝟜",
            '5' => "𝟝", '6' => "𝟞", '7' => "𝟟", '8' => "𝟠", '9' => "𝟡",
            _ => {
                out.push(c);
                continue;
            }
        };
        out.push_str(mapped);
    }
    out
}

/// Lookup table for LaTeX commands that have a direct Unicode equivalent.
/// Order: lowercase Greek, uppercase Greek, operators, arrows.
fn lookup_symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        // Lowercase Greek
        "alpha" => "α", "beta" => "β", "gamma" => "γ", "delta" => "δ",
        "epsilon" => "ε", "varepsilon" => "ε", "zeta" => "ζ", "eta" => "η",
        "theta" => "θ", "vartheta" => "ϑ", "iota" => "ι", "kappa" => "κ",
        "lambda" => "λ", "mu" => "μ", "nu" => "ν", "xi" => "ξ",
        "pi" => "π", "varpi" => "ϖ", "rho" => "ρ", "varrho" => "ϱ",
        "sigma" => "σ", "varsigma" => "ς", "tau" => "τ", "upsilon" => "υ",
        "phi" => "φ", "varphi" => "ϕ", "chi" => "χ", "psi" => "ψ",
        "omega" => "ω",
        // Uppercase Greek
        "Alpha" => "Α", "Beta" => "Β", "Gamma" => "Γ", "Delta" => "Δ",
        "Epsilon" => "Ε", "Zeta" => "Ζ", "Eta" => "Η", "Theta" => "Θ",
        "Iota" => "Ι", "Kappa" => "Κ", "Lambda" => "Λ", "Mu" => "Μ",
        "Nu" => "Ν", "Xi" => "Ξ", "Pi" => "Π", "Rho" => "Ρ",
        "Sigma" => "Σ", "Tau" => "Τ", "Upsilon" => "Υ", "Phi" => "Φ",
        "Chi" => "Χ", "Psi" => "Ψ", "Omega" => "Ω",
        // Math operators
        "sum" => "Σ", "prod" => "∏", "int" => "∫", "oint" => "∮",
        "partial" => "∂", "nabla" => "∇", "infty" => "∞", "aleph" => "ℵ",
        "emptyset" => "∅", "in" => "∈", "notin" => "∉", "ni" => "∋",
        "subset" => "⊂", "supset" => "⊃", "subseteq" => "⊆", "supseteq" => "⊇",
        "cup" => "∪", "cap" => "∩", "setminus" => "∖",
        "forall" => "∀", "exists" => "∃", "nexists" => "∄",
        "leq" => "≤", "le" => "≤", "geq" => "≥", "ge" => "≥",
        "neq" => "≠", "ne" => "≠", "approx" => "≈", "equiv" => "≡",
        "sim" => "∼", "simeq" => "≃", "cong" => "≅", "propto" => "∝",
        "cdot" => "·", "cdots" => "⋯", "ldots" => "…", "dots" => "…",
        "times" => "×", "div" => "÷", "pm" => "±", "mp" => "∓",
        "star" => "⋆", "ast" => "∗", "circ" => "∘", "bullet" => "•",
        "wedge" => "∧", "vee" => "∨", "land" => "∧", "lor" => "∨",
        "lnot" => "¬", "neg" => "¬",
        // Arrows
        "to" => "→", "rightarrow" => "→", "leftarrow" => "←", "gets" => "←",
        "Rightarrow" => "⇒", "Leftarrow" => "⇐", "Leftrightarrow" => "⇔",
        "leftrightarrow" => "↔", "mapsto" => "↦", "uparrow" => "↑",
        "downarrow" => "↓", "Uparrow" => "⇑", "Downarrow" => "⇓",
        // Misc
        "angle" => "∠", "perp" => "⊥", "parallel" => "∥", "prime" => "′",
        "hbar" => "ℏ", "ell" => "ℓ", "Re" => "ℜ", "Im" => "ℑ",
        // Spacing / nops
        "," | ";" | ":" | "!" | " " | "quad" | "qquad" => "",
        "langle" => "⟨", "rangle" => "⟩", "lceil" => "⌈", "rceil" => "⌉",
        "lfloor" => "⌊", "rfloor" => "⌋",
        _ => return None,
    })
}

/// Convert a subscript string to Unicode subscripts if every char has a
/// subscript form. Returns None if any char can't be mapped.
fn convert_subscript(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len() * 3);
    for c in s.chars() {
        let mapped = match c {
            '0' => '₀', '1' => '₁', '2' => '₂', '3' => '₃', '4' => '₄',
            '5' => '₅', '6' => '₆', '7' => '₇', '8' => '₈', '9' => '₉',
            '+' => '₊', '-' => '₋', '=' => '₌', '(' => '₍', ')' => '₎',
            'a' => 'ₐ', 'e' => 'ₑ', 'h' => 'ₕ', 'i' => 'ᵢ', 'j' => 'ⱼ',
            'k' => 'ₖ', 'l' => 'ₗ', 'm' => 'ₘ', 'n' => 'ₙ', 'o' => 'ₒ',
            'p' => 'ₚ', 'r' => 'ᵣ', 's' => 'ₛ', 't' => 'ₜ', 'u' => 'ᵤ',
            'v' => 'ᵥ', 'x' => 'ₓ',
            _ => return None,
        };
        out.push(mapped);
    }
    Some(out)
}

/// Convert a superscript string to Unicode superscripts if every char has
/// a superscript form. Returns None if any char can't be mapped.
fn convert_superscript(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len() * 3);
    for c in s.chars() {
        let mapped = match c {
            '0' => '⁰', '1' => '¹', '2' => '²', '3' => '³', '4' => '⁴',
            '5' => '⁵', '6' => '⁶', '7' => '⁷', '8' => '⁸', '9' => '⁹',
            '+' => '⁺', '-' => '⁻', '=' => '⁼', '(' => '⁽', ')' => '⁾',
            'a' => 'ᵃ', 'b' => 'ᵇ', 'c' => 'ᶜ', 'd' => 'ᵈ', 'e' => 'ᵉ',
            'f' => 'ᶠ', 'g' => 'ᵍ', 'h' => 'ʰ', 'i' => 'ⁱ', 'j' => 'ʲ',
            'k' => 'ᵏ', 'l' => 'ˡ', 'm' => 'ᵐ', 'n' => 'ⁿ', 'o' => 'ᵒ',
            'p' => 'ᵖ', 'r' => 'ʳ', 's' => 'ˢ', 't' => 'ᵗ', 'u' => 'ᵘ',
            'v' => 'ᵛ', 'w' => 'ʷ', 'x' => 'ˣ', 'y' => 'ʸ', 'z' => 'ᶻ',
            _ => return None,
        };
        out.push(mapped);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_borrowed() {
        let out = strip_latex_to_unicode("hello world 你好");
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, "hello world 你好");
    }

    #[test]
    fn text_wrapper_is_unwrapped() {
        assert_eq!(strip_latex_to_unicode("\\text{状态}"), "状态");
        assert_eq!(strip_latex_to_unicode("\\text{signed}"), "signed");
    }

    #[test]
    fn mathbb_converts() {
        assert_eq!(strip_latex_to_unicode("\\mathbb{E}"), "𝔼");
        assert_eq!(strip_latex_to_unicode("\\mathbb{R}"), "ℝ");
        assert_eq!(strip_latex_to_unicode("\\mathbb{E}[T]"), "𝔼[T]");
    }

    #[test]
    fn subscript_with_braces() {
        assert_eq!(strip_latex_to_unicode("t_{transit}"), "tₜᵣₐₙₛᵢₜ");
        assert_eq!(strip_latex_to_unicode("x_1"), "x₁");
        assert_eq!(strip_latex_to_unicode("a_{i+1}"), "aᵢ₊₁");
    }

    #[test]
    fn subscript_falls_back_for_unmappable() {
        // 'q' has no Unicode subscript → fallback to `_xxx`.
        let out = strip_latex_to_unicode("x_{q}");
        assert_eq!(out, "x_q");
    }

    #[test]
    fn superscript_with_braces() {
        assert_eq!(strip_latex_to_unicode("x^2"), "x²");
        // `\pi` is converted to `π` inside the brace body. `π` has no Unicode
        // superscript form, so `convert_superscript` returns None for "iπ"
        // (any non-mappable char poisons the whole run) and we fall back to
        // the literal `^iπ`. Not beautiful, but honest — the alternative
        // (superscripting the letters `p`,`i` instead of the Greek π) would
        // hide the mathematical meaning.
        assert_eq!(strip_latex_to_unicode("e^{i\\pi}"), "e^iπ");
    }

    #[test]
    fn superscript_all_digits() {
        assert_eq!(strip_latex_to_unicode("10^{23}"), "10²³");
        assert_eq!(strip_latex_to_unicode("c^{2}"), "c²");
    }

    #[test]
    fn greek_letters() {
        assert_eq!(strip_latex_to_unicode("3\\sigma"), "3σ");
        assert_eq!(strip_latex_to_unicode("\\alpha + \\beta"), "α + β");
        assert_eq!(strip_latex_to_unicode("\\Sigma \\Delta"), "Σ Δ");
    }

    #[test]
    fn operators() {
        assert_eq!(strip_latex_to_unicode("x \\leq y"), "x ≤ y");
        assert_eq!(strip_latex_to_unicode("a \\neq b"), "a ≠ b");
        assert_eq!(strip_latex_to_unicode("x \\in \\mathbb{R}"), "x ∈ ℝ");
        assert_eq!(strip_latex_to_unicode("\\forall x"), "∀ x");
    }

    #[test]
    fn fraction_to_slash() {
        assert_eq!(strip_latex_to_unicode("\\frac{a}{b}"), "a/b");
        assert_eq!(strip_latex_to_unicode("\\frac{x+1}{2}"), "x+1/2");
    }

    #[test]
    fn sqrt() {
        assert_eq!(strip_latex_to_unicode("\\sqrt{2}"), "√2");
        assert_eq!(strip_latex_to_unicode("\\sqrt{x+y}"), "√x+y");
    }

    #[test]
    fn dollar_delimiters_stripped() {
        assert_eq!(strip_latex_to_unicode("$x^2$"), "x²");
        assert_eq!(strip_latex_to_unicode("$$E = mc^2$$"), "E = mc²");
    }

    #[test]
    fn full_observed_edge_labels() {
        // Reproduces the two edge labels from the 2026-04-19 screenshot.
        let in1 = "物流异常 t_{transit} >\\mathbb{E}[T] + 3\\sigma";
        let out1 = strip_latex_to_unicode(in1);
        assert_eq!(out1, "物流异常 tₜᵣₐₙₛᵢₜ >𝔼[T] + 3σ");

        let in2 = "签收确认 \\text{状态} = \\text{signed}";
        let out2 = strip_latex_to_unicode(in2);
        assert_eq!(out2, "签收确认 状态 = signed");
    }

    #[test]
    fn idempotent_on_output() {
        let samples = [
            "plain text",
            "\\mathbb{R}",
            "\\alpha \\leq \\beta",
            "t_{0} + \\Delta t",
            "\\text{abc}",
            "$x + y$",
        ];
        for s in &samples {
            let once = strip_latex_to_unicode(s);
            let twice = strip_latex_to_unicode(&once);
            assert_eq!(once, twice, "not idempotent for {s:?}");
        }
    }

    #[test]
    fn unknown_command_drops_backslash() {
        // Unknown command without arg: keep the name, drop backslash.
        assert_eq!(strip_latex_to_unicode("\\unknowncmd"), "unknowncmd");
    }

    #[test]
    fn unknown_command_with_braces_keeps_inner() {
        // Unknown command with arg: drop cmd and braces, keep inner.
        assert_eq!(strip_latex_to_unicode("\\bogus{inner}"), "inner");
    }
}
