use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ariadne::{Label, Report, ReportKind, Source};
use clap::{Args, Parser, Subcommand, ValueEnum};
use rustyline::error::ReadlineError;
use rustyline::{DefaultEditor, EventHandler, KeyCode, KeyEvent, Modifiers};
use scarpet_fmt::{BraceStyle, Config, FmtError, LineEnding, format_source};
use scarpet_syntax::ast::{Code, LowerError};
use scarpet_syntax::parser::{ParseError, has_open_delimiter, parse_source};
use scarpet_vm::{Evalute, GlobalState, ScarpetVm};
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
    /// Start an interactive REPL: read a statement, evaluate it in a session VM
    /// whose variables and function definitions persist across submissions, and
    /// print the resulting value (or a parse / lowering / evaluation
    /// diagnostic), then prompt again. Exits on Ctrl+D.
    Repl,
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
    /// Target maximum comment width. `-1` disables comment wrapping.
    comment_width: Option<isize>,
    /// Line ending for inserted breaks: `"lf"` (default), `"crlf"`, `"auto"`
    /// (match the source), or `"native"` (the host platform's).
    line_ending: Option<String>,
    /// Opening-delimiter placement for broken blocks: `"same_line"` (default)
    /// or `"next_line"`.
    brace_style: Option<String>,
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
        Some("auto") => LineEnding::Auto,
        Some("native") => LineEnding::Native,
        Some(other) => {
            return Err(format!(
                "{name}: line_ending must be \"lf\", \"crlf\", \"auto\", or \"native\", got {other:?}"
            ));
        }
    };
    let brace_style = match file.brace_style.as_deref() {
        None => default.brace_style,
        Some("same_line") => BraceStyle::SameLine,
        Some("next_line") => BraceStyle::NextLine,
        Some(other) => {
            return Err(format!(
                "{name}: brace_style must be \"same_line\" or \"next_line\", got {other:?}"
            ));
        }
    };
    let config = Config {
        indent_width: file.indent.unwrap_or(default.indent_width),
        max_width: file.max_width.unwrap_or(default.max_width),
        comment_width: match file.comment_width {
            None => default.comment_width,
            Some(-1) => None,
            Some(n) if n > 0 => Some(n as usize),
            Some(n) => {
                return Err(format!(
                    "{name}: comment_width must be -1 or at least 1, got {n}"
                ));
            }
        },
        line_ending,
        brace_style,
    };
    if config.max_width == 0 {
        return Err(format!("{name}: max_width must be at least 1"));
    }
    Ok(config)
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
        Cmd::Format(args) => run_format(args),
        Cmd::Repl => run_repl(),
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

/// Run an interactive REPL. Reads one statement per submission, parses it with
/// [`parse_source`], lowers the CST to a `scarpet-syntax` [`Code`] (statement
/// sequence) via `Code::try_from`, then evaluates it in a session VM and prints
/// the resulting value — or a rustc-style diagnostic for a parse, lowering, or
/// evaluation error. The VM is created once per session, so variables and
/// function definitions persist from one submission to the next.
///
/// On a terminal, input is read through `rustyline` for line editing and an
/// in-session command history; see [`run_repl_interactive`]. When stdin is not a
/// terminal (piped or redirected input), it falls back to a plain line reader
/// with no prompt or banner — so `echo 'a=5' | scarpet repl` emits just the
/// value (and `echo '1+' | scarpet repl` just the diagnostic); see
/// [`run_repl_piped`].
fn run_repl() -> ExitCode {
    if std::io::stdin().is_terminal() {
        run_repl_interactive()
    } else {
        run_repl_piped()
    }
}

