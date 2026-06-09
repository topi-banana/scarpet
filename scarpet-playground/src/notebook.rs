//! The notebook view: a column of cells run against a persistent kernel, so a
//! binding made in one cell is visible to the next.
//!
//! [`Notebook`] is the model (cells, id allocator, sample loading); the rendering
//! is three presentational components in [`view`] — [`NotebookActions`] (header
//! buttons), [`NotebookView`] (the column), and [`CellView`](view::CellView) (one
//! cell) — that reach [`App`](crate::app::App) only through callbacks. The
//! persistent kernel is [`Session`](session::Session) in [`session`].

pub mod session;
mod view;

use scarpet_fmt::Config;

use crate::notebook::session::CellOutput;
use crate::shared::diagnostics_for;

pub use view::{NotebookActions, NotebookView};

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
/// [`NotebookView`]/[`CellView`](view::CellView) props and skip re-rendering when
/// unchanged.
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
