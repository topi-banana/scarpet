//! A small Wadler/Lindig-style pretty-printing IR.
//!
//! A [`Doc`] describes a document; [`Doc::render`] lays it out at a target
//! width, choosing for each `Group` whether to print it flat (on one line) or
//! broken (with its `Line`s expanded to newlines). `HardLine` and `BlankLine`
//! always break and force any enclosing group to break with them — this is how
//! a `//` comment (which runs to end-of-line) forces its group open.

#![allow(dead_code)] // Builders are consumed by `lower` in later phases.

use std::borrow::Cow;

#[derive(Debug, Clone)]
pub enum Doc {
    /// Empty document.
    Nil,
    /// Literal text with no embedded newline.
    Text(Cow<'static, str>),
    /// A line comment, rendered verbatim unless `comment_width` asks for
    /// wrapping.
    Comment(Cow<'static, str>),
    /// A space when flat, a newline + indent when broken.
    Line,
    /// Nothing when flat, a newline + indent when broken.
    SoftLine,
    /// Always a newline + indent. Forces the enclosing group to break.
    HardLine,
    /// A blank line (two newlines) + indent. Forces the enclosing group to break.
    BlankLine,
    /// `broken` when the enclosing group breaks, `flat` otherwise.
    IfBreak(Box<Doc>, Box<Doc>),
    /// A sequence of documents.
    Concat(Vec<Doc>),
    /// Increase the indent of the contained document by `n` levels; each level
    /// is `indent_width` spaces, applied at render time.
    Nest(isize, Box<Doc>),
    /// A group: rendered flat if it fits the remaining width, else broken.
    Group(Box<Doc>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Flat,
    Break,
}

// ---- builders ------------------------------------------------------

pub fn nil() -> Doc {
    Doc::Nil
}

pub fn text(s: impl Into<Cow<'static, str>>) -> Doc {
    Doc::Text(s.into())
}

pub fn comment(s: impl Into<Cow<'static, str>>) -> Doc {
    Doc::Comment(s.into())
}

pub fn line() -> Doc {
    Doc::Line
}

pub fn softline() -> Doc {
    Doc::SoftLine
}

pub fn hardline() -> Doc {
    Doc::HardLine
}

pub fn blank_line() -> Doc {
    Doc::BlankLine
}

pub fn if_break(broken: Doc, flat: Doc) -> Doc {
    Doc::IfBreak(Box::new(broken), Box::new(flat))
}

pub fn space() -> Doc {
    Doc::Text(Cow::Borrowed(" "))
}

pub fn group(d: Doc) -> Doc {
    Doc::Group(Box::new(d))
}

/// Indent the contained document by one level. A level's width in spaces is
/// supplied at render time, so this builder is style-agnostic.
pub fn nest(d: Doc) -> Doc {
    Doc::Nest(1, Box::new(d))
}

/// Concatenate documents, dropping `Nil`s and collapsing the trivial cases.
pub fn concat(parts: impl IntoIterator<Item = Doc>) -> Doc {
    let mut v: Vec<Doc> = parts
        .into_iter()
        .filter(|d| !matches!(d, Doc::Nil))
        .collect();
    match v.len() {
        0 => Doc::Nil,
        1 => v.pop().unwrap(),
        _ => Doc::Concat(v),
    }
}

/// Concatenate documents with `sep` between consecutive items.
pub fn join(items: impl IntoIterator<Item = Doc>, sep: Doc) -> Doc {
    let mut parts = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        if i > 0 {
            parts.push(sep.clone());
        }
        parts.push(item);
    }
    concat(parts)
}

impl Doc {
    /// Render at the given target width, indenting each level by `indent_width`
    /// spaces. Breaks the formatter inserts are emitted as `line_ending` (e.g.
    /// `"\n"` or `"\r\n"`); bytes inside `Text` nodes are left untouched. Each
    /// line is right-trimmed; the result carries no enforced trailing newline
    /// (the caller appends one).
    pub fn render(
        &self,
        width: usize,
        comment_width: Option<usize>,
        indent_width: usize,
        line_ending: &str,
    ) -> String {
        let step = indent_width as isize;
        let mut out = String::new();
        let mut col: isize = 0;
        // Work stack of (indent, mode, doc), processed LIFO.
        let mut stack: Vec<(isize, Mode, &Doc)> = vec![(0, Mode::Break, self)];
        while let Some((indent, mode, doc)) = stack.pop() {
            match doc {
                Doc::Nil => {}
                Doc::Text(s) => {
                    out.push_str(s);
                    col += s.chars().count() as isize;
                }
                Doc::Comment(s) => {
                    col = push_comment(&mut out, s, col, indent, comment_width, line_ending);
                }
                Doc::Line => match mode {
                    Mode::Flat => {
                        out.push(' ');
                        col += 1;
                    }
                    Mode::Break => {
                        newline(&mut out, indent, false, line_ending);
                        col = indent;
                    }
                },
                Doc::SoftLine => match mode {
                    Mode::Flat => {}
                    Mode::Break => {
                        newline(&mut out, indent, false, line_ending);
                        col = indent;
                    }
                },
                Doc::HardLine => {
                    newline(&mut out, indent, false, line_ending);
                    col = indent;
                }
                Doc::BlankLine => {
                    newline(&mut out, indent, true, line_ending);
                    col = indent;
                }
                Doc::IfBreak(broken, flat) => {
                    let chosen = if mode == Mode::Break { broken } else { flat };
                    stack.push((indent, mode, chosen));
                }
                Doc::Concat(parts) => {
                    for p in parts.iter().rev() {
                        stack.push((indent, mode, p));
                    }
                }
                Doc::Nest(n, d) => stack.push((indent + n * step, mode, d)),
                Doc::Group(d) => {
                    let m = if fits(width as isize - col, d) {
                        Mode::Flat
                    } else {
                        Mode::Break
                    };
                    stack.push((indent, m, d));
                }
            }
        }
        out
    }
}

/// Append the break `nl` (or a blank line: two of them) followed by `indent`
/// spaces, trimming any trailing spaces from the line just finished.
fn newline(out: &mut String, indent: isize, blank: bool, nl: &str) {
    while out.ends_with(' ') {
        out.pop();
    }
    out.push_str(nl);
    if blank {
        out.push_str(nl);
    }
    for _ in 0..indent {
        out.push(' ');
    }
}

/// Whether `doc` rendered flat fits in `remaining` columns. A group is laid out
/// flat iff its *own* flat rendering fits — what follows the group is
/// deliberately not considered, which keeps the decision local and cheap. A
/// `HardLine`/`BlankLine` inside means the group cannot be flat at all.
fn fits(mut remaining: isize, doc: &Doc) -> bool {
    if remaining < 0 {
        return false;
    }
    let mut work: Vec<&Doc> = vec![doc];
    while let Some(d) = work.pop() {
        match d {
            Doc::Nil => {}
            Doc::Text(s) => {
                remaining -= s.chars().count() as isize;
                if remaining < 0 {
                    return false;
                }
            }
            Doc::Comment(s) => {
                remaining -= s.chars().count() as isize;
                if remaining < 0 {
                    return false;
                }
            }
            // Flat: `Line` is a space, `SoftLine` is nothing.
            Doc::Line => {
                remaining -= 1;
                if remaining < 0 {
                    return false;
                }
            }
            Doc::SoftLine => {}
            Doc::HardLine | Doc::BlankLine => return false,
            Doc::IfBreak(_, flat) => work.push(flat),
            Doc::Concat(parts) => {
                for p in parts.iter().rev() {
                    work.push(p);
                }
            }
            // Indentation never affects fit: a level's width is added only when
            // broken, so the break/flat choice is independent of `indent_width`
            // — which is what keeps formatting idempotent per config.
            Doc::Nest(_, d) => work.push(d),
            Doc::Group(d) => work.push(d),
        }
    }
    true
}

fn push_comment(
    out: &mut String,
    raw: &str,
    col: isize,
    indent: isize,
    comment_width: Option<usize>,
    nl: &str,
) -> isize {
    let Some(width) = comment_width else {
        out.push_str(raw);
        return col + raw.chars().count() as isize;
    };
    let Some(rest) = raw.strip_prefix("//") else {
        out.push_str(raw);
        return col + raw.chars().count() as isize;
    };
    let (prefix, body) = if let Some(body) = rest.strip_prefix(' ') {
        ("// ", body)
    } else {
        ("//", rest)
    };
    let lines = wrap_comment_body(prefix, body, col as usize, indent.max(0) as usize, width);
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            newline(out, indent, false, nl);
        }
        out.push_str(line);
    }
    let last_len = lines.last().map_or(0, |line| line.chars().count()) as isize;
    if lines.len() > 1 {
        indent + last_len
    } else {
        col + last_len
    }
}

