//! The notebook's presentational components: [`NotebookActions`] (header
//! buttons), [`NotebookView`] (the scrollable column), and [`CellView`] (one
//! cell). All reach [`App`](crate::app::App) only through callbacks; the model
//! lives in the parent [`notebook`](super) module.

use web_sys::HtmlTextAreaElement;
use yew::prelude::*;

use super::Cell;
use super::session::CellOutput;
use crate::shared::{BTN_BASE, BTN_BORDERED, BTN_INK, BTN_LINK, BTN_SM};

/// The cell editor textarea: fills the row beside the badge gutter, vertically
/// resizable, monospace.
const CELL_EDITOR: &str =
    "min-w-0 flex-1 resize-y bg-canvas p-3 font-mono text-[13px] leading-5 text-ink outline-none";
/// The captured `print` output block beneath a cell.
const CELL_OUT: &str =
    "overflow-auto whitespace-pre bg-canvas px-3 py-2 font-mono text-[13px] leading-5 text-ink";

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
