//! The notebook view: a column of cells run against a persistent kernel, so a
//! binding made in one cell is visible to the next.
//!
//! [`Notebook`] is the model (cells, id allocator, sample loading); the rendering
//! is three presentational components — [`NotebookActions`] (header buttons),
//! [`NotebookView`] (the column), and [`CellView`] (one cell) — that reach
//! [`App`](crate::app::App) only through callbacks.

use scarpet_fmt::Config;
use web_sys::HtmlTextAreaElement;
use yew::prelude::*;

use crate::session::CellOutput;
use crate::shared::{BTN_BASE, BTN_BORDERED, BTN_INK, BTN_LINK, BTN_SM, diagnostics_for};

/// The cell editor textarea: fills the row beside the badge gutter, vertically
/// resizable, monospace.
const CELL_EDITOR: &str =
    "min-w-0 flex-1 resize-y bg-canvas p-3 font-mono text-[13px] leading-5 text-ink outline-none";
/// The captured `print` output block beneath a cell.
const CELL_OUT: &str =
    "overflow-auto whitespace-pre bg-canvas px-3 py-2 font-mono text-[13px] leading-5 text-ink";

/// A starter notebook loaded the first time the Notebook view is opened. It
/// exercises cross-cell variable use (cell 4 reads cell 1), cross-cell function
/// use (cell 3 calls cell 2), `print` output, and a bare-expression result.
const NB_SAMPLE: &[&str] = &[
    "a = 5;",
    "fib(n) -> if(n < 2, n, fib(n-1)+fib(n-2));",
    "print('fib(10) = ' + fib(10));\nfib(10)",
    "a * 2",
];

/// One notebook cell: its source, the result of its last run, and the execution
/// number to badge it with. `Clone`/`PartialEq` let a snapshot ride in
/// [`NotebookView`]/[`CellView`] props and skip re-rendering when unchanged.
#[derive(Clone, PartialEq)]
pub struct Cell {
    /// Stable identity for the list `key`, so reorder/delete never recreates a
    /// textarea (preserving its DOM value and focus). Never reused.
    id: u64,
    source: String,
    output: CellOutput,
    /// The `[n]` badge — `None` until the cell has run.
    exec: Option<u32>,
}

/// The notebook's cells plus the id allocator and a one-shot "loaded the sample"
/// flag.
pub struct Notebook {
    cells: Vec<Cell>,
    next_id: u64,
    loaded: bool,
}

impl Notebook {
    /// An empty notebook (the sample is loaded lazily on first view).
    pub(crate) fn empty() -> Self {
        Notebook {
            cells: Vec::new(),
            next_id: 0,
            loaded: false,
        }
    }

    /// A clone of the cells, to hand to [`NotebookView`] as props.
    pub(crate) fn snapshot(&self) -> Vec<Cell> {
        self.cells.clone()
    }

    /// Append a new cell holding `source`, assigning it a fresh id.
    fn push_cell(&mut self, source: String) {
        let id = self.next_id;
        self.next_id += 1;
        self.cells.push(Cell {
            id,
            source,
            output: CellOutput::NotRun,
            exec: None,
        });
    }

    /// On the first call, fill the notebook with the starter sample; later calls
    /// do nothing (so reopening the view does not clobber the user's cells).
    pub(crate) fn ensure_sample(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        for src in NB_SAMPLE {
            self.push_cell((*src).to_owned());
        }
    }

    /// Append a fresh, empty cell.
    pub(crate) fn add_cell(&mut self) {
        self.push_cell(String::new());
    }

    /// Remove the cell with `id`, if present.
    pub(crate) fn delete(&mut self, id: u64) {
        self.cells.retain(|c| c.id != id);
    }

    /// Swap the cell with `id` one position earlier.
    pub(crate) fn move_up(&mut self, id: u64) {
        if let Some(i) = self.cells.iter().position(|c| c.id == id)
            && i > 0
        {
            self.cells.swap(i, i - 1);
        }
    }

    /// Swap the cell with `id` one position later.
    pub(crate) fn move_down(&mut self, id: u64) {
        if let Some(i) = self.cells.iter().position(|c| c.id == id)
            && i + 1 < self.cells.len()
        {
            self.cells.swap(i, i + 1);
        }
    }

    /// Update a cell's source after a textarea edit.
    pub(crate) fn edit(&mut self, id: u64, source: String) {
        if let Some(cell) = self.cells.iter_mut().find(|c| c.id == id) {
            cell.source = source;
        }
    }

    /// The ids of every cell, in order — for "Run all".
    pub(crate) fn cell_ids(&self) -> Vec<u64> {
        self.cells.iter().map(|c| c.id).collect()
    }

