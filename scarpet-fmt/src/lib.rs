//! Code formatter for the Scarpet language.
//!
//! Parses source into the `scarpet-syntax` CST and pretty-prints it at a fixed
//! style. Comments and blank-line separators are preserved; horizontal
//! whitespace is normalized by the formatter.

mod config;
mod doc;
mod lower;
mod trivia;

pub use config::{BinopSeparator, BraceStyle, Config, LineEnding, TrailingComma};
use scarpet_syntax::parser::{Cst, ParseError, parse_source};

/// Format Scarpet source text. Parses, then renders per `config`.
///
/// Returns [`FmtError::Parse`] if the source does not parse.
pub fn format_source(src: &str, config: &Config) -> Result<String, FmtError> {
    let cst = parse_source(src).map_err(FmtError::Parse)?;
    // `Auto` picks its line ending from the source, so resolve it now while
    // `src` is in hand: `format_cst` sees only the CST, which carries no
    // line-ending information of its own.
    let config = Config {
        line_ending: config.line_ending.resolve(src),
        ..*config
    };
    Ok(format_cst(&cst, &config))
}

/// Format an already-parsed CST. Infallible: a well-formed CST always renders.
pub fn format_cst(cst: &Cst<'_>, config: &Config) -> String {
    render_top(lower::program(cst, config), config)
}

/// Render a top-level document, guaranteeing the output ends in exactly one
/// newline (with no trailing blank lines or spaces).
fn render_top(doc: doc::Doc, config: &Config) -> String {
    let mut s = doc.render(
        config.max_width,
        config.comment_width,
        config.indent_width,
        config.line_ending.as_str(),
    );
    s.truncate(s.trim_end().len());
    s.push_str(config.line_ending.as_str());
    s
}

/// An error produced while formatting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FmtError {
    /// The source failed to parse. Boxed because [`ParseError`] is large and
    /// this is the cold path (keeps `Result`'s `Ok` arm cheap).
    Parse(Box<ParseError>),
}

impl std::fmt::Display for FmtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FmtError::Parse(e) => {
                write!(f, "parse error at byte {}: {}", e.span.start, e.message())
            }
        }
    }
}

impl std::error::Error for FmtError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atoms_round_trip() {
        let cfg = Config::default();
        assert_eq!(format_source("42", &cfg).unwrap(), "42\n");
        assert_eq!(format_source("0xff", &cfg).unwrap(), "0xff\n");
        assert_eq!(format_source("'hi'", &cfg).unwrap(), "'hi'\n");
        assert_eq!(format_source("foo", &cfg).unwrap(), "foo\n");
    }

    #[test]
    fn parse_error_surfaces() {
        let r = format_source("(", &Config::default());
        assert!(matches!(r, Err(FmtError::Parse(_))));
    }

    #[test]
    fn crlf_line_ending_applies_to_inserted_breaks() {
        let cfg = Config {
            line_ending: LineEnding::Crlf,
            ..Config::default()
        };
        assert_eq!(format_source("// c\nx", &cfg).unwrap(), "// c\r\nx\r\n");
    }

    #[test]
    fn auto_line_ending_follows_lf_source() {
        let cfg = Config {
            line_ending: LineEnding::Auto,
            ..Config::default()
        };
        assert_eq!(format_source("// c\nx", &cfg).unwrap(), "// c\nx\n");
    }

    #[test]
    fn auto_line_ending_follows_crlf_source() {
        let cfg = Config {
            line_ending: LineEnding::Auto,
            ..Config::default()
        };
        assert_eq!(format_source("// c\r\nx", &cfg).unwrap(), "// c\r\nx\r\n");
    }

    #[test]
    fn auto_line_ending_falls_back_to_native_without_a_break() {
        let cfg = Config {
            line_ending: LineEnding::Auto,
            ..Config::default()
        };
        let native = if cfg!(windows) { "\r\n" } else { "\n" };
        assert_eq!(format_source("42", &cfg).unwrap(), format!("42{native}"));
    }

    #[test]
    fn crlf_output_reparses_and_is_idempotent() {
        use scarpet_syntax::parser::{parse_source, strip_trivia};
        let cfg = Config {
            line_ending: LineEnding::Crlf,
            ..Config::default()
        };
        let src = "// lead\nfoo(a, b);\nbar()->(x;y)\n";
        let cst1 = parse_source(src).unwrap();
        let formatted = format_cst(&cst1, &cfg);
        assert!(formatted.contains("\r\n"), "expected CRLF in {formatted:?}");
        assert!(!formatted.contains("\r\r"), "doubled CR in {formatted:?}");
        let cst2 = parse_source(&formatted).unwrap();
        assert_eq!(
            strip_trivia(&cst1),
            strip_trivia(&cst2),
            "CRLF formatting changed structure"
        );
        assert_eq!(
            formatted,
            format_cst(&cst2, &cfg),
            "CRLF formatting not idempotent"
        );
    }
}

/// Round-trip the whole `example/` corpus to prove the formatter is safe.
#[cfg(test)]
mod corpus {
    use crate::{BinopSeparator, BraceStyle, Config, TrailingComma, format_cst};
    use scarpet_syntax::parser::{parse_source, strip_trivia};
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    /// Files whose Scarpet source doesn't parse (upstream typos). Mirrors the
    /// list in `scarpet-syntax`'s corpus runner; these are skipped.
    const KNOWN_BAD: &[&str] = &[
        "gnembon/scarpet/programs/survival/portalorient.sc",
        "gnembon/scarpet/programs/survival/rifts/rifts.sc",
        "Ghoulboy78/Scarpet-edit/se.sc",
    ];

