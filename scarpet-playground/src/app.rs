//! The root component: a header that toggles between the two-pane editor and the
//! notebook, the shared formatter-options bar, and the active view's body. Both
//! views share one [`Config`]; the notebook also owns a persistent [`Session`].

use scarpet_fmt::{BraceStyle, Config, LineEnding};
use yew::prelude::*;

use crate::editor::{Mode, SAMPLE};
use crate::notebook::Notebook;
use crate::session::{CellOutput, Session};
use crate::shared::{BTN_BASE, BTN_BORDERED, BTN_INK};

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
    /// A numeric control fired with an unparseable value; keep the current config.
    Noop,
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

    /// The header: brand, the Editor/Notebook toggle, and the active view's
    /// actions on the right.
    fn view_header(&self, ctx: &Context<Self>) -> Html {
        let link = ctx.link();
        let on_editor = link.callback(|_| Msg::SwitchView(View::Editor));
        let on_notebook = link.callback(|_| Msg::SwitchView(View::Notebook));

        let editor_cls = if self.view == View::Editor {
            classes!(BTN_BASE, BTN_INK)
        } else {
            classes!(BTN_BASE, BTN_BORDERED)
        };
        let notebook_cls = if self.view == View::Notebook {
            classes!(BTN_BASE, BTN_INK)
        } else {
            classes!(BTN_BASE, BTN_BORDERED)
        };

        let actions = match self.view {
            View::Editor => self.view_editor_actions(ctx),
            View::Notebook => self.view_notebook_actions(ctx),
        };

        html! {
            <header class="flex h-16 shrink-0 items-center justify-between border-b border-hairline bg-canvas px-6">
                <div class="flex items-center gap-4">
                    <div class="flex items-baseline gap-2">
                        <span class="text-base font-semibold tracking-tight">{ "scarpet" }</span>
                        <span class="text-sm text-mute">{ "playground" }</span>
                    </div>
                    <div class="flex items-center gap-1">
                        <button onclick={on_editor} class={editor_cls}>{ "Editor" }</button>
                        <button onclick={on_notebook} class={notebook_cls}>{ "Notebook" }</button>
                    </div>
                </div>
                <div class="flex items-center gap-2">{ actions }</div>
            </header>
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
                self.config.indent_width = w;
                self.reformat_if_showing();
                true
            }
            Msg::SetMaxWidth(w) => {
                self.config.max_width = w;
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
            Msg::Noop => false,
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
        html! {
            <div class="flex h-screen flex-col bg-canvas-soft text-ink">
                { self.view_header(ctx) }
                { self.view_options(ctx) }
                { match self.view {
                    View::Editor => self.view_editor(ctx),
                    View::Notebook => self.view_notebook(ctx),
                } }
            </div>
        }
    }
}