    /// A copy of a cell's current source, if the cell exists.
    pub(crate) fn source_of(&self, id: u64) -> Option<String> {
        self.cells
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.source.clone())
    }

    /// Store a run's result and execution number on the cell.
    pub(crate) fn set_result(&mut self, id: u64, output: CellOutput, exec: u32) {
        if let Some(cell) = self.cells.iter_mut().find(|c| c.id == id) {
            cell.output = output;
            cell.exec = Some(exec);
        }
    }

    /// Clear every cell's output and badge — used on restart, keeping the cells'
    /// source and order.
    pub(crate) fn reset_outputs(&mut self) {
        for cell in &mut self.cells {
            cell.output = CellOutput::NotRun;
            cell.exec = None;
        }
    }

    /// Format a single cell's source with the shared config; on a parse error,
    /// surface it in that cell's output area without touching its text.
    pub(crate) fn format_cell(&mut self, id: u64, config: &Config) {
        if let Some(cell) = self.cells.iter_mut().find(|c| c.id == id) {
            match scarpet_fmt::format_source(&cell.source, config) {
                Ok(formatted) => cell.source = formatted,
                Err(scarpet_fmt::FmtError::Parse(err)) => {
                    cell.output = CellOutput::Error {
                        title: "Parse error",
                        printed: String::new(),
                        lines: diagnostics_for(&err),
                    };
                }
            }
        }
    }
}

#[derive(Properties, PartialEq)]
pub struct NotebookActionsProps {
    pub on_add: Callback<()>,
    pub on_run_all: Callback<()>,
    pub on_restart: Callback<()>,
}

/// The notebook's header buttons (Add cell / Restart / Run all).
pub struct NotebookActions;

impl Component for NotebookActions {
    type Message = ();
    type Properties = NotebookActionsProps;

    fn create(_ctx: &Context<Self>) -> Self {
        Self
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let props = ctx.props();
        let on_add = {
            let cb = props.on_add.clone();
            Callback::from(move |_: web_sys::MouseEvent| cb.emit(()))
        };
        let on_restart = {
            let cb = props.on_restart.clone();
            Callback::from(move |_: web_sys::MouseEvent| cb.emit(()))
        };
        let on_run_all = {
            let cb = props.on_run_all.clone();
            Callback::from(move |_: web_sys::MouseEvent| cb.emit(()))
        };
        html! {
            <button onclick={on_add} class={classes!(BTN_BASE, BTN_BORDERED)}>{ "Add cell" }</button>
            <button onclick={on_restart} class={classes!(BTN_BASE, BTN_BORDERED)}>{ "Restart" }</button>
            <button onclick={on_run_all} class={classes!(BTN_BASE, BTN_LINK)}>{ "Run all" }</button>
        }
    }
}

#[derive(Properties, PartialEq)]
pub struct NotebookViewProps {
    pub cells: Vec<Cell>,
    pub on_add: Callback<()>,
    pub on_edit: Callback<(u64, String)>,
    pub on_run: Callback<u64>,
    pub on_format: Callback<u64>,
    pub on_delete: Callback<u64>,
    pub on_move_up: Callback<u64>,
    pub on_move_down: Callback<u64>,
}

/// The scrollable column of cells, with a trailing "Add cell" button.
pub struct NotebookView;

impl Component for NotebookView {
    type Message = ();
    type Properties = NotebookViewProps;

    fn create(_ctx: &Context<Self>) -> Self {
        Self
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let props = ctx.props();
        let on_add = {
            let cb = props.on_add.clone();
            Callback::from(move |_: web_sys::MouseEvent| cb.emit(()))
        };
        html! {
            <main class="min-h-0 flex-1 overflow-auto bg-canvas-soft p-6">
                <div class="mx-auto flex max-w-4xl flex-col gap-4">
                    { for props.cells.iter().map(|cell| html! {
                        <CellView
                            key={cell.id.to_string()}
                            id={cell.id}
                            source={cell.source.clone()}
                            output={cell.output.clone()}
                            exec={cell.exec}
                            on_edit={props.on_edit.clone()}
                            on_run={props.on_run.clone()}
                            on_format={props.on_format.clone()}
                            on_delete={props.on_delete.clone()}
                            on_move_up={props.on_move_up.clone()}
                            on_move_down={props.on_move_down.clone()}
                        />
                    }) }
                    <div>
                        <button onclick={on_add} class={classes!(BTN_SM, BTN_BORDERED)}>{ "+ Add cell" }</button>
                    </div>
                </div>
            </main>
        }
    }
}

#[derive(Properties, PartialEq)]
pub struct CellViewProps {
    pub id: u64,
    pub source: AttrValue,
    pub output: CellOutput,
    pub exec: Option<u32>,
    pub on_edit: Callback<(u64, String)>,
    pub on_run: Callback<u64>,
    pub on_format: Callback<u64>,
    pub on_delete: Callback<u64>,
    pub on_move_up: Callback<u64>,
    pub on_move_down: Callback<u64>,
}

/// One cell: badge gutter, editor textarea, per-cell controls, and its output.
pub struct CellView;

impl Component for CellView {
    type Message = ();
    type Properties = CellViewProps;