    fn corpus_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("example")
    }

    fn walk_sc(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk_sc(&p, out);
            } else if p.extension().and_then(|e| e.to_str()) == Some("sc") {
                out.push(p);
            }
        }
    }

    /// Round-trip every corpus file under `config`, collecting human-readable
    /// failures. Returns an empty list (after printing a skip notice) when the
    /// `example/` submodule isn't checked out.
    fn roundtrip_failures(config: &Config) -> Vec<String> {
        let root = corpus_root();
        if !root.is_dir() {
            eprintln!(
                "skipping corpus test: {} absent (run `git submodule update --init`)",
                root.display()
            );
            return Vec::new();
        }
        let mut files = Vec::new();
        walk_sc(&root, &mut files);
        files.sort();

        let known_bad: HashSet<&str> = KNOWN_BAD.iter().copied().collect();
        let mut failures = Vec::new();
        for f in &files {
            let rel = f
                .strip_prefix(&root)
                .unwrap_or(f)
                .to_string_lossy()
                .replace('\\', "/");
            if known_bad.contains(rel.as_str()) {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(f) else {
                continue;
            };
            let cst1 = match parse_source(&src) {
                Ok(c) => c,
                Err(_) => {
                    failures.push(format!("{rel}: unexpected parse failure"));
                    continue;
                }
            };
            let formatted = format_cst(&cst1, config);
            let cst2 = match parse_source(&formatted) {
                Ok(c) => c,
                Err(e) => {
                    failures.push(format!("{rel}: formatted output failed to parse: {e:?}"));
                    continue;
                }
            };
            if strip_trivia(&cst1) != strip_trivia(&cst2) {
                failures.push(format!("{rel}: structure changed after formatting"));
                continue;
            }
            let reformatted = format_cst(&cst2, config);
            if formatted != reformatted {
                failures.push(format!("{rel}: not idempotent"));
            }
        }
        failures
    }

    /// Every corpus file must format (a) non-destructively — re-parsing the
    /// output yields a structurally-equal CST — and (b) idempotently. Skips
    /// quietly when the `example/` submodule isn't checked out.
    #[test]
    fn roundtrip_is_nondestructive_and_idempotent() {
        let failures = roundtrip_failures(&Config::default());
        assert!(
            failures.is_empty(),
            "corpus round-trip failures ({}):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// The same non-destructive + idempotent guarantee must hold under the
    /// non-default `brace_style = next_line` layout.
    #[test]
    fn roundtrip_next_line_braces_is_nondestructive_and_idempotent() {
        let config = Config {
            brace_style: BraceStyle::NextLine,
            ..Config::default()
        };
        let failures = roundtrip_failures(&config);
        assert!(
            failures.is_empty(),
            "next-line corpus round-trip failures ({}):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// The same guarantee under `trailing_comma = always`, which adds a comma to
    /// flat one-line collections too — the layout that departs furthest from the
    /// default and so most needs the non-destructive + idempotent proof.
    #[test]
    fn roundtrip_always_trailing_comma_is_nondestructive_and_idempotent() {
        let config = Config {
            trailing_comma: TrailingComma::Always,
            ..Config::default()
        };
        let failures = roundtrip_failures(&config);
        assert!(
            failures.is_empty(),
            "always-trailing-comma corpus round-trip failures ({}):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// The same guarantee under `trailing_comma = never`, which strips the comma
    /// the default adds to a broken collection.
    #[test]
    fn roundtrip_never_trailing_comma_is_nondestructive_and_idempotent() {
        let config = Config {
            trailing_comma: TrailingComma::Never,
            ..Config::default()
        };
        let failures = roundtrip_failures(&config);
        assert!(
            failures.is_empty(),
            "never-trailing-comma corpus round-trip failures ({}):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// The same guarantee under `binop_separator = front`, which moves every
    /// wrapping binary operator to the head of its line — the layout that most
    /// stresses the operator-trivia rebinding the non-destructive proof guards.
    #[test]
    fn roundtrip_front_binop_separator_is_nondestructive_and_idempotent() {
        let config = Config {
            binop_separator: BinopSeparator::Front,
            ..Config::default()
        };
        let failures = roundtrip_failures(&config);
        assert!(
            failures.is_empty(),
            "front-binop-separator corpus round-trip failures ({}):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// The same guarantee under `blank_lines_upper_bound = 2`, which keeps runs
    /// of up to two blank lines instead of collapsing every run to one.
    #[test]
    fn roundtrip_blank_lines_upper_bound_is_nondestructive_and_idempotent() {
        let config = Config {
            blank_lines_upper_bound: 2,
            ..Config::default()
        };
        let failures = roundtrip_failures(&config);
        assert!(
            failures.is_empty(),
            "blank-lines-upper-bound corpus round-trip failures ({}):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// The same guarantee under `blank_lines_lower_bound = 1`, which forces a
    /// blank line between every pair of adjacent statements — the layout that
    /// inserts the most trivia, so the strongest test of the non-destructive
    /// (trivia-stripping) and idempotent proofs.
    #[test]
    fn roundtrip_blank_lines_lower_bound_is_nondestructive_and_idempotent() {
        let config = Config {
            blank_lines_lower_bound: 1,
            ..Config::default()
        };
        let failures = roundtrip_failures(&config);
        assert!(
            failures.is_empty(),
            "blank-lines-lower-bound corpus round-trip failures ({}):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}
