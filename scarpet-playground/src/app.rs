//! The root component: it owns all state and composes the presentational
//! components — [`Header`], [`OptionsBar`], the editor's
//! [`EditorActions`]/[`EditorView`], and the notebook's
//! [`NotebookActions`]/[`NotebookView`] — wiring each to a [`Msg`] via callbacks.
//! Both views share one [`Config`]; the notebook also owns a persistent
//! [`Session`].

use scarpet_fmt::{BraceStyle, Config, LineEnding};
use yew::prelude::*;

use crate::editor::{EditorActions, EditorView, Mode, SAMPLE};
use crate::header::Header;
use crate::notebook::{Notebook, NotebookActions, NotebookView};
use crate::options::OptionsBar;
use crate::session::{CellOutput, Session};

/// Which screen is showing.
#[derive(Clone, Copy, PartialEq)]
pub enum View {
    /// The original two-pane editor (Format / Syntax tree / AST / Run).
    Editor,
    /// The notebook: a column of cells over a persistent kernel.
    Notebook,
}

pub struct App {
    pub(crate) view: View,
    pub(crate) input: String,
    pub(crate) output: String,
    /// Human-readable editor diagnostics: a parse error's headline plus an
    /// optional `help:` line, or a lowering error's message.
    pub(crate) diagnostics: Vec<String>,
    /// Heading shown above [`diagnostics`](Self::diagnostics).
    pub(crate) diagnostics_title: &'static str,
    /// The tool that produced the editor `output`, once one has run.
    pub(crate) mode: Option<Mode>,
    /// The formatting style, edited from the options bar and shared by both
    /// views' Format.
    pub(crate) config: Config,
    pub(crate) notebook: Notebook,
    pub(crate) session: Session,
}

pub enum Msg {
    /// The editor input textarea changed.
    Input(String),
    /// Run an editor tool over the current input.
    Run(Mode),
    /// Set the formatter's indentation step, in spaces.
    SetIndentWidth(usize),
    /// Set the formatter's target line width.
    SetMaxWidth(usize),
    /// Set the comment-wrap width; `None` leaves comments unwrapped.
    SetCommentWidth(Option<usize>),
    /// Set the line ending the formatter emits for inserted breaks.
    SetLineEnding(LineEnding),
    /// Set where an opening delimiter sits on a broken block.
    SetBraceStyle(BraceStyle),
    /// Switch between the editor and the notebook.
    SwitchView(View),
    /// A cell's textarea changed (tracked without re-rendering).
    NbEditCell {
        id: u64,
        source: String,
    },
    /// Run a single cell in the persistent kernel.
    NbRunCell(u64),
    /// Run every cell top-to-bottom, stopping at the first error.
    NbRunAll,
    /// Append a new empty cell.
    NbAddCell,
    /// Delete a cell.
    NbDeleteCell(u64),
    /// Move a cell one position earlier / later.
    NbMoveUp(u64),
    NbMoveDown(u64),
    /// Format a single cell's source.
    NbFormatCell(u64),
    /// Reset the kernel: drop all variables and definitions, clear cell outputs.
    NbRestart,
}

impl App {
    /// Run one cell in the persistent kernel and record its result and badge.
    /// Blank cells are skipped (and do not advance the execution counter).
    fn run_cell(&mut self, id: u64) {
        let Some(src) = self.notebook.source_of(id) else {
            return;
        };
        if src.trim().is_empty() {
            return;
        }
        let out = self.session.run(&src);
        let exec = self.session.exec_counter;
        self.notebook.set_result(id, out, exec);
    }

    /// Run every cell in order, stopping at the first that errors (so a failed
    /// definition does not cascade). Earlier cells keep their results.
    fn run_all(&mut self) {
        for id in self.notebook.cell_ids() {
            let Some(src) = self.notebook.source_of(id) else {
                continue;
            };
            if src.trim().is_empty() {
                continue;
            }
            let out = self.session.run(&src);
            let exec = self.session.exec_counter;
            let errored = matches!(out, CellOutput::Error { .. });
            self.notebook.set_result(id, out, exec);
            if errored {
                break;
            }
        }
    }
}

impl Component for App {
    type Message = Msg;
    type Properties = ();

