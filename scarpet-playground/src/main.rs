//! `scarpet-playground`: a browser playground for the `scarpet` formatter, syntax
//! tree, and VM.
//!
//! Two screens, toggled from the header:
//! - **Editor** — type Scarpet (`.sc`) source on the left, then run the
//!   formatter, dump the lossless syntax tree (CST), lower it to the typed AST,
//!   or evaluate it once with `scarpet-vm` — on the right.
//! - **Notebook** — a column of cells over a persistent kernel, so a binding or
//!   function defined in one cell is visible to the next (the CLI REPL model).
//!
//! Everything runs in the browser via `wasm32`; there is no server round-trip.

mod app;
mod editor;
mod notebook;
mod session;
mod shared;

fn main() {
    yew::Renderer::<app::App>::new().render();
}