fn wrap_comment_body(
    prefix: &str,
    body: &str,
    start_col: usize,
    continuation_col: usize,
    width: usize,
) -> Vec<String> {
    let prefix_len = prefix.chars().count();
    if width == 0 || start_col + prefix_len >= width {
        return vec![format!("{prefix}{body}")];
    }

    let mut lines = Vec::new();
    let mut current = String::from(prefix);
    let mut current_len = start_col + prefix_len;
    let continuation_len = continuation_col + prefix_len;
    let mut first_word = true;

    for word in body.split_whitespace() {
        let word_len = word.chars().count();
        let sep_len = usize::from(!first_word);
        if !first_word && current_len + sep_len + word_len > width {
            lines.push(current);
            current = String::from(prefix);
            current_len = continuation_len;
            first_word = true;
        }
        if !first_word {
            current.push(' ');
            current_len += 1;
        }
        current.push_str(word);
        current_len += word_len;
        first_word = false;
    }

    if first_word {
        current.push_str(body);
    }
    lines.push(current);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All `Doc` tests render at `indent_width = 4` to match the default style;
    /// the expected strings bake in 4-space indentation.
    const W: usize = 4;

    #[test]
    fn plain_text() {
        assert_eq!(text("hello").render(80, None, W, "\n"), "hello");
    }

    #[test]
    fn group_stays_flat_when_it_fits() {
        let d = group(concat([text("a"), line(), text("b")]));
        assert_eq!(d.render(80, None, W, "\n"), "a b");
    }

    #[test]
    fn group_breaks_when_too_wide() {
        let d = group(concat([text("aaa"), line(), text("bbb")]));
        assert_eq!(d.render(4, None, W, "\n"), "aaa\nbbb");
    }

    #[test]
    fn nest_indents_broken_lines() {
        let d = group(concat([
            text("("),
            nest(concat([softline(), text("x")])),
            softline(),
            text(")"),
        ]));
        assert_eq!(d.clone().render(80, None, W, "\n"), "(x)");
        assert_eq!(d.render(2, None, W, "\n"), "(\n    x\n)");
    }

    #[test]
    fn hardline_forces_break_even_when_it_fits() {
        let d = group(concat([text("a"), hardline(), text("b")]));
        assert_eq!(d.render(80, None, W, "\n"), "a\nb");
    }

    #[test]
    fn blank_line_emits_two_newlines() {
        let d = concat([text("a"), blank_line(), text("b")]);
        assert_eq!(d.render(80, None, W, "\n"), "a\n\nb");
    }

    #[test]
    fn if_break_follows_group_mode() {
        let flat = group(concat([
            text("x"),
            if_break(text(","), nil()),
            line(),
            text("y"),
        ]));
        assert_eq!(flat.render(80, None, W, "\n"), "x y");
        let broken = group(concat([
            text("xxxx"),
            if_break(text(","), nil()),
            line(),
            text("yyyy"),
        ]));
        assert_eq!(broken.render(4, None, W, "\n"), "xxxx,\nyyyy");
    }

    #[test]
    fn trailing_spaces_trimmed_before_newline() {
        let d = concat([text("a"), space(), hardline(), text("b")]);
        assert_eq!(d.render(80, None, W, "\n"), "a\nb");
    }

    #[test]
    fn join_inserts_separators() {
        let d = join([text("a"), text("b"), text("c")], text(", "));
        assert_eq!(d.render(80, None, W, "\n"), "a, b, c");
    }

    #[test]
    fn render_uses_supplied_line_ending() {
        let d = group(concat([text("aaa"), line(), text("bbb")]));
        assert_eq!(d.render(4, None, W, "\r\n"), "aaa\r\nbbb");
    }

    #[test]
    fn crlf_blank_line_emits_two_crlf() {
        let d = concat([text("a"), blank_line(), text("b")]);
        assert_eq!(d.render(80, None, W, "\r\n"), "a\r\n\r\nb");
    }

    #[test]
    fn comment_width_wraps_line_comments() {
        let d = comment("// one two three four");
        assert_eq!(
            d.render(80, Some(12), W, "\n"),
            "// one two\n// three\n// four"
        );
    }

    #[test]
    fn disabled_comment_width_keeps_comment_verbatim() {
        let d = comment("// one two three four");
        assert_eq!(d.render(80, None, W, "\n"), "// one two three four");
    }

    #[test]
    fn wrapped_comment_continuation_uses_current_indent() {
        let d = concat([
            text("("),
            nest(concat([hardline(), comment("// one two three")])),
        ]);
        assert_eq!(
            d.render(80, Some(13), W, "\n"),
            "(\n    // one\n    // two\n    // three"
        );
    }
}