    fn create(_ctx: &Context<Self>) -> Self {
        let mut app = App {
            view: View::Editor,
            input: SAMPLE.to_string(),
            output: String::new(),
            diagnostics: Vec::new(),
            diagnostics_title: "Parse error",
            mode: None,
            config: Config::default(),
            notebook: Notebook::empty(),
            session: Session::new(),
        };
        app.run(Mode::Format);
        app
    }

    fn update(&mut self, _ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            // Track the value without re-rendering: the textarea DOM already
            // holds it, so re-rendering here would be wasted work.
            Msg::Input(value) => {
                self.input = value;
                false
            }
            Msg::Run(mode) => {
                self.run(mode);
                true
            }
            Msg::SetIndentWidth(w) => {
                self.config.indent_width = w.clamp(1, 16);
                self.reformat_if_showing();
                true
            }
            Msg::SetMaxWidth(w) => {
                self.config.max_width = w.max(1);
                self.reformat_if_showing();
                true
            }
            Msg::SetCommentWidth(w) => {
                self.config.comment_width = w;
                self.reformat_if_showing();
                true
            }
            Msg::SetLineEnding(le) => {
                self.config.line_ending = le;
                self.reformat_if_showing();
                true
            }
            Msg::SetBraceStyle(bs) => {
                self.config.brace_style = bs;
                self.reformat_if_showing();
                true
            }
            Msg::SwitchView(view) => {
                self.view = view;
                if view == View::Notebook {
                    self.notebook.ensure_sample();
                }
                true
            }
            // Same rationale as `Msg::Input`: the textarea holds the text.
            Msg::NbEditCell { id, source } => {
                self.notebook.edit(id, source);
                false
            }
            Msg::NbRunCell(id) => {
                self.run_cell(id);
                true
            }
            Msg::NbRunAll => {
                self.run_all();
                true
            }
            Msg::NbAddCell => {
                self.notebook.add_cell();
                true
            }
            Msg::NbDeleteCell(id) => {
                self.notebook.delete(id);
                true
            }
            Msg::NbMoveUp(id) => {
                self.notebook.move_up(id);
                true
            }
            Msg::NbMoveDown(id) => {
                self.notebook.move_down(id);
                true
            }
            Msg::NbFormatCell(id) => {
                self.notebook.format_cell(id, &self.config);
                true
            }
            Msg::NbRestart => {
                self.session = Session::new();
                self.notebook.reset_outputs();
                true
            }
        }
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let link = ctx.link();

        // The active view's header actions and body.
        let actions = match self.view {
            View::Editor => html! { <EditorActions on_run={link.callback(Msg::Run)} /> },
            View::Notebook => html! {
                <NotebookActions
                    on_add={link.callback(|_: ()| Msg::NbAddCell)}
                    on_run_all={link.callback(|_: ()| Msg::NbRunAll)}
                    on_restart={link.callback(|_: ()| Msg::NbRestart)}
                />
            },
        };
        let body = match self.view {
            View::Editor => html! {
                <EditorView
                    input={self.input.clone()}
                    output={self.output.clone()}
                    diagnostics={self.diagnostics.clone()}
                    diagnostics_title={self.diagnostics_title}
                    mode={self.mode}
                    on_input={link.callback(Msg::Input)}
                />
            },
            View::Notebook => html! {
                <NotebookView
                    cells={self.notebook.snapshot()}
                    on_add={link.callback(|_: ()| Msg::NbAddCell)}
                    on_edit={link.callback(|(id, source)| Msg::NbEditCell { id, source })}
                    on_run={link.callback(Msg::NbRunCell)}
                    on_format={link.callback(Msg::NbFormatCell)}
                    on_delete={link.callback(Msg::NbDeleteCell)}
                    on_move_up={link.callback(Msg::NbMoveUp)}
                    on_move_down={link.callback(Msg::NbMoveDown)}
                />
            },
        };

        html! {
            <div class="flex h-screen flex-col bg-canvas-soft text-ink">
                <Header view={self.view} on_switch={link.callback(Msg::SwitchView)}>
                    { actions }
                </Header>
                <OptionsBar
                    config={self.config}
                    on_indent={link.callback(Msg::SetIndentWidth)}
                    on_max={link.callback(Msg::SetMaxWidth)}
                    on_comment={link.callback(Msg::SetCommentWidth)}
                    on_line_ending={link.callback(Msg::SetLineEnding)}
                    on_brace={link.callback(Msg::SetBraceStyle)}
                />
                { body }
            </div>
        }
    }
}
