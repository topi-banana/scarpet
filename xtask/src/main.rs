//! Workspace automation tasks, invoked as `cargo xtask <command>`
//! (the alias lives in `.cargo/config.toml`).

mod codegen;

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    let flag = args.next();
    if args.next().is_some() {
        eprintln!("error: unexpected extra arguments");
        print_usage();
        return ExitCode::FAILURE;
    }
    match command.as_deref() {
        Some("codegen") => {
            let mode = match flag.as_deref() {
                None => codegen::Mode::Overwrite,
                Some("--check") => codegen::Mode::Verify,
                Some(other) => {
                    eprintln!("error: unknown flag `{other}`");
                    print_usage();
                    return ExitCode::FAILURE;
                }
            };
            match codegen::run(mode) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("error: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("--help" | "-h" | "help") => {
            print_usage();
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("error: unknown task `{other}`");
            print_usage();
            ExitCode::FAILURE
        }
        None => {
            print_usage();
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    eprintln!(
        "\
usage: cargo xtask <command>

commands:
    codegen [--check]    regenerate scarpet-syntax/src/syntax_kind.rs and
                         scarpet-syntax/src/cst/generated.rs from scarpet.ungram
                         (--check: verify they are up to date instead of writing)"
    );
}
