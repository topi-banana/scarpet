use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use scarpet_fmt::format_source;

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
            Err(e) => {
                eprintln!("{}: {e}", path.display());
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
                    return ExitCode::FAILURE;
                }
            } else {
                print!("{formatted}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("stdin: {e}");
            ExitCode::FAILURE
        }
    }
}
