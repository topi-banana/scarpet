//! The original two-pane editor view: type Scarpet on the left, then run the
//! formatter, dump the CST, lower to the AST, or evaluate it (a one-shot run,
//! independent of the notebook's persistent kernel) on the right.

use scarpet_fmt::{BraceStyle, LineEnding};
use scarpet_syntax::ast::Code;
use scarpet_syntax::parser::ParseError;
use scarpet_vm::{Evalute, GlobalState};
use std::cell::RefCell;
use std::rc::Rc;
use web_sys::{HtmlInputElement, HtmlSelectElement, HtmlTextAreaElement};
use yew::prelude::*;

use crate::app::{App, Msg};
use crate::shared::{
    BTN_BASE, BTN_BORDERED, BTN_INK, BTN_LINK, EDITOR, LABEL, SharedBuffer, diagnostics_for,
};

/// Which tool produced the current editor output.
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// `scarpet-fmt` pretty-printer output.
    Format,
    /// `scarpet-syntax` lossless CST dump.
    Syntax,
    /// `scarpet-syntax` typed AST dump — the CST lowered via `Code::try_from`.
    Ast,
    /// `scarpet-vm` evaluation — lower to the AST, run it once, and show what the
    /// program printed (captured from the VM's stdout).
    Run,
}

impl Mode {
    /// Label shown above the output pane.
    fn output_title(self) -> &'static str {
        match self {
            Mode::Format => "Formatted",
            Mode::Syntax => "Syntax tree",
            Mode::Ast => "AST",
            Mode::Run => "Output",
        }
    }
}

/// A small sample so the playground does something on first load.
pub const SAMPLE: &str = "// Scarpet sample — hit Run, Format, Syntax tree, or AST.
fib(n) -> if(n < 2, n, fib(n-1)+fib(n-2));
print('fib(10) = '+fib(10));
print('fib(20) = '+fib(20));

fizzbuzz(n) -> for(range(1, n + 1), if(_ % 15 == 0, print('FizzBuzz'), _ % 3 == 0, print('Fizz'), _ % 5 == 0, print('Buzz'), print(_)));
fizzbuzz(20)
";

impl App {
    /// Run `mode` over the current editor input, refreshing `output` and
    /// `diagnostics`.
    pub(crate) fn run(&mut self, mode: Mode) {
        match mode {
            Mode::Format => match scarpet_fmt::format_source(&self.input, &self.config) {
                Ok(formatted) => {
                    self.output = formatted;
                    self.diagnostics = Vec::new();
                }
                Err(scarpet_fmt::FmtError::Parse(err)) => self.report_parse(&err),
            },
            Mode::Syntax => match scarpet_syntax::parser::parse_source(&self.input) {
                Ok(cst) => {
                    self.output = format!("{cst:#?}");
                    self.diagnostics = Vec::new();
                }
                Err(err) => self.report_parse(&err),
            },
            // The AST is the CST lowered via `Code::try_from`, so it has two
            // failure modes: a parse error (shared with the other tools) or a
            // lowering error where a well-formed parse has no valid AST (e.g.
            // `1 = 2`, no assignable target). The lowering error carries no span.
            Mode::Ast => match scarpet_syntax::parser::parse_source(&self.input) {
                Ok(cst) => match Code::try_from(&cst) {
                    Ok(code) => {
                        self.output = format!("{code:#?}");
                        self.diagnostics = Vec::new();
                    }
                    Err(err) => {
                        self.output = String::new();
                        self.diagnostics = vec![err.to_string()];
                        self.diagnostics_title = "Lowering error";
                    }
                },
                Err(err) => self.report_parse(&err),
            },
            Mode::Run => self.run_vm(),
        }
        self.mode = Some(mode);
    }

