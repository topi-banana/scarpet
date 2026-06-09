//! The top header bar: the brand, the Editor/Notebook toggle, and a slot for the
//! active view's action buttons.

use yew::prelude::*;

use crate::app::View;
use crate::shared::{BTN_BASE, BTN_BORDERED, BTN_INK};

#[derive(Properties, PartialEq)]
pub struct HeaderProps {
    /// The active view, so its toggle segment is highlighted.
    pub view: View,
    /// Fired with the view a toggle segment selects.
    pub on_switch: Callback<View>,
    /// The active view's action buttons, rendered on the right.
    pub children: Html,
}

/// The top header bar.
pub struct Header;

impl Component for Header {
    type Message = ();
    type Properties = HeaderProps;

    fn create(_ctx: &Context<Self>) -> Self {
        Self
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let props = ctx.props();
        let on_editor = {
            let on_switch = props.on_switch.clone();
            Callback::from(move |_: web_sys::MouseEvent| on_switch.emit(View::Editor))
        };
        let on_notebook = {
            let on_switch = props.on_switch.clone();
            Callback::from(move |_: web_sys::MouseEvent| on_switch.emit(View::Notebook))
        };

        let editor_cls = if props.view == View::Editor {
            classes!(BTN_BASE, BTN_INK)
        } else {
            classes!(BTN_BASE, BTN_BORDERED)
        };
        let notebook_cls = if props.view == View::Notebook {
            classes!(BTN_BASE, BTN_INK)
        } else {
            classes!(BTN_BASE, BTN_BORDERED)
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
                <div class="flex items-center gap-2">{ props.children.clone() }</div>
            </header>
        }
    }
}
