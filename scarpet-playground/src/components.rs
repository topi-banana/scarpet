//! Cross-view chrome shared by both screens: the top [`Header`](header::Header)
//! bar and the formatter [`OptionsBar`](options::OptionsBar). Neither owns app
//! state; both reach [`App`](crate::app::App) only through callbacks.

pub mod header;
pub mod options;