    /// Evaluate the editor input with `scarpet-vm` and show what it printed. This
    /// is a fresh, one-shot VM each time (unlike the notebook's persistent
    /// kernel): parse and lower like [`Mode::Ast`], then run the AST in a VM
    /// whose `print` output is captured into a buffer.
    fn run_vm(&mut self) {
        let cst = match scarpet_syntax::parser::parse_source(&self.input) {
            Ok(cst) => cst,
            Err(err) => return self.report_parse(&err),
        };
        let code = match Code::try_from(&cst) {
            Ok(code) => code,
            Err(err) => {
                self.output = String::new();
                self.diagnostics = vec![err.to_string()];
                self.diagnostics_title = "Lowering error";
                return;
            }
        };
        let buffer = Rc::new(RefCell::new(Vec::<u8>::new()));
        let mut global = GlobalState::with_stdout(Box::new(SharedBuffer(buffer.clone())));
        let mut vm = global.create_new_vm();
        let result = vm.push(code);
        let printed = String::from_utf8_lossy(&buffer.borrow()).into_owned();
        match result {
            Ok(_) => {
                self.output = printed;
                self.diagnostics = Vec::new();
            }
            Err(err) => {
                self.output = printed;
                self.diagnostics = vec![err.to_string()];
                self.diagnostics_title = "Runtime error";
            }
        }
    }

    /// Record a parse error as the current diagnostics, clearing any stale output.
    fn report_parse(&mut self, err: &ParseError) {
        self.output = String::new();
        self.diagnostics = diagnostics_for(err);
        self.diagnostics_title = "Parse error";
    }

    /// Re-run the formatter in place after a config change, but only while the
    /// Format view is showing. The syntax tree and AST are config-independent.
    pub(crate) fn reformat_if_showing(&mut self) {
        if self.mode == Some(Mode::Format) {
            self.run(Mode::Format);
        }
    }

    /// The editor view's tool buttons (shown in the header on the right).
    pub(crate) fn view_editor_actions(&self, ctx: &Context<Self>) -> Html {
        let link = ctx.link();
        let on_format = link.callback(|_| Msg::Run(Mode::Format));
        let on_syntax = link.callback(|_| Msg::Run(Mode::Syntax));
        let on_ast = link.callback(|_| Msg::Run(Mode::Ast));
        let on_run = link.callback(|_| Msg::Run(Mode::Run));
        html! {
            <button onclick={on_syntax} class={classes!(BTN_BASE, BTN_BORDERED)}>{ "Syntax tree" }</button>
            <button onclick={on_ast} class={classes!(BTN_BASE, BTN_BORDERED)}>{ "AST" }</button>
            <button onclick={on_format} class={classes!(BTN_BASE, BTN_INK)}>{ "Format" }</button>
            <button onclick={on_run} class={classes!(BTN_BASE, BTN_LINK)}>{ "Run" }</button>
        }
    }

    /// The two-pane editor body: input textarea on the left, output on the right.
    pub(crate) fn view_editor(&self, ctx: &Context<Self>) -> Html {
        let link = ctx.link();
        let oninput = link.callback(|e: InputEvent| {
            let textarea: HtmlTextAreaElement = e.target_unchecked_into();
            Msg::Input(textarea.value())
        });
        let output_title = self.mode.map_or("Output", Mode::output_title);

        html! {
            <main class="grid min-h-0 flex-1 grid-cols-1 md:grid-cols-2">
                <section class="flex min-h-0 flex-col border-b border-hairline md:border-b-0 md:border-r">
                    <div class={LABEL}>{ "Input" }</div>
                    <textarea
                        class={EDITOR}
                        spellcheck="false"
                        placeholder="Type Scarpet source here…"
                        value={self.input.clone()}
                        oninput={oninput}
                    />
                </section>
                <section class="flex min-h-0 flex-col">
                    <div class={LABEL}>{ output_title }</div>
                    <pre class={EDITOR}>{ &self.output }</pre>
                    { self.view_diagnostics() }
                </section>
            </main>
        }
    }

    /// The diagnostics strip below the editor output, or nothing when it parsed.
    fn view_diagnostics(&self) -> Html {
        if self.diagnostics.is_empty() {
            return html! {};
        }
        html! {
            <div class="max-h-40 shrink-0 overflow-auto border-t border-hairline bg-canvas px-4 py-2 font-mono text-xs text-error">
                <div class="pb-1 font-medium">{ self.diagnostics_title }</div>
                { for self.diagnostics.iter().map(|d| html! { <div class="py-0.5">{ d }</div> }) }
            </div>
        }
    }

