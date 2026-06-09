use std::collections::HashMap;
use std::ops::ControlFlow;

use async_lsp::ClientSocket;
use async_lsp::lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, DocumentDiagnosticReport,
    DocumentDiagnosticReportResult, FullDocumentDiagnosticReport, InitializeResult, OneOf,
    Position, PublishDiagnosticsParams, Range, RelatedFullDocumentDiagnosticReport,
    ServerCapabilities, ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit,
    Url, notification, request,
};
use async_lsp::router::Router;
use async_lsp::server::LifecycleLayer;
use scarpet_fmt::{Config, format_source};
use scarpet_syntax::parser::{ParseError, parse_source};
use tower::ServiceBuilder;

/// Run the Scarpet language server on stdin/stdout.
///
/// This function owns its Tokio runtime so synchronous callers, including
/// `scarpet-cli`, can start the language server without duplicating setup code.
pub fn run_stdio() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(run_stdio_async())?;
    Ok(())
}

async fn run_stdio_async() -> async_lsp::Result<()> {
    let (server, _) = async_lsp::MainLoop::new_server(|client| {
        let mut router = Router::new(ServerState::new(client.clone()));
        router
            .request::<request::Initialize, _>(|_, _| async move {
                Ok(InitializeResult {
                    capabilities: ServerCapabilities {
                        text_document_sync: Some(TextDocumentSyncCapability::Kind(
                            TextDocumentSyncKind::FULL,
                        )),
                        document_formatting_provider: Some(OneOf::Left(true)),
                        diagnostic_provider: Some(
                            async_lsp::lsp_types::DiagnosticServerCapabilities::Options(
                                async_lsp::lsp_types::DiagnosticOptions {
                                    identifier: Some("scarpet".to_string()),
                                    inter_file_dependencies: false,
                                    workspace_diagnostics: false,
                                    ..Default::default()
                                },
                            ),
                        ),
                        ..ServerCapabilities::default()
                    },
                    server_info: Some(ServerInfo {
                        name: "scarpet-lsp".to_string(),
                        version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    }),
                })
            })
            .request::<request::Formatting, _>(|state, params| {
                let text = state.documents.get(&params.text_document.uri).cloned();
                async move { Ok(format_document(text)) }
            })
            .request::<request::DocumentDiagnosticRequest, _>(|state, params| {
                let uri = params.text_document.uri;
                let text = state.documents.get(&uri).cloned();
                async move {
                    Ok(DocumentDiagnosticReportResult::Report(
                        DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                            related_documents: None,
                            full_document_diagnostic_report: FullDocumentDiagnosticReport {
                                result_id: None,
                                items: text.as_deref().map(diagnostics_for).unwrap_or_default(),
                            },
                        }),
                    ))
                }
            })
            .notification::<notification::Initialized>(|_, _| ControlFlow::Continue(()))
            .notification::<notification::DidOpenTextDocument>(|state, params| {
                state.did_open(params)
            })
            .notification::<notification::DidChangeTextDocument>(|state, params| {
                state.did_change(params)
            })
            .notification::<notification::DidSaveTextDocument>(|state, params| {
                state.did_save(params)
            })
            .notification::<notification::DidCloseTextDocument>(|state, params| {
                state.did_close(params)
            });

        ServiceBuilder::new()
            .layer(LifecycleLayer::default())
            .service(router)
    });

    let (stdin, stdout) = (
        async_lsp::stdio::PipeStdin::lock_tokio()?,
        async_lsp::stdio::PipeStdout::lock_tokio()?,
    );
    server.run_buffered(stdin, stdout).await
}

struct ServerState {
    client: ClientSocket,
    documents: HashMap<Url, String>,
}

impl ServerState {
    fn new(client: ClientSocket) -> Self {
        Self {
            client,
            documents: HashMap::new(),
        }
    }

    fn did_open(
        &mut self,
        params: DidOpenTextDocumentParams,
    ) -> ControlFlow<async_lsp::Result<()>> {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        self.documents.insert(uri.clone(), text);
        self.publish_diagnostics(uri);
        ControlFlow::Continue(())
    }

