use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ariadne::{Label, Report, ReportKind, Source};
use clap::{Args, Parser, Subcommand};
use scarpet_fmt::{FmtError, format_source};
use scarpet_syntax::parser::ParseError;
use similar::{ChangeTag, TextDiff};

#[derive(Parser)]
#[command(name = "scarpet", about = "Scarpet language tools")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Format Scarpet source.
    Format(FormatArgs),
}

#[derive(Args)]
struct FormatArgs {
    /// Files to format. Reads stdin if none are given.
    files: Vec<PathBuf>,
    /// Overwrite each file in place instead of writing to stdout.
    #[arg(short = 'w', long = "in-place", conflicts_with = "check")]
    in_place: bool,
    /// Exit non-zero if any input is not already formatted; write nothing.
    #[arg(long)]
    check: bool,
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
        Cmd::Format(args) => run_format(args),
    }
}

fn run_format(args: FormatArgs) -> ExitCode {
    if args.files.is_empty() {
        return format_stdin(args.check);
    }
    let mut code = ExitCode::SUCCESS;
    for path in &args.files {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                return ExitCode::from(2);
            }
        };
        match format_source(&src) {
            Ok(formatted) => {
                if let Some(c) = apply(path, &src, &formatted, &args) {
                    code = c;
                }
            }
            Err(FmtError::Parse(e)) => {
                report_parse_error(&path.display().to_string(), &src, &e);
                code = ExitCode::FAILURE;
            }
        }
    }
    code
}

/// Emit one file's result per the mode (check / in-place / stdout). Returns a
/// non-success code to fold in, or `None` to leave the running code unchanged.
fn apply(path: &Path, src: &str, formatted: &str, args: &FormatArgs) -> Option<ExitCode> {
    if args.check {
        if formatted != src {
            print_diff(&path.display().to_string(), src, formatted);
            return Some(ExitCode::FAILURE);
        }
    } else if args.in_place {
        if formatted != src
            && let Err(e) = std::fs::write(path, formatted)
        {
            eprintln!("{}: {e}", path.display());
            return Some(ExitCode::from(2));
        }
    } else {
        print!("{formatted}");
    }
    None
}

fn format_stdin(check: bool) -> ExitCode {
    let mut src = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut src) {
        eprintln!("stdin: {e}");
        return ExitCode::from(2);
    }
    match format_source(&src) {
        Ok(formatted) => {
            if check {
                if formatted != src {
                    print_diff("<stdin>", &src, &formatted);
                    return ExitCode::FAILURE;
                }
            } else {
                print!("{formatted}");
            }
            ExitCode::SUCCESS
        }
        Err(FmtError::Parse(e)) => {
            report_parse_error("<stdin>", &src, &e);
            ExitCode::FAILURE
        }
    }
}

/// Render a parse error to stderr as a rustc-style ariadne diagnostic that
/// underlines the offending span in `src`. `name` labels the source — a file
/// path, or `<stdin>`. Colour is auto-disabled when stderr isn't a terminal.
fn report_parse_error(name: &str, src: &str, e: &ParseError) {
    let msg = e.kind.message();
    let _ = Report::build(ReportKind::Error, (name, e.span.clone()))
        .with_message(msg)
        .with_label(Label::new((name, e.span.clone())).with_message(msg))
        .finish()
        .eprint((name, Source::from(src)));
}

/// Print a rustfmt-style unified diff of `src` (the original) against
/// `formatted` (how it should look) to stdout. Changes are grouped into hunks
/// with three lines of context; each hunk is headed `Diff in <name> at line
/// <N>:`, where `N` is the 1-based line in the original. Removed (original)
/// lines are prefixed `-`, inserted (formatted) lines `+`. Colour is
/// auto-disabled when stdout isn't a terminal.
fn print_diff(name: &str, src: &str, formatted: &str) {
    print!(
        "{}",
        render_diff(name, src, formatted, std::io::stdout().is_terminal())
    );
}

/// Build the diff text for [`print_diff`]. Split out so it can be unit tested
/// off a terminal; `color` toggles ANSI colouring of the `+`/`-` lines.
fn render_diff(name: &str, src: &str, formatted: &str, color: bool) -> String {
    use std::fmt::Write as _;

    let diff = TextDiff::from_lines(src, formatted);
    let mut out = String::new();
    for group in diff.grouped_ops(3) {
        let start = group[0].old_range().start + 1;
        let _ = writeln!(out, "Diff in {name} at line {start}:");
        for op in &group {
            for change in diff.iter_changes(op) {
                let (sign, paint) = match change.tag() {
                    ChangeTag::Delete => ('-', color.then_some("\x1b[31m")),
                    ChangeTag::Insert => ('+', color.then_some("\x1b[32m")),
                    ChangeTag::Equal => (' ', None),
                };
                let line = change.value();
                let line = line.strip_suffix('\n').unwrap_or(line);
                match paint {
                    Some(c) => {
                        let _ = writeln!(out, "{c}{sign}{line}\x1b[0m");
                    }
                    None => {
                        let _ = writeln!(out, "{sign}{line}");
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_renders_hunk_header_and_signs() {
        let src = "{\n\tfoo( a , b )\n}\n";
        let formatted = "{\n\tfoo(a, b)\n}\n";
        let out = render_diff("example.sc", src, formatted, false);
        assert!(out.contains("Diff in example.sc at line 1:"), "{out}");
        assert!(out.contains("-\tfoo( a , b )"), "{out}");
        assert!(out.contains("+\tfoo(a, b)"), "{out}");
        assert!(out.contains(" {\n"), "context line kept: {out}");
    }

    #[test]
    fn diff_is_plain_when_color_off() {
        let out = render_diff("x", "a\n", "b\n", false);
        assert!(!out.contains('\x1b'), "{out}");
    }

    #[test]
    fn diff_colours_added_and_removed_lines() {
        let out = render_diff("x", "a\n", "b\n", true);
        assert!(out.contains("\x1b[31m-a\x1b[0m"), "{out}");
        assert!(out.contains("\x1b[32m+b\x1b[0m"), "{out}");
    }
}
