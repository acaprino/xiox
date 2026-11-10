//! Regole di pulizia esterne (Winapp2, CleanerML): modello, espansione dei
//! percorsi e percorsi protetti.

pub mod model;
pub mod expand;
pub mod guard;
pub mod collect;
pub mod cleanerml;
pub mod embedded;
pub mod updater;
pub mod winapp2;

pub use expand::expand;
pub use guard::protected;
pub use model::{Action, Cleaner, Env, Opt, Origin, RegRoot, Search};
