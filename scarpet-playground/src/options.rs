//! The formatter-options bar shared by both views. It parses each control's
//! input and emits only well-formed values; an unparseable entry emits nothing,
//! leaving the current config untouched.

use scarpet_fmt::{BraceStyle, Config, LineEnding};
use web_sys::{HtmlInputElement, HtmlSelectElement};
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct OptionsBarProps {
    /// The current style, so the controls show its values.
    pub config: Config,
    pub on_indent: Callback<usize>,
    pub on_max: Callback<usize>,
    pub on_comment: Callback<Option<usize>>,
    pub on_line_ending: Callback<LineEnding>,
    pub on_brace: Callback<BraceStyle>,
}

#[function_component(OptionsBar)]
pub fn options_bar(props: &OptionsBarProps) -> Html {
    let config = props.config;

    let on_indent = {
        let cb = props.on_indent.clone();
        Callback::from(move |e: web_sys::InputEvent| {
            let input: HtmlInputElement = e.target_unchecked_into();
            if let Ok(v) = input.value().parse::<usize>() {
                cb.emit(v);
            }
        })
    };
    let on_max = {
        let cb = props.on_max.clone();
        Callback::from(move |e: web_sys::InputEvent| {
            let input: HtmlInputElement = e.target_unchecked_into();
            if let Ok(v) = input.value().parse::<usize>() {
                cb.emit(v);
            }
        })
    };
    let on_comment = {
        let cb = props.on_comment.clone();
        Callback::from(move |e: web_sys::InputEvent| {
            let input: HtmlInputElement = e.target_unchecked_into();
            // 0 means "leave comments unwrapped" (`comment_width = None`).
            match input.value().parse::<usize>() {
                Ok(0) => cb.emit(None),
                Ok(v) => cb.emit(Some(v)),
                Err(_) => {}
            }
        })
    };
    let on_line_ending = {
        let cb = props.on_line_ending.clone();
        Callback::from(move |e: web_sys::Event| {
            let select: HtmlSelectElement = e.target_unchecked_into();
            cb.emit(match select.value().as_str() {
                "crlf" => LineEnding::Crlf,
                "auto" => LineEnding::Auto,
                "native" => LineEnding::Native,
                _ => LineEnding::Lf,
            });
        })
    };
    let on_brace = {
        let cb = props.on_brace.clone();
        Callback::from(move |e: web_sys::Event| {
            let select: HtmlSelectElement = e.target_unchecked_into();
            cb.emit(match select.value().as_str() {
                "next" => BraceStyle::NextLine,
                _ => BraceStyle::SameLine,
            });
        })
    };

    let bar =
        "flex flex-wrap items-center gap-x-5 gap-y-2 border-b border-hairline bg-canvas px-6 py-2";
    let lbl = "flex items-center gap-2 font-mono text-xs uppercase tracking-wider text-mute";
    let num = "w-14 rounded-md border border-hairline bg-canvas px-2 py-1 text-right font-mono text-xs normal-case text-ink outline-none focus:border-link";
    let sel = "rounded-md border border-hairline bg-canvas px-2 py-1 font-mono text-xs normal-case text-ink outline-none focus:border-link";

    html! {
        <div class={bar}>
            <label class={lbl}>
                { "Indent" }
                <input type="number" min="1" max="16" class={num}
                    value={config.indent_width.to_string()} oninput={on_indent} />
            </label>
            <label class={lbl}>
                { "Width" }
                <input type="number" min="1" class={num}
                    value={config.max_width.to_string()} oninput={on_max} />
            </label>
            <label class={lbl} title="0 leaves comments unwrapped">
                { "Comment" }
                <input type="number" min="0" class={num}
                    value={config.comment_width.unwrap_or(0).to_string()} oninput={on_comment} />
            </label>
            <label class={lbl}>
                { "Endings" }
                <select class={sel} onchange={on_line_ending}>
                    <option value="lf" selected={config.line_ending == LineEnding::Lf}>{ "LF" }</option>
                    <option value="crlf" selected={config.line_ending == LineEnding::Crlf}>{ "CRLF" }</option>
                    <option value="auto" selected={config.line_ending == LineEnding::Auto}>{ "Auto" }</option>
                    <option value="native" selected={config.line_ending == LineEnding::Native}>{ "Native" }</option>
                </select>
            </label>
            <label class={lbl}>
                { "Braces" }
                <select class={sel} onchange={on_brace}>
                    <option value="same" selected={config.brace_style == BraceStyle::SameLine}>{ "Same line" }</option>
                    <option value="next" selected={config.brace_style == BraceStyle::NextLine}>{ "Next line" }</option>
                </select>
            </label>
        </div>
    }
}
