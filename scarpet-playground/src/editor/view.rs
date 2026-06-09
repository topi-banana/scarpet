//! The editor's two presentational components: [`EditorActions`] (the header
//! buttons) and [`EditorView`] (the panes). Both reach [`App`](crate::app::App)
//! only through callbacks; the tool logic lives in the parent [`editor`](super)
//! module.

use web_sys::HtmlTextAreaElement;
use yew::prelude::*;

use super::Mode;
use crate::shared::{BTN_BASE, BTN_BORDERED, BTN_INK, BTN_LINK, EDITOR, LABEL};

#[derive(Properties, PartialEq)]
pub struct EditorActionsProps {
    /// Fired with the tool to run.
    pub on_run: Callback<Mode>,
}

/// The editor's header buttons (Syntax tree / AST / Format / Run).
pub struct EditorActions;

impl Component for EditorActions {
    type Message = ();
    type Properties = EditorActionsProps;

    fn create(_ctx: &Context<Self>) -> Self {
        Self
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let props = ctx.props();
        let run = |mode: Mode| {
            let cb = props.on_run.clone();
            Callback::from(move |_: web_sys::MouseEvent| cb.emit(mode))
        };
        html! {
            <button onclick={run(Mode::Syntax)} class={classes!(BTN_BASE, BTN_BORDERED)}>{ "Syntax tree" }</button>
            <button onclick={run(Mode::Ast)} class={classes!(BTN_BASE, BTN_BORDERED)}>{ "AST" }</button>
            <button onclick={run(Mode::Format)} class={classes!(BTN_BASE, BTN_INK)}>{ "Format" }</button>
            <button onclick={run(Mode::Run)} class={classes!(BTN_BASE, BTN_LINK)}>{ "Run" }</button>
        }
    }
}

#[derive(Properties, PartialEq)]
pub struct EditorViewProps {
    pub input: AttrValue,
    pub output: AttrValue,
    pub diagnostics: Vec<String>,
    pub diagnostics_title: AttrValue,
    pub mode: Option<Mode>,
    /// Fired with the textarea's new value on every edit.
    pub on_input: Callback<String>,
}

/// The two-pane body: input textarea on the left, output (and any diagnostics)
/// on the right.
pub struct EditorView;

impl Component for EditorView {
    type Message = ();
    type Properties = EditorViewProps;

    fn create(_ctx: &Context<Self>) -> Self {
        Self
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let props = ctx.props();
        let oninput = {
            let cb = props.on_input.clone();
            Callback::from(move |e: web_sys::InputEvent| {
                let textarea: HtmlTextAreaElement = e.target_unchecked_into();
                cb.emit(textarea.value());
            })
        };
        let output_title = props.mode.map_or("Output", Mode::output_title);

        let diagnostics = if props.diagnostics.is_empty() {
            html! {}
        } else {
            html! {
                <div class="max-h-40 shrink-0 overflow-auto border-t border-hairline bg-canvas px-4 py-2 font-mono text-xs text-error">
                    <div class="pb-1 font-medium">{ props.diagnostics_title.clone() }</div>
                    { for props.diagnostics.iter().map(|d| html! { <div class="py-0.5">{ d }</div> }) }
                </div>
            }
        };

        html! {
            <main class="grid min-h-0 flex-1 grid-cols-1 md:grid-cols-2">
                <section class="flex min-h-0 flex-col border-b border-hairline md:border-b-0 md:border-r">
                    <div class={LABEL}>{ "Input" }</div>
                    <textarea
                        class={EDITOR}
                        spellcheck="false"
                        placeholder="Type Scarpet source here…"
                        value={props.input.clone()}
                        oninput={oninput}
                    />
                </section>
                <section class="flex min-h-0 flex-col">
                    <div class={LABEL}>{ output_title }</div>
                    <pre class={EDITOR}>{ props.output.clone() }</pre>
                    { diagnostics }
                </section>
            </main>
        }
    }
}