/// The terminal REPL, backed by `rustyline` for line editing and an up/down
/// command history. The history is kept only for the session — it is not
/// written to disk.
///
/// A submission may span several lines, read one physical line at a time like
/// Python's REPL. The first line uses the `scarpet> ` prompt; while the input
/// gathered so far still has an unclosed `(`/`[`/`{`, each further line is read
/// with a `.......| ` continuation prompt sized to line up under the first, and
/// the submission is parsed only once the brackets balance. Shift+Enter — and
/// Alt+Enter, its fallback on terminals that send the same bytes for Enter and
/// Shift+Enter — inserts a newline within the line being edited, so input the
/// bracket check can't tell is unfinished (such as a trailing `->`) can still be
/// continued onto another line.
///
/// Ctrl+C abandons the submission in progress and prompts again; Ctrl+D (or end
/// of input) exits.
fn run_repl_interactive() -> ExitCode {
    // The first-line and continuation prompts. The continuation prompt is the
    // same display width as `scarpet> ` so input starts in the same column on
    // every line.
    const PROMPT: &str = "scarpet> ";
    const CONTINUATION: &str = ".......| ";

    let mut rl = match DefaultEditor::new() {
        Ok(rl) => rl,
        Err(e) => {
            eprintln!("repl: {e}");
            return ExitCode::from(2);
        }
    };
    // Bind Shift+Enter (and Alt+Enter, its fallback on terminals that send the
    // same bytes for Enter and Shift+Enter) to insert a newline, so a line can
    // be continued even when its brackets already balance.
    rl.bind_sequence(
        KeyEvent(KeyCode::Enter, Modifiers::SHIFT),
        EventHandler::Simple(rustyline::Cmd::Newline),
    );
    rl.bind_sequence(
        KeyEvent(KeyCode::Enter, Modifiers::ALT),
        EventHandler::Simple(rustyline::Cmd::Newline),
    );
    eprintln!(
        "Scarpet REPL. Enter submits (continuing while brackets \
         are open); Shift+Enter or Alt+Enter forces a newline; Ctrl+D exits."
    );
    // One VM for the whole session, so a variable or function defined in one
    // submission is visible to the next. The VM (and any function it defines)
    // borrows its source for `'src`; each submission is leaked to `'static`
    // below so a single session-long source lifetime works out.
    let mut global: GlobalState<'static> = GlobalState::new();
    let mut vm = global.create_new_vm();
    // Each iteration of the outer loop reads one submission, joining physical
    // lines with `\n` into `buf` until the brackets balance.
    'submission: loop {
        let mut buf = String::new();
        loop {
            let prompt = if buf.is_empty() { PROMPT } else { CONTINUATION };
            match rl.readline(prompt) {
                Ok(line) => {
                    // `rustyline` returns the line without its trailing newline,
                    // so reinsert one between continued lines.
                    if !buf.is_empty() {
                        buf.push('\n');
                    }
                    buf.push_str(&line);
                    // Keep the submission open for another line while a bracket
                    // is still unclosed; otherwise it is ready to parse.
                    if input_incomplete(&buf) {
                        continue;
                    }
                    break;
                }
                // Ctrl+C: drop the whole submission in progress, prompt afresh.
                Err(ReadlineError::Interrupted) => continue 'submission,
                // Ctrl+D or end of input: leave the REPL.
                Err(ReadlineError::Eof) => return ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("repl: {e}");
                    return ExitCode::from(2);
                }
            }
        }
        // Blank submissions are ignored and kept out of the history, matching
        // the piped path and Python's REPL.
        if buf.trim().is_empty() {
            continue;
        }
        // Record every non-blank submission, valid or not, so the history
        // mirrors exactly what was typed.
        let _ = rl.add_history_entry(buf.as_str());
        // Leak this submission to `'static` so the session VM can borrow it
        // (and anything it defines from it) for the rest of the run.
        let src: &'static str = Box::leak(buf.into_boxed_str());
        handle_submission(&mut vm, "<repl>", src);
    }
}

/// The non-terminal REPL: a plain line reader for piped or redirected stdin,
/// with no prompt or banner. Reads one statement per line, evaluating it in the
/// session VM and printing the resulting value on success, or a diagnostic on a
/// parse, lowering, or evaluation error — so `echo '1+' | scarpet repl` emits
/// only the diagnostic. Like the interactive path, one VM serves the whole
/// session, so definitions persist line to line. Ends at end-of-input.
fn run_repl_piped() -> ExitCode {
    use std::io::BufRead as _;

    // One VM for the whole session (see [`run_repl_interactive`]): each line is
    // leaked to `'static` so the VM can borrow it for the rest of the run.
    let mut global: GlobalState<'static> = GlobalState::new();
    let mut vm = global.create_new_vm();

    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut line = String::new();
    loop {
        line.clear();
        match input.read_line(&mut line) {
            // End of input.
            Ok(0) => return ExitCode::SUCCESS,
            Ok(_) => {
                // Drop the trailing newline, then take ownership of the line so
                // it can be leaked to `'static` for the session VM to borrow.
                let len = line.trim_end_matches(['\n', '\r']).len();
                line.truncate(len);
                let src: &'static str = Box::leak(std::mem::take(&mut line).into_boxed_str());
                handle_submission(&mut vm, "<repl>", src);
            }
            Err(e) => {
                eprintln!("repl: {e}");
                return ExitCode::from(2);
            }
        }
    }
}

/// The outcome of checking one REPL submission.
enum Checked<'s> {
    /// Whitespace-only input — nothing to do.
    Blank,
    /// Parsed and lowered cleanly; the statement-sequence AST (borrowing `src`).
    Ast(Code<'s>),
    /// Failed to parse.
    Parse(ParseError),
    /// Parsed, but could not be lowered to an AST.
    Lower(LowerError),
}

