use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ariadne::{Label, Report, ReportKind, Source};
use clap::{Args, Parser, Subcommand};
use scarpet_fmt::{Config, FmtError, format_source};
use scarpet_syntax::parser::ParseError;
use serde::Deserialize;

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
    let file: ConfigFile = toml::from_str(&text).map_err(|e| format!("{name}: {e}"))?;
    let default = Config::default();
    let config = Config {
        indent_width: file.indent.unwrap_or(default.indent_width),
        max_width: file.max_width.unwrap_or(default.max_width),
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
        return format_stdin(args.check, &config);
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
            println!("{}: not formatted", path.display());
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

fn format_stdin(check: bool, config: &Config) -> ExitCode {
    let mut src = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut src) {
        eprintln!("stdin: {e}");
        return ExitCode::from(2);
    }
    match format_source(&src, config) {
        Ok(formatted) => {
            if check {
                if formatted != src {
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
