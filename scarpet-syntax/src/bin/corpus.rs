//! Stand-alone corpus runner: walks every `.sc` file under
//! `example/<org>/<repo>/`, parses each, and reports the parse rate. For every
//! file that fails it also captures the parser error and the offending source
//! line so the report shows *why* it failed, not just *which* file. This is a
//! progress metric, not a gate — it always exits 0 regardless of how many files
//! fail to parse (the only non-zero exit is a missing corpus root).
//!
//! Run with `cargo run -p scarpet-syntax --bin corpus`. Pass `--markdown` to
//! emit a GitHub-flavoured Markdown report (CI feeds it into the job summary and
//! the sticky PR comment) instead of the plain-text summary.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use scarpet_syntax::parser::{Code, ParseError};

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

#[derive(Debug)]
struct Failure {
    path: String,
    /// Multi-line, human-readable error: kind, location, and a source excerpt.
    error: String,
}

#[derive(Debug, Default)]
struct Outcome {
    total: usize,
    failed: Vec<Failure>,
}

impl Outcome {
    /// Files that parsed cleanly.
    fn parsed(&self) -> usize {
        self.total - self.failed.len()
    }

    /// Share of the corpus that parsed, in percent (0 for an empty corpus).
    fn parse_rate(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.parsed() as f64 / self.total as f64 * 100.0
    }
}

/// Render a parse error as a `rustc`-style excerpt:
///
/// ```text
/// unexpected token at line 12:5
/// 12 | foo(bar baz)
///         ^
/// ```
fn describe_error(src: &str, e: &ParseError) -> String {
    let kind = e.kind.message();

    // Clamp to a char boundary so slicing can never panic on a stray offset.
    let mut at = e.span.start.min(src.len());
    while at > 0 && !src.is_char_boundary(at) {
        at -= 1;
    }

    let line_start = src[..at].rfind('\n').map_or(0, |i| i + 1);
    let line_no = src[..at].bytes().filter(|&b| b == b'\n').count() + 1;
    let line_end = src[at..].find('\n').map_or(src.len(), |i| at + i);
    let line_text = src[line_start..line_end].trim_end_matches('\r');

    let caret_col = src[line_start..at].chars().count();
    let gutter = format!("{line_no} | ");
    // Keep tabs in the caret padding (spaces elsewhere) so the marker lines up
    // no matter how wide the viewer renders a tab.
    let prefix: String = src[line_start..at]
        .chars()
        .map(|c| if c == '\t' { '\t' } else { ' ' })
        .collect();
    let pad = format!("{}{prefix}", " ".repeat(gutter.chars().count()));
    format!(
        "{kind} at line {line_no}:{}\n{gutter}{line_text}\n{pad}^",
        caret_col + 1
    )
}

/// Walk the corpus and parse every `.sc` file. Returns Err only if the root is
/// missing (a corpus full of parse failures is still a successful run).
fn run() -> Result<Outcome, String> {
    let root = corpus_root();
    if !root.is_dir() {
        return Err(format!(
            "corpus missing at {} — run `git submodule update --init`",
            root.display()
        ));
    }
    let mut files = Vec::new();
    walk_sc(&root, &mut files);
    files.sort();

    let mut out = Outcome {
        total: files.len(),
        ..Default::default()
    };
    for f in &files {
        let rel = f
            .strip_prefix(&root)
            .unwrap_or(f)
            .to_string_lossy()
            .replace('\\', "/");
        let src = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => {
                out.failed.push(Failure {
                    path: rel,
                    error: format!("could not read file: {e}"),
                });
                continue;
            }
        };
        let parsed = match Code::from_source(&src) {
            Ok(code) => code.parse(),
            Err(_) => {
                out.failed.push(Failure {
                    path: rel,
                    error: "lexing failed".to_string(),
                });
                continue;
            }
        };
        if let Err(e) = parsed {
            out.failed.push(Failure {
                path: rel,
                error: describe_error(&src, &e),
            });
        }
    }
    Ok(out)
}

/// Plain-text report for local runs.
fn render_human(o: &Outcome) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "corpus: {} files | parsed={} ({:.1}%) failed={}",
        o.total,
        o.parsed(),
        o.parse_rate(),
        o.failed.len(),
    );
    if !o.failed.is_empty() {
        let _ = writeln!(s, "\nFailed to parse ({}):", o.failed.len());
        for fail in &o.failed {
            let _ = writeln!(s, "\n{}", fail.path);
            for line in fail.error.lines() {
                let _ = writeln!(s, "  {line}");
            }
        }
    }
    s
}

/// GitHub-flavoured Markdown report for CI summaries and PR comments.
fn render_markdown(o: &Outcome) -> String {
    let mut s = String::new();
    let status = if o.failed.is_empty() { "✅" } else { "⚠️" };

    let _ = writeln!(s, "### Corpus parse results");
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "{status} **Parse rate: {:.1}%** — {} / {} files parsed",
        o.parse_rate(),
        o.parsed(),
        o.total,
    );

    if !o.failed.is_empty() {
        let _ = writeln!(s);
        let _ = writeln!(s, "<details>");
        let _ = writeln!(s, "<summary>Failed to parse ({})</summary>", o.failed.len());
        let _ = writeln!(s);
        for fail in &o.failed {
            let _ = writeln!(s, "`{}`", fail.path);
            let _ = writeln!(s);
            let _ = writeln!(s, "```text");
            let _ = writeln!(s, "{}", fail.error);
            let _ = writeln!(s, "```");
            let _ = writeln!(s);
        }
        let _ = writeln!(s, "</details>");
    }
    s
}

fn main() -> ExitCode {
    let markdown = std::env::args().skip(1).any(|a| a == "--markdown");

    let outcome = match run() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    let report = if markdown {
        render_markdown(&outcome)
    } else {
        render_human(&outcome)
    };
    print!("{report}");

    // Parse failures are a metric, not a gate — always succeed.
    ExitCode::SUCCESS
}