/// Check one REPL submission: parse it, then lower the CST to a [`Code`] (a
/// `;`-separated statement sequence — the natural root for a submission, not the
/// comma-level `Args`). Blank (whitespace-only) lines are ignored — pressing
/// Enter at an empty prompt does nothing, as in Python's REPL. The
/// [`ParseError`] / [`LowerError`] own their data; the [`Code`] AST borrows
/// `src`'s identifiers and literals.
fn check_line(src: &str) -> Checked<'_> {
    if src.trim().is_empty() {
        return Checked::Blank;
    }
    let cst = match parse_source(src) {
        Ok(cst) => cst,
        Err(e) => return Checked::Parse(*e),
    };
    match Code::try_from(&cst) {
        Ok(code) => Checked::Ast(code),
        Err(e) => Checked::Lower(e),
    }
}

/// Check one REPL submission and report the outcome: evaluate it in `vm` — the
/// session VM, so any variable or function it sets persists to later
/// submissions — and print the resulting value to stdout, or a rustc-style
/// diagnostic to stderr for a parse, lowering, or evaluation error. An
/// evaluation error leaves the VM intact rather than ending the session. Blank
/// submissions produce no output. `name` labels the source in diagnostics.
fn handle_submission(vm: &mut ScarpetVm<'_, 'static>, name: &str, src: &'static str) {
    match check_line(src) {
        Checked::Blank => {}
        // Evaluate in the session VM and print the value (still its `Debug`
        // form). A `VmError` — or a poisoned value lock — is reported by its
        // human-readable `Display`, without tearing the REPL down, so the next
        // prompt still has the same VM.
        Checked::Ast(ast) => match vm.push(ast) {
            Ok(value) => match value.lock() {
                Ok(v) => println!("{v:?}"),
                Err(e) => eprintln!("repl: {e}"),
            },
            Err(e) => eprintln!("repl: {e}"),
        },
        Checked::Parse(e) => report_parse_error(name, src, &e),
        Checked::Lower(e) => report_lower_error(name, src, &e),
    }
}

/// Whether `src` is an unfinished submission the REPL should keep open for more
/// input instead of parsing now: true while a `(`/`[`/`{` is still unclosed.
/// Delegates to [`has_open_delimiter`], which runs the Scarpet lexer, so a
/// bracket inside a `'…'` string or a `//` comment never counts. A surplus
/// *closing* bracket is not treated as incomplete — the input is submitted so
/// the parser can report it. Anything else that should span lines can still be
/// continued with Shift+Enter / Alt+Enter.
fn input_incomplete(src: &str) -> bool {
    has_open_delimiter(src)
}

/// Render a parse error to stderr as a rustc-style ariadne diagnostic that
/// underlines the offending span in `src`. `name` labels the source — a file
/// path, or `<stdin>`. Colour is auto-disabled when stderr isn't a terminal.
fn report_parse_error(name: &str, src: &str, e: &ParseError) {
    // The title carries the full rustc-style line; the caret label is the
    // shorter `caret_label` (an `expected …` clause or a delimiter headline).
    // A delimiter error adds a second label at its opener; some errors a `help:`.
    let mut report = Report::build(ReportKind::Error, (name, e.span.clone()))
        .with_message(e.message())
        .with_label(Label::new((name, e.span.clone())).with_message(e.caret_label()));
    if let Some((span, label)) = &e.secondary {
        report = report.with_label(Label::new((name, span.clone())).with_message(label));
    }
    if let Some(help) = &e.help {
        report = report.with_help(help);
    }
    let _ = report.finish().eprint((name, Source::from(src)));
}

