//! Pure core of `xiox observe`: the baseline snapshot of one project, the differences found by
//! later checks, the thresholds, the report and a bounded size measurer. It runs nothing and
//! writes nothing: the process that owns it feeds it scans and prints its events.

mod measure;
mod state;

pub use measure::measure_bytes;
pub use state::{AttentionKind, Event, ItemView, Observer, Params, Report, Totals};

#[cfg(test)]
mod tests;
