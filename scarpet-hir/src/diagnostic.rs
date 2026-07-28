//! Located findings, in one shape every front end can adapt.
//!
//! `scarpet-lsp`'s `parse_diagnostic` and `scarpet-playground`'s
//! `diagnostics_for` are today two independent renderings of the same
//! [`ParseError`]. [`Diagnostic`] exists so a third one is not written: analysis
//! results and parse errors arrive as the same type, with a severity, a stable
//! machine-readable [`LintCode`], and byte spans that convert straight into
//! `std::ops::Range<usize>` — what `ariadne` reports and the language server's
//! `byte_range_to_lsp_range` already take.

use std::fmt;

use scarpet_syntax::parser::ParseError;

use crate::span::Span;

/// How much a finding matters.
///
/// Most of what a Scarpet checker can say is a [`Warning`](Severity::Warning):
/// the language is close to total, so a suspicious expression usually still has
/// a defined — if surprising — meaning.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl Severity {
    /// The lowercase name, used as a diagnostic prefix and as a CSS hook.
    pub const fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which check produced a finding.
///
/// Stable across releases so a caller can filter, deny, or suppress by name —
/// `scarpet-cli` already has a `--deny` flag shaped for exactly this.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LintCode {
    /// A syntax error, forwarded from the parser.
    Parse,
    /// A call whose argument count cannot satisfy the callee's signature.
    WrongArgCount,
    /// A call to a name that is neither defined in the program nor a documented
    /// builtin.
    UnknownFunction,
    /// An operator that coerces its operand with `as_number` was given
    /// something that provably is not a number.
    ExpectedNumber,
    /// A bare map entry that cannot be a key/value pair.
    MapEntryNotPair,
    /// An arithmetic operator that will silently concatenate as text instead.
    StringFallback,
    /// A variable read before anything writes it.
    ReadBeforeWrite,
}

impl LintCode {
    /// The kebab-case name, for `--deny` flags and LSP `code` fields.
    pub const fn as_str(self) -> &'static str {
        match self {
            LintCode::Parse => "parse",
            LintCode::WrongArgCount => "wrong-arg-count",
            LintCode::UnknownFunction => "unknown-function",
            LintCode::ExpectedNumber => "expected-number",
            LintCode::MapEntryNotPair => "map-entry-not-pair",
            LintCode::StringFallback => "string-fallback",
            LintCode::ReadBeforeWrite => "read-before-write",
        }
    }
}

impl fmt::Display for LintCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A secondary location that explains a finding — where a function was defined,
/// which bracket was left open.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Related {
    pub span: Span,
    pub message: String,
}

/// One finding about a program.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub span: Span,
    pub severity: Severity,
    pub code: LintCode,
    pub message: String,
    /// A suggested fix, rendered as `help:` by every front end.
    pub help: Option<String>,
    pub related: Vec<Related>,
}

impl Diagnostic {
    /// A finding with no help text and no related locations.
    pub fn new(
        span: Span,
        severity: Severity,
        code: LintCode,
        message: impl Into<String>,
    ) -> Diagnostic {
        Diagnostic {
            span,
            severity,
            code,
            message: message.into(),
            help: None,
            related: Vec::new(),
        }
    }

    pub fn error(span: Span, code: LintCode, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(span, Severity::Error, code, message)
    }

    pub fn warning(span: Span, code: LintCode, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(span, Severity::Warning, code, message)
    }

    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Diagnostic {
        self.help = Some(help.into());
        self
    }

    #[must_use]
    pub fn with_related(mut self, span: Span, message: impl Into<String>) -> Diagnostic {
        self.related.push(Related {
            span,
            message: message.into(),
        });
        self
    }

    /// Adapt a parse error, keeping its caret span, its rustc-style headline,
    /// its `help:` line, and its secondary label.
    pub fn from_parse_error(error: &ParseError) -> Diagnostic {
        let mut diagnostic = Diagnostic::error(
            Span::new(error.span.start as u32, error.span.end as u32),
            LintCode::Parse,
            error.message(),
        );
        diagnostic.help = error.help.clone();
        if let Some((span, label)) = &error.secondary {
            diagnostic = diagnostic
                .with_related(Span::new(span.start as u32, span.end as u32), label.clone());
        }
        diagnostic
    }

    /// `error: message`, the one-line form a plain-text front end shows.
    pub fn headline(&self) -> String {
        format!("{}: {}", self.severity, self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scarpet_syntax::parser::parse;

    #[test]
    fn a_parse_error_keeps_its_span_and_help() {
        let error = parse("[1, 2").expect_err("unclosed bracket");
        let diagnostic = Diagnostic::from_parse_error(&error);
        assert_eq!(diagnostic.severity, Severity::Error);
        assert_eq!(diagnostic.code, LintCode::Parse);
        assert_eq!(
            Into::<std::ops::Range<usize>>::into(diagnostic.span),
            error.span
        );
        // The unclosed `[` is reported as a secondary location.
        assert_eq!(diagnostic.related.len(), error.secondary.iter().count());
    }

    #[test]
    fn codes_are_kebab_case() {
        assert_eq!(LintCode::StringFallback.as_str(), "string-fallback");
        assert_eq!(LintCode::WrongArgCount.to_string(), "wrong-arg-count");
    }
}