/// Render an AST-lowering error to stderr in the same rustc-style ariadne form
/// as [`report_parse_error`]. A [`LowerError`] carries no source span (the CST
/// has none), so the label spans the whole submission rather than one token.
fn report_lower_error(name: &str, src: &str, e: &LowerError) {
    let span = 0..src.len();
    let _ = Report::build(ReportKind::Error, (name, span.clone()))
        .with_message(e.to_string())
        .with_label(Label::new((name, span)).with_message("cannot be lowered to an AST"))
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
            Cmd::Repl => unreachable!("parse_format only parses `format` invocations"),
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
    fn parse_config_reads_auto_line_ending() {
        let cfg = parse_config("line_ending = \"auto\"", "x").unwrap();
        assert_eq!(cfg.line_ending, LineEnding::Auto);
    }

    #[test]
    fn parse_config_reads_native_line_ending() {
        let cfg = parse_config("line_ending = \"native\"", "x").unwrap();
        assert_eq!(cfg.line_ending, LineEnding::Native);
    }

    #[test]
    fn parse_config_defaults_comment_width_to_disabled() {
        assert_eq!(parse_config("", "x").unwrap().comment_width, None);
    }

    #[test]
    fn parse_config_reads_comment_width() {
        let cfg = parse_config("comment_width = 72", "x").unwrap();
        assert_eq!(cfg.comment_width, Some(72));
    }

    #[test]
    fn parse_config_disables_comment_width_with_minus_one() {
        let cfg = parse_config("comment_width = -1", "x").unwrap();
        assert_eq!(cfg.comment_width, None);
    }

    #[test]
    fn parse_config_rejects_zero_comment_width() {
        let err = parse_config("comment_width = 0", "x").unwrap_err();
        assert!(err.contains("comment_width"), "{err}");
    }

    #[test]
    fn parse_config_rejects_unknown_line_ending() {
        let err = parse_config("line_ending = \"mac\"", "x").unwrap_err();
        assert!(err.contains("line_ending"), "{err}");
    }

    #[test]
    fn parse_config_defaults_brace_style_to_same_line() {
        assert_eq!(
            parse_config("", "x").unwrap().brace_style,
            BraceStyle::SameLine
        );
    }

    #[test]
    fn parse_config_reads_next_line() {
        let cfg = parse_config("brace_style = \"next_line\"", "x").unwrap();
        assert_eq!(cfg.brace_style, BraceStyle::NextLine);
    }

    #[test]
    fn parse_config_rejects_unknown_brace_style() {
        let err = parse_config("brace_style = \"allman\"", "x").unwrap_err();
        assert!(err.contains("brace_style"), "{err}");
    }

    #[test]
    fn repl_ignores_blank_lines() {
        // Pressing Enter at an empty prompt must do nothing, even though an
        // empty program does not parse.
        assert!(matches!(check_line(""), Checked::Blank));
        assert!(matches!(check_line("   "), Checked::Blank));
        assert!(matches!(check_line("\t  "), Checked::Blank));
    }

    #[test]
    fn repl_lowers_valid_statements() {
        assert!(matches!(
            check_line("print('Hello World!')"),
            Checked::Ast(_)
        ));
        assert!(matches!(check_line("a = 5"), Checked::Ast(_)));
        assert!(matches!(check_line("foo(a, b) -> a + b"), Checked::Ast(_)));
        // A lone trailing `;` is lenient, and several `;`-separated statements
        // on one line are a single valid program.
        assert!(matches!(check_line("a = 5;"), Checked::Ast(_)));
        assert!(matches!(check_line("a; b; c"), Checked::Ast(_)));
    }

    #[test]
    fn repl_reports_parse_errors() {
        // Incomplete input (ends mid-expression) and stray tokens both fail.
        assert!(matches!(check_line("1 +"), Checked::Parse(_)));
        assert!(matches!(check_line(")"), Checked::Parse(_)));
        assert!(matches!(check_line("print("), Checked::Parse(_)));
    }

    #[test]
    fn repl_reports_lowering_errors() {
        // These parse, but cannot be lowered: a top-level comma has no `;`-level
        // to land in, and two `...rest` binders share one parameter list. (A
        // non-assignable target like `1 = 2` now lowers and fails at evaluation
        // instead.)
        assert!(matches!(check_line("a, b"), Checked::Lower(_)));
        assert!(matches!(
            check_line("f(...a, ...b) -> 0"),
            Checked::Lower(_)
        ));
    }

    #[test]
    fn repl_continues_while_brackets_open() {
        // An unclosed bracket keeps the submission open for another line.
        assert!(input_incomplete("foo("));
        assert!(input_incomplete("[1, 2,"));
        assert!(input_incomplete("foo() -> ("));
        // Balanced input is complete and ready to submit.
        assert!(!input_incomplete("foo()"));
        assert!(!input_incomplete("a = 5"));
        assert!(!input_incomplete("(a + b) * [c]"));
        // A surplus close is submitted, not held open, so the parser reports it.
        assert!(!input_incomplete("a)"));
    }

    #[test]
    fn repl_ignores_brackets_in_strings_and_comments() {
        // A bracket inside a string literal doesn't count toward the depth.
        assert!(!input_incomplete("print('(')"));
        assert!(!input_incomplete("a = ')['"));
        // Brackets after `//` are a comment and are ignored.
        assert!(!input_incomplete("foo() // (open"));
        // A multi-line string is still held open by its enclosing bracket, so a
        // string can be continued onto the next line inside a call.
        assert!(input_incomplete("print('hello"));
    }

    #[test]
    fn repl_parses_reassembled_multiline_submission() {
        // The interactive loop joins continuation lines with '\n' before
        // parsing, so a bracketed body split across several lines must parse as
        // a single submission.
        assert!(matches!(check_line("foo(\n  1,\n  2\n)"), Checked::Ast(_)));
        assert!(matches!(check_line("[\n  1,\n  2\n]"), Checked::Ast(_)));
    }
}
