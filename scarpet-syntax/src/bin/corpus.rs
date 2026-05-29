//! Stand-alone integration runner: walks every `.sc` file under
//! `example/<org>/<repo>/` and prints a summary. Exits with code 1 if any file
//! parsed unexpectedly (either an unannounced failure or a `KNOWN_BAD` entry
//! that has since started parsing cleanly).
//!
//! Run with `cargo run -p scarpet-syntax --bin corpus`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use scarpet_syntax::parser::Code;

/// Files whose Scarpet source contains a parse-blocking typo upstream.
/// Paths are relative to the workspace `example/` root.
const KNOWN_BAD: &[&str] = &[
    // `sidelist = l[];` — relies on the legacy `[` → `l(` preprocessor desugar
    "gnembon/scarpet/programs/survival/portalorient.sc",
    // `if(decor, ..., '']` — closing bracket mismatched (should be `)`)
    "gnembon/scarpet/programs/survival/rifts/rifts.sc",
    // Two adjacent list literals with a missing `,` between them
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

#[derive(Debug, Default)]
struct Outcome {
    total: usize,
    ok: usize,
    expected_failures: usize,
    unexpected_failures: Vec<String>,
    unexpected_passes: Vec<String>,
}

/// Walk the corpus and classify each file against `KNOWN_BAD`. Returns Err if
/// the root is missing.
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

    let known_bad: HashSet<&str> = KNOWN_BAD.iter().copied().collect();
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
        let is_known_bad = known_bad.contains(rel.as_str());
        let parsed = std::fs::read_to_string(f).is_ok_and(|src| {
            Code::from_source(&src)
                .ok()
                .and_then(|c| c.parse().ok())
                .is_some()
        });
        match (parsed, is_known_bad) {
            (true, false) => out.ok += 1,
            (true, true) => out.unexpected_passes.push(rel),
            (false, true) => out.expected_failures += 1,
            (false, false) => out.unexpected_failures.push(rel),
        }
    }
    Ok(out)
}

fn main() -> ExitCode {
    let outcome = match run() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    println!(
        "corpus: {} files | ok={} expected_failures={} unexpected_failures={} unexpected_passes={}",
        outcome.total,
        outcome.ok,
        outcome.expected_failures,
        outcome.unexpected_failures.len(),
        outcome.unexpected_passes.len(),
    );

    if !outcome.unexpected_failures.is_empty() {
        println!(
            "\nUnexpected parse failures ({}):",
            outcome.unexpected_failures.len()
        );
        for f in &outcome.unexpected_failures {
            println!("  - {f}");
        }
    }
    if !outcome.unexpected_passes.is_empty() {
        println!(
            "\nFiles that unexpectedly parsed (remove from KNOWN_BAD): {}",
            outcome.unexpected_passes.len()
        );
        for f in &outcome.unexpected_passes {
            println!("  - {f}");
        }
    }

    if outcome.unexpected_failures.is_empty() && outcome.unexpected_passes.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
