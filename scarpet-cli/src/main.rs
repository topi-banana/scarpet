use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ariadne::{Label, Report, ReportKind, Source};
use clap::{Args, Parser, Subcommand, ValueEnum};
use scarpet_fmt::{Config, FmtError, LineEnding, format_source};
use scarpet_syntax::parser::ParseError;
use serde::Deserialize;
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
    /// Path to a TOML config file. Defaults to `scarpet-fmt.toml` in the
    /// current directory when present.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Promote a warning class to a hard error, like clippy's `-D warnings`.
    /// Pass `warnings` so that an unformatted file makes `--check` exit
    /// non-zero instead of only printing its diff. Repeatable; needs `--check`.
    #[arg(short = 'D', long = "deny", value_name = "WARNING", requires = "check")]
    deny: Vec<DenyWarning>,
}

/// A warning class that `-D`/`--deny` promotes to a hard error, mirroring
/// clippy's `-D warnings`. Only `warnings` exists today — an unformatted file
/// under `--check` — but the value-taking shape leaves room to name more.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum DenyWarning {
    /// Any formatting difference: a file that is not already formatted.
    Warnings,
}

/// Whether `-D warnings` was passed — i.e. formatting differences should fail
/// the run instead of only being reported.
fn diffs_denied(deny: &[DenyWarning]) -> bool {
    deny.contains(&DenyWarning::Warnings)
}

/// The default config file, read from the current directory when `--config`
/// is not supplied.
const DEFAULT_CONFIG: &str = "scarpet-fmt.toml";

/// The TOML config schema. Every key is optional; unset keys fall back to the
/// formatter's defaults. Parsing lives here in the CLI so that `scarpet-fmt`
/// stays free of file I/O (it builds for `wasm`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    /// Indentation step, in spaces.
    indent: Option<usize>,
    /// Target maximum line width before a group breaks.
    max_width: Option<usize>,
    /// Line ending for inserted breaks: `"lf"` (default) or `"crlf"`.
    line_ending: Option<String>,
}

/// Resolve the formatting [`Config`]. An explicit `--config` path must exist
/// and parse. Otherwise `scarpet-fmt.toml` in the current directory is used if
/// present; a missing default file falls back to [`Config::default`].
fn resolve_config(explicit: Option<&Path>) -> Result<Config, String> {
    let (text, name) = match explicit {
        Some(path) => {
            let s =
                std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            (s, path.display().to_string())
        }
        None => match std::fs::read_to_string(DEFAULT_CONFIG) {
            Ok(s) => (s, DEFAULT_CONFIG.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(format!("{DEFAULT_CONFIG}: {e}")),
        },
    };
    parse_config(&text, &name)
}

/// Parse TOML config `text` into a [`Config`], filling unset keys from
/// [`Config::default`]. `name` labels the source in error messages. Split out
/// from [`resolve_config`] so it is unit-testable without touching the
/// filesystem.
fn parse_config(text: &str, name: &str) -> Result<Config, String> {
    let file: ConfigFile = toml::from_str(text).map_err(|e| format!("{name}: {e}"))?;
    let default = Config::default();
    let line_ending = match file.line_ending.as_deref() {
        None => default.line_ending,
        Some("lf") => LineEnding::Lf,
        Some("crlf") => LineEnding::Crlf,
        Some(other) => {
            return Err(format!(
                "{name}: line_ending must be \"lf\" or \"crlf\", got {other:?}"
            ));
        }
    };
    let config = Config {
        indent_width: file.indent.unwrap_or(default.indent_width),
        max_width: file.max_width.unwrap_or(default.max_width),
        line_ending,
        brace_style: default.brace_style,
    };
    if config.max_width == 0 {
        return Err(format!("{name}: max_width must be at least 1"));
    }
    Ok(config)
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
        Cmd::Format(args) => run_format(args),
    }
}

fn run_format(args: FormatArgs) -> ExitCode {
    let config = match resolve_config(args.config.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    if args.files.is_empty() {
        return format_stdin(args.check, &args.deny, &config);
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
        match format_source(&src, &config) {
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
            if diffs_denied(&args.deny) {
                return Some(ExitCode::FAILURE);
            }
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

fn format_stdin(check: bool, deny: &[DenyWarning], config: &Config) -> ExitCode {
    let mut src = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut src) {
        eprintln!("stdin: {e}");
        return ExitCode::from(2);
    }
    match format_source(&src, config) {
        Ok(formatted) => {
            if check {
                if formatted != src {
                    print_diff("<stdin>", &src, &formatted);
                    if diffs_denied(deny) {
                        return ExitCode::FAILURE;
                    }
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

    /// Parse a `format` invocation, returning its args (or the clap error).
    fn parse_format(argv: &[&str]) -> Result<FormatArgs, clap::Error> {
        Cli::try_parse_from(argv.iter().copied()).map(|cli| match cli.cmd {
            Cmd::Format(args) => args,
        })
    }

    #[test]
    fn deny_warnings_requires_check() {
        // `-D warnings` on its own is rejected...
        assert!(parse_format(&["scarpet", "format", "-D", "warnings", "f.sc"]).is_err());
        // ...but is accepted together with `--check`.
        let args = parse_format(&["scarpet", "format", "--check", "-D", "warnings"]).unwrap();
        assert!(diffs_denied(&args.deny));
    }

    #[test]
    fn deny_rejects_unknown_warning_class() {
        assert!(parse_format(&["scarpet", "format", "--check", "-D", "bogus"]).is_err());
    }

    #[test]
    fn check_without_deny_does_not_promote_diffs() {
        let args = parse_format(&["scarpet", "format", "--check"]).unwrap();
        assert!(!diffs_denied(&args.deny));
    }

    #[test]
    fn parse_config_defaults_line_ending_to_lf() {
        assert_eq!(parse_config("", "x").unwrap().line_ending, LineEnding::Lf);
    }

    #[test]
    fn parse_config_reads_crlf() {
        let cfg = parse_config("line_ending = \"crlf\"", "x").unwrap();
        assert_eq!(cfg.line_ending, LineEnding::Crlf);
    }

    #[test]
    fn parse_config_rejects_unknown_line_ending() {
        let err = parse_config("line_ending = \"mac\"", "x").unwrap_err();
        assert!(err.contains("line_ending"), "{err}");
    }
}