    fn did_change(
        &mut self,
        params: DidChangeTextDocumentParams,
    ) -> ControlFlow<async_lsp::Result<()>> {
        let uri = params.text_document.uri;
        if let Some(change) = params.content_changes.into_iter().next_back() {
            self.documents.insert(uri.clone(), change.text);
            self.publish_diagnostics(uri);
        }
        ControlFlow::Continue(())
    }

    fn did_save(
        &mut self,
        params: DidSaveTextDocumentParams,
    ) -> ControlFlow<async_lsp::Result<()>> {
        self.publish_diagnostics(params.text_document.uri);
        ControlFlow::Continue(())
    }

    fn did_close(
        &mut self,
        params: DidCloseTextDocumentParams,
    ) -> ControlFlow<async_lsp::Result<()>> {
        let uri = params.text_document.uri;
        self.documents.remove(&uri);
        self.client
            .notify::<notification::PublishDiagnostics>(PublishDiagnosticsParams {
                uri,
                diagnostics: Vec::new(),
                version: None,
            })
            .ok();
        ControlFlow::Continue(())
    }

    fn publish_diagnostics(&mut self, uri: Url) {
        let diagnostics = self
            .documents
            .get(&uri)
            .map(|text| diagnostics_for(text))
            .unwrap_or_default();
        self.client
            .notify::<notification::PublishDiagnostics>(PublishDiagnosticsParams {
                uri,
                diagnostics,
                version: None,
            })
            .ok();
    }
}

fn format_document(text: Option<String>) -> Option<Vec<TextEdit>> {
    let text = text?;
    let formatted = format_source(&text, &Config::default()).ok()?;
    if formatted == text {
        return Some(Vec::new());
    }
    Some(vec![TextEdit {
        range: full_document_range(&text),
        new_text: formatted,
    }])
}

fn diagnostics_for(text: &str) -> Vec<Diagnostic> {
    match parse_source(text) {
        Ok(_) => Vec::new(),
        Err(e) => vec![parse_diagnostic(text, &e)],
    }
}

fn parse_diagnostic(text: &str, error: &ParseError) -> Diagnostic {
    let mut message = error.message();
    if let Some(help) = &error.help {
        message.push_str("\nhelp: ");
        message.push_str(help);
    }
    Diagnostic {
        range: byte_range_to_lsp_range(text, error.span.clone()),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("scarpet".to_string()),
        message,
        related_information: error.secondary.as_ref().map(|(span, label)| {
            vec![async_lsp::lsp_types::DiagnosticRelatedInformation {
                location: async_lsp::lsp_types::Location {
                    uri: Url::parse("file:///<scarpet>").expect("static URI is valid"),
                    range: byte_range_to_lsp_range(text, span.clone()),
                },
                message: label.clone(),
            }]
        }),
        ..Diagnostic::default()
    }
}

fn full_document_range(text: &str) -> Range {
    Range {
        start: Position::new(0, 0),
        end: offset_to_position(text, text.len()),
    }
}

fn byte_range_to_lsp_range(text: &str, range: std::ops::Range<usize>) -> Range {
    let start = offset_to_position(text, range.start);
    let mut end = offset_to_position(text, range.end);
    if start == end {
        end.character += 1;
    }
    Range { start, end }
}

fn offset_to_position(text: &str, offset: usize) -> Position {
    let offset = offset.min(text.len());
    let mut line = 0;
    let mut line_start = 0;
    for (idx, ch) in text.char_indices() {
        if idx >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            line_start = idx + ch.len_utf8();
        }
    }
    let character = text[line_start..offset]
        .encode_utf16()
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    Position::new(line, character)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_errors_become_diagnostics() {
        let diagnostics = diagnostics_for("(");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn offsets_are_utf16_positions() {
        assert_eq!(
            "a\né".find('é').map(|idx| offset_to_position("a\né", idx)),
            Some(Position::new(1, 0))
        );
        assert_eq!(
            offset_to_position("a\n🙂b", "a\n🙂".len()),
            Position::new(1, 2)
        );
    }

    #[test]
    fn formatting_replaces_whole_document() {
        let edits = format_document(Some("foo(1,2)".to_string())).unwrap();
        assert_eq!(edits.len(), 1);
        assert!(edits[0].new_text.ends_with('\n'));
    }
}