    fn create(_ctx: &Context<Self>) -> Self {
        Self
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let props = ctx.props();
        let id = props.id;

        let oninput = {
            let cb = props.on_edit.clone();
            Callback::from(move |e: web_sys::InputEvent| {
                let textarea: HtmlTextAreaElement = e.target_unchecked_into();
                cb.emit((id, textarea.value()));
            })
        };
        // Shift+Enter runs the cell instead of inserting a newline; other keys fall
        // through to the textarea untouched.
        let onkeydown = {
            let cb = props.on_run.clone();
            Callback::from(move |e: web_sys::KeyboardEvent| {
                if e.key() == "Enter" && e.shift_key() {
                    e.prevent_default();
                    cb.emit(id);
                }
            })
        };
        let on_run = {
            let cb = props.on_run.clone();
            Callback::from(move |_: web_sys::MouseEvent| cb.emit(id))
        };
        let on_fmt = {
            let cb = props.on_format.clone();
            Callback::from(move |_: web_sys::MouseEvent| cb.emit(id))
        };
        let on_del = {
            let cb = props.on_delete.clone();
            Callback::from(move |_: web_sys::MouseEvent| cb.emit(id))
        };
        let on_up = {
            let cb = props.on_move_up.clone();
            Callback::from(move |_: web_sys::MouseEvent| cb.emit(id))
        };
        let on_down = {
            let cb = props.on_move_down.clone();
            Callback::from(move |_: web_sys::MouseEvent| cb.emit(id))
        };

        let badge = match props.exec {
            Some(n) => format!("[{n}]"),
            None => "[ ]".to_owned(),
        };
        let rows = props.source.lines().count().clamp(3, 24);

        html! {
            <div class="overflow-hidden rounded-md border border-hairline bg-canvas">
                <div class="flex items-stretch">
                    <div class="flex w-12 shrink-0 select-none items-start justify-center border-r border-hairline py-3 font-mono text-xs text-mute">
                        { badge }
                    </div>
                    <textarea
                        class={CELL_EDITOR}
                        rows={rows.to_string()}
                        spellcheck="false"
                        placeholder="Scarpet…"
                        value={props.source.clone()}
                        oninput={oninput}
                        onkeydown={onkeydown}
                    />
                </div>
                <div class="flex items-center gap-2 border-t border-hairline bg-canvas-soft px-3 py-2">
                    <button onclick={on_run} class={classes!(BTN_SM, BTN_LINK)}>{ "Run" }</button>
                    <button onclick={on_fmt} class={classes!(BTN_SM, BTN_INK)}>{ "Format" }</button>
                    <div class="flex-1" />
                    <button onclick={on_up} class={classes!(BTN_SM, BTN_BORDERED)} title="Move up">{ "↑" }</button>
                    <button onclick={on_down} class={classes!(BTN_SM, BTN_BORDERED)} title="Move down">{ "↓" }</button>
                    <button onclick={on_del} class={classes!(BTN_SM, BTN_BORDERED)}>{ "Delete" }</button>
                </div>
                { cell_output_view(&props.output) }
            </div>
        }
    }
}

/// The output area beneath a cell: `print` text, the result value, an error
/// strip, or a faint "(no output)" — separated by hairlines via `divide-y`.
fn cell_output_view(output: &CellOutput) -> Html {
    match output {
        CellOutput::NotRun => html! {},
        CellOutput::Ok { printed, value } => {
            let mut rows: Vec<Html> = Vec::new();
            if !printed.is_empty() {
                rows.push(html! { <pre class={CELL_OUT}>{ printed }</pre> });
            }
            if value != "null" {
                rows.push(html! {
                    <div class="flex gap-2 px-3 py-2 font-mono text-[13px]">
                        <span class="select-none text-mute">{ "=>" }</span>
                        <span class="whitespace-pre-wrap text-ink">{ value }</span>
                    </div>
                });
            }
            if rows.is_empty() {
                rows.push(html! {
                    <div class="px-3 py-2 font-mono text-xs text-mute">{ "(no output)" }</div>
                });
            }
            html! {
                <div class="divide-y divide-hairline border-t border-hairline">
                    { for rows.into_iter() }
                </div>
            }
        }
        CellOutput::Error {
            title,
            printed,
            lines,
        } => {
            let mut rows: Vec<Html> = Vec::new();
            if !printed.is_empty() {
                rows.push(html! { <pre class={CELL_OUT}>{ printed }</pre> });
            }
            rows.push(html! {
                <div class="bg-canvas px-3 py-2 font-mono text-xs text-error">
                    <div class="pb-1 font-medium">{ *title }</div>
                    { for lines.iter().map(|d| html! { <div class="py-0.5">{ d }</div> }) }
                </div>
            });
            html! {
                <div class="divide-y divide-hairline border-t border-hairline">
                    { for rows.into_iter() }
                </div>
            }
        }
    }
}