    /// The formatter-options bar between the header and the body. Edits apply
    /// live to the editor's Format view and to per-cell Format in the notebook.
    pub(crate) fn view_options(&self, ctx: &Context<Self>) -> Html {
        let link = ctx.link();

        let on_indent = link.callback(|e: InputEvent| {
            let input: HtmlInputElement = e.target_unchecked_into();
            match input.value().parse::<usize>() {
                Ok(v) => Msg::SetIndentWidth(v.clamp(1, 16)),
                Err(_) => Msg::Noop,
            }
        });
        let on_max = link.callback(|e: InputEvent| {
            let input: HtmlInputElement = e.target_unchecked_into();
            match input.value().parse::<usize>() {
                Ok(v) => Msg::SetMaxWidth(v.max(1)),
                Err(_) => Msg::Noop,
            }
        });
        let on_comment = link.callback(|e: InputEvent| {
            let input: HtmlInputElement = e.target_unchecked_into();
            match input.value().parse::<usize>() {
                Ok(0) => Msg::SetCommentWidth(None),
                Ok(v) => Msg::SetCommentWidth(Some(v)),
                Err(_) => Msg::Noop,
            }
        });
        let on_line_ending = link.callback(|e: Event| {
            let select: HtmlSelectElement = e.target_unchecked_into();
            Msg::SetLineEnding(match select.value().as_str() {
                "crlf" => LineEnding::Crlf,
                "auto" => LineEnding::Auto,
                "native" => LineEnding::Native,
                _ => LineEnding::Lf,
            })
        });
        let on_brace = link.callback(|e: Event| {
            let select: HtmlSelectElement = e.target_unchecked_into();
            Msg::SetBraceStyle(match select.value().as_str() {
                "next" => BraceStyle::NextLine,
                _ => BraceStyle::SameLine,
            })
        });

        let bar = "flex flex-wrap items-center gap-x-5 gap-y-2 border-b border-hairline bg-canvas px-6 py-2";
        let lbl = "flex items-center gap-2 font-mono text-xs uppercase tracking-wider text-mute";
        let num = "w-14 rounded-md border border-hairline bg-canvas px-2 py-1 text-right font-mono text-xs normal-case text-ink outline-none focus:border-link";
        let sel = "rounded-md border border-hairline bg-canvas px-2 py-1 font-mono text-xs normal-case text-ink outline-none focus:border-link";

        html! {
            <div class={bar}>
                <label class={lbl}>
                    { "Indent" }
                    <input type="number" min="1" max="16" class={num}
                        value={self.config.indent_width.to_string()} oninput={on_indent} />
                </label>
                <label class={lbl}>
                    { "Width" }
                    <input type="number" min="1" class={num}
                        value={self.config.max_width.to_string()} oninput={on_max} />
                </label>
                <label class={lbl} title="0 leaves comments unwrapped">
                    { "Comment" }
                    <input type="number" min="0" class={num}
                        value={self.config.comment_width.unwrap_or(0).to_string()} oninput={on_comment} />
                </label>
                <label class={lbl}>
                    { "Endings" }
                    <select class={sel} onchange={on_line_ending}>
                        <option value="lf" selected={self.config.line_ending == LineEnding::Lf}>{ "LF" }</option>
                        <option value="crlf" selected={self.config.line_ending == LineEnding::Crlf}>{ "CRLF" }</option>
                        <option value="auto" selected={self.config.line_ending == LineEnding::Auto}>{ "Auto" }</option>
                        <option value="native" selected={self.config.line_ending == LineEnding::Native}>{ "Native" }</option>
                    </select>
                </label>
                <label class={lbl}>
                    { "Braces" }
                    <select class={sel} onchange={on_brace}>
                        <option value="same" selected={self.config.brace_style == BraceStyle::SameLine}>{ "Same line" }</option>
                        <option value="next" selected={self.config.brace_style == BraceStyle::NextLine}>{ "Next line" }</option>
                    </select>
                </label>
            </div>
        }
    }
}
