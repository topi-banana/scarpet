//! The type view's presentational components: the collapsible tree and the
//! detail bar under it. Both reach [`App`](crate::app::App) only through
//! callbacks; the flattening lives in the parent [`hir`](super) module.

use yew::prelude::*;

use scarpet_hir::ExprId;

/// Which colour a type badge gets.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// A scalar: number, string, boolean, null.
    Scalar,
    /// A list or map — worth telling apart from a scalar at a glance.
    Container,
    /// `unknown` or `never`: the analysis has nothing to say, so the badge
    /// recedes instead of competing for attention.
    Vague,
}

impl Tone {
    fn class(self) -> &'static str {
        match self {
            Tone::Scalar => "bg-link-soft text-link",
            Tone::Container => "bg-violet-soft text-violet",
            Tone::Vague => "bg-canvas-soft text-mute",
        }
    }
}

/// One line of the tree.
#[derive(Clone, PartialEq)]
pub struct HirRow {
    pub id: ExprId,
    pub depth: usize,
    pub label: String,
    pub ty: String,
    pub tone: Tone,
    pub expandable: bool,
    pub expanded: bool,
    /// Whether a diagnostic landed on this node.
    pub flagged: bool,
}

/// Everything the type view needs to draw itself.
///
/// The rows are behind an `Rc` because Yew compares and clones props on every
/// render, and a large program's tree is thousands of them.
#[derive(Clone, PartialEq)]
pub struct HirPanel {
    pub rows: std::rc::Rc<Vec<HirRow>>,
    pub selected: Option<ExprId>,
    /// The selected node's `(start, end, source text)`.
    pub detail: Option<(u32, u32, String)>,
    /// How many rows were dropped by the row cap.
    pub truncated: usize,
}

#[derive(Properties, PartialEq)]
pub struct HirTreeProps {
    pub panel: HirPanel,
    /// Fired with the node whose children should be shown or hidden.
    pub on_toggle: Callback<ExprId>,
    /// Fired with the node the detail bar should describe.
    pub on_select: Callback<ExprId>,
}

/// The type tree: one row per HIR node, indented by depth, each with its
/// inferred type in a badge.
pub struct HirTree;

impl Component for HirTree {
    type Message = ();
    type Properties = HirTreeProps;

    fn create(_ctx: &Context<Self>) -> Self {
        Self
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let props = ctx.props();
        let panel = &props.panel;

        let rows = panel.rows.iter().map(|row| {
            let selected = panel.selected == Some(row.id);
            let background = if selected {
                "bg-link-soft"
            } else {
                "hover:bg-canvas-soft"
            };

            // A disclosure triangle only where there is something to disclose;
            // a leaf keeps the same indent with a blank of equal width.
            let toggle = if row.expandable {
                let cb = props.on_toggle.clone();
                let id = row.id;
                let onclick = Callback::from(move |e: web_sys::MouseEvent| {
                    // The row itself selects; the triangle only folds.
                    e.stop_propagation();
                    cb.emit(id);
                });
                html! {
                    <span class="inline-block w-4 shrink-0 cursor-pointer text-mute select-none" onclick={onclick}>
                        { if row.expanded { "▾" } else { "▸" } }
                    </span>
                }
            } else {
                html! { <span class="inline-block w-4 shrink-0" /> }
            };

            let onclick = {
                let cb = props.on_select.clone();
                let id = row.id;
                Callback::from(move |_: web_sys::MouseEvent| cb.emit(id))
            };

            html! {
                <div
                    class={classes!(
                        "flex", "cursor-default", "items-center", "gap-2",
                        "whitespace-nowrap", "py-0.5", "pr-4", background,
                    )}
                    style={format!("padding-left: {}px", 8 + row.depth * 14)}
                    onclick={onclick}
                >
                    { toggle }
                    <span class="shrink-0">{ &row.label }</span>
                    if row.flagged {
                        <span class="shrink-0 text-warning" title="a diagnostic points here">{ "⚠" }</span>
                    }
                    <span class="min-w-0 grow" />
                    <span class={classes!(
                        "shrink-0", "rounded", "px-1.5", "py-px", "text-[11px]", row.tone.class(),
                    )}>{ &row.ty }</span>
                </div>
            }
        });

        let truncated = if panel.truncated == 0 {
            html! {}
        } else {
            html! {
                <div class="px-4 py-1 text-xs text-mute">
                    { format!("… {} more nodes not shown", panel.truncated) }
                </div>
            }
        };

        let detail = match &panel.detail {
            None => html! {
                <div class="shrink-0 border-t border-hairline bg-canvas px-4 py-2 font-mono text-xs text-mute">
                    { "Select a node to see the source it covers." }
                </div>
            },
            Some((start, end, text)) => html! {
                <div class="shrink-0 border-t border-hairline bg-canvas px-4 py-2 font-mono text-xs">
                    <span class="text-mute">{ format!("{start}..{end}") }</span>
                    <span class="pl-3 text-ink">{ single_line(text) }</span>
                </div>
            },
        };

        html! {
            <div class="flex min-h-0 flex-1 flex-col">
                <div class="min-h-0 flex-1 overflow-auto bg-canvas py-1 font-mono text-[13px] leading-5 text-ink">
                    { for rows }
                    { truncated }
                </div>
                { detail }
            </div>
        }
    }
}

/// Collapse a multi-line span into one line so the detail bar keeps its height.
fn single_line(text: &str) -> String {
    const MAX: usize = 160;
    let mut flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > MAX {
        flat = flat.chars().take(MAX).collect::<String>() + "…";
    }
    flat
}
