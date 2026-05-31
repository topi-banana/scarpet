//! `scarpet-playground`: a browser playground for the `scarpet` formatter and syntax tree.
//!
//! A two-pane editor — type Scarpet (`.sc`) source on the left, then run the formatter or
//! dump the lossless syntax tree on the right with the buttons in the top-right. The
//! formatter's `Config` is editable from the options bar between the header and the panes.
//! Everything runs in the browser via `wasm32`; there is no server round-trip.

use scarpet_fmt::{BraceStyle, Config, LineEnding};
use scarpet_syntax::parser::ParseError;
use web_sys::{HtmlInputElement, HtmlSelectElement, HtmlTextAreaElement};
use yew::prelude::*;

/// Which tool produced the current output.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// `scarpet-fmt` pretty-printer output.
    Format,
    /// `scarpet-syntax` lossless CST dump.
    Syntax,
}

impl Mode {
    /// Label shown above the output pane.
    fn output_title(self) -> &'static str {
        match self {
            Mode::Format => "Formatted",
            Mode::Syntax => "Syntax tree",
        }
    }
}

enum Msg {
    /// The input textarea changed.
    Input(String),
    /// Run a tool over the current input.
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
}

/// A small (deliberately unformatted) sample so the playground does something on first load.
const SAMPLE: &str = "// Scarpet sample — hit Format or Syntax tree.
fib(n) -> if(n < 2, n, fib(n-1)+fib(n-2));
sum = 0;
loop(10, sum += fib(_) );
print('sum of first 10 fib = '+sum)
";

/// Shared base classes for the toolbar buttons.
const BTN_BASE: &str = "inline-flex h-9 cursor-pointer items-center rounded-md px-3 text-sm font-medium transition-colors";

struct App {
    input: String,
    output: String,
    /// Human-readable diagnostics: a parse error's headline plus an optional `help:` line.
    diagnostics: Vec<String>,
    /// The tool that produced `output`, once one has run.
    mode: Option<Mode>,
    /// The formatting style applied in [`Mode::Format`], edited from the options bar.
    config: Config,
}

/// Render a parse error as a `start..end  message` headline plus an optional `help:` line.
fn diagnostics_for(err: &ParseError) -> Vec<String> {
    let mut out = vec![format!(
        "{}..{}  {}",
        err.span.start,
        err.span.end,
        err.message()
    )];
    if let Some(help) = &err.help {
        out.push(format!("help: {help}"));
    }
    out
}

impl App {
    /// Run `mode` over the current input, refreshing `output` and `diagnostics`.
    fn run(&mut self, mode: Mode) {
        match mode {
            Mode::Format => match scarpet_fmt::format_source(&self.input, &self.config) {
                Ok(formatted) => {
                    self.output = formatted;
                    self.diagnostics = Vec::new();
                }
                Err(scarpet_fmt::FmtError::Parse(err)) => {
                    self.output = String::new();
                    self.diagnostics = diagnostics_for(&err);
                }
            },
            Mode::Syntax => match scarpet_syntax::parser::parse_source(&self.input) {
                Ok(cst) => {
                    self.output = format!("{cst:#?}");
                    self.diagnostics = Vec::new();
                }
                Err(err) => {
                    self.output = String::new();
                    self.diagnostics = diagnostics_for(&err);
                }
            },
        }
        self.mode = Some(mode);
    }

    /// Re-run the formatter in place after a config change, but only while the
    /// Format view is showing. The syntax tree is config-independent, so there a
    /// config change just updates the controls (it applies on the next Format).
    fn reformat_if_showing(&mut self) {
        if self.mode == Some(Mode::Format) {
            self.run(Mode::Format);
        }
    }

    /// The diagnostics strip below the output, or nothing when the input parsed.
    fn view_diagnostics(&self) -> Html {
        if self.diagnostics.is_empty() {
            return html! {};
        }
        html! {
            <div class="max-h-40 shrink-0 overflow-auto border-t border-hairline bg-canvas px-4 py-2 font-mono text-xs text-error">
                <div class="pb-1 font-medium">{ "Parse error" }</div>
                { for self.diagnostics.iter().map(|d| html! { <div class="py-0.5">{ d }</div> }) }
            </div>
        }
    }

    /// The formatter-options bar between the header and the panes. Edits apply
    /// live to the Format view; in the Syntax view they take effect on next Format.
    fn view_options(&self, ctx: &Context<Self>) -> Html {
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

impl Component for App {
    type Message = Msg;
    type Properties = ();

    fn create(_ctx: &Context<Self>) -> Self {
        let mut app = App {
            input: SAMPLE.to_string(),
            output: String::new(),
            diagnostics: Vec::new(),
            mode: None,
            config: Config::default(),
        };
        app.run(Mode::Format);
        app
    }

    fn update(&mut self, _ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            // Track the value without re-rendering: the textarea DOM already holds it, so
            // re-rendering here would be wasted work. The next `Run` reads `self.input`.
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
        }
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let link = ctx.link();
        let oninput = link.callback(|e: InputEvent| {
            let textarea: HtmlTextAreaElement = e.target_unchecked_into();
            Msg::Input(textarea.value())
        });
        let on_format = link.callback(|_| Msg::Run(Mode::Format));
        let on_syntax = link.callback(|_| Msg::Run(Mode::Syntax));

        let output_title = self.mode.map_or("Output", Mode::output_title);
        let label = "border-b border-hairline bg-canvas px-4 py-2 font-mono text-xs font-medium uppercase tracking-wider text-mute";
        let editor = "min-h-0 flex-1 overflow-auto whitespace-pre bg-canvas p-4 font-mono text-[13px] leading-5 text-ink outline-none";

        html! {
            <div class="flex h-screen flex-col bg-canvas-soft text-ink">
                <header class="flex h-16 shrink-0 items-center justify-between border-b border-hairline bg-canvas px-6">
                    <div class="flex items-baseline gap-2">
                        <span class="text-base font-semibold tracking-tight">{ "scarpet" }</span>
                        <span class="text-sm text-mute">{ "playground" }</span>
                    </div>
                    <div class="flex items-center gap-2">
                        <button
                            onclick={on_syntax}
                            class={classes!(BTN_BASE, "border", "border-hairline", "bg-canvas", "text-ink", "hover:bg-canvas-soft")}
                        >
                            { "Syntax tree" }
                        </button>
                        <button
                            onclick={on_format}
                            class={classes!(BTN_BASE, "bg-ink", "text-canvas", "hover:opacity-90")}
                        >
                            { "Format" }
                        </button>
                    </div>
                </header>
                { self.view_options(ctx) }
                <main class="grid min-h-0 flex-1 grid-cols-1 md:grid-cols-2">
                    <section class="flex min-h-0 flex-col border-b border-hairline md:border-b-0 md:border-r">
                        <div class={label}>{ "Input" }</div>
                        <textarea
                            class={editor}
                            spellcheck="false"
                            placeholder="Type Scarpet source here…"
                            value={self.input.clone()}
                            oninput={oninput}
                        />
                    </section>
                    <section class="flex min-h-0 flex-col">
                        <div class={label}>{ output_title }</div>
                        <pre class={editor}>{ &self.output }</pre>
                        { self.view_diagnostics() }
                    </section>
                </main>
            </div>
        }
    }
}

fn main() {
    yew::Renderer::<App>::new().render();
}
