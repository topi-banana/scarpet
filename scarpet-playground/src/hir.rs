//! The type view: run `scarpet-hir` over the editor input and lay its HIR out
//! as a collapsible tree with a type badge on every node.
//!
//! The model lives here on [`App`]; the presentational components are in
//! [`view`]. Flattening happens here rather than in the components because Yew
//! renders a *list* of rows, not nested components — a deeply nested program
//! would otherwise become a deeply nested component tree, and the whole point of
//! `scarpet-hir`'s iterative walk is to keep depth off the stack.
//!
//! The rows are cached rather than rebuilt per render: `view` runs on every
//! message, and rebuilding would walk the whole arena — over a hundred thousand
//! nodes for a large script — just to redraw a selection highlight. Only
//! analysing and folding change them.

mod view;

use std::collections::HashSet;
use std::rc::Rc;

use scarpet_hir::{Analysis, ExprId, Ty};

use crate::app::App;
use crate::shared::render;

pub use view::{HirPanel, HirRow, HirTree, Tone};

/// How many rows the tree renders before it stops.
///
/// One `<div>` per HIR node, and the corpus has files with tens of thousands of
/// them. Truncating keeps a pasted 150 KB script from locking the tab; the view
/// says how many rows it dropped rather than pretending it showed everything.
const MAX_ROWS: usize = 2_000;

/// An analysis together with the exact text it was run on.
///
/// The two must travel together. Like the other tools, the type view keeps its
/// result on screen after the source is edited — but a `Span` only means
/// something against the buffer it was measured in, so slicing the *current*
/// input with an *older* analysis's spans would show text from the wrong
/// offsets in the detail bar.
pub struct Analyzed {
    pub source: String,
    pub analysis: Analysis,
}

impl App {
    /// Analyse the current input and rebuild the tree.
    pub(crate) fn run_hir(&mut self) {
        let analysis = scarpet_hir::analyze(&self.input);
        self.diagnostics = analysis.diagnostics().iter().flat_map(render).collect();
        self.diagnostics_title = "Diagnostics";
        self.output = String::new();
        // A fresh tree: nothing folded, nothing selected.
        self.hir_collapsed.clear();
        self.hir_selected = None;
        self.analysis = Some(Rc::new(Analyzed {
            source: self.input.clone(),
            analysis,
        }));
        self.rebuild_hir_rows();
    }

    /// Refresh the row cache. Called when the analysis or the folded set
    /// changes — not per render.
    pub(crate) fn rebuild_hir_rows(&mut self) {
        let (rows, truncated) = self.flatten_hir();
        self.hir_rows = Rc::new(rows);
        self.hir_truncated = truncated;
    }

    /// Flatten the analysis into the rows the tree draws, honouring the folded
    /// set, along with how many the row cap dropped.
    fn flatten_hir(&self) -> (Vec<HirRow>, usize) {
        let Some(analyzed) = self.analysis.as_ref() else {
            return (Vec::new(), 0);
        };
        let Some(hir) = analyzed.analysis.hir() else {
            return (Vec::new(), 0);
        };

        // Attribute each diagnostic to the innermost node it lands on, so a
        // flagged row is the one actually at fault.
        let flagged: HashSet<ExprId> = analyzed
            .analysis
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| hir.expr_at(diagnostic.span.start))
            .collect();

        let mut rows = Vec::new();
        let mut visible = 0usize;
        // Depth of the folded ancestor whose subtree is being skipped.
        let mut skip_below: Option<usize> = None;
        for (id, depth) in scarpet_hir::walk(hir) {
            if skip_below.is_some_and(|limit| depth > limit) {
                continue;
            }
            skip_below = None;
            let expandable = hir.children(id).next().is_some();
            let expanded = expandable && !self.hir_collapsed.contains(&id);
            if expandable && !expanded {
                skip_below = Some(depth);
            }
            visible += 1;
            if rows.len() >= MAX_ROWS {
                continue;
            }
            let ty = analyzed.analysis.ty(id);
            rows.push(HirRow {
                id,
                depth,
                label: hir.label(id),
                ty: ty.to_string(),
                tone: tone_of(ty),
                expandable,
                expanded,
                flagged: flagged.contains(&id),
            });
        }
        let truncated = visible - rows.len();
        (rows, truncated)
    }

    /// The tree the view renders, or `None` when there is nothing to show —
    /// no analysis yet, or a source that did not parse.
    pub(crate) fn hir_panel(&self) -> Option<HirPanel> {
        let analyzed = self.analysis.as_ref()?;
        let hir = analyzed.analysis.hir()?;
        // The detail is read against `analyzed.source`, the text the spans were
        // measured in, not against whatever the textarea holds now.
        let detail = self.hir_selected.map(|id| {
            let span = hir.span(id);
            (span.start, span.end, span.text(&analyzed.source).to_owned())
        });
        Some(HirPanel {
            rows: Rc::clone(&self.hir_rows),
            selected: self.hir_selected,
            detail,
            truncated: self.hir_truncated,
        })
    }
}

/// Which of the three badge colours a type gets: containers stand out from
/// scalars, and "no information" recedes.
fn tone_of(ty: &Ty) -> Tone {
    match ty {
        Ty::Unknown | Ty::Never => Tone::Vague,
        Ty::List(_) | Ty::Map(_, _) => Tone::Container,
        Ty::Union(members) => {
            if members
                .iter()
                .any(|member| matches!(member, Ty::List(_) | Ty::Map(_, _)))
            {
                Tone::Container
            } else {
                Tone::Scalar
            }
        }
        _ => Tone::Scalar,
    }
}
