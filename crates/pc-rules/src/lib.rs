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
pub mod regguard;
pub mod safedelete;
pub mod wildcard;

pub use expand::expand;
pub use guard::{protected, protected_dir_removal, Guard};
pub use regguard::reg_allowed;
pub use wildcard::wildcard_match;
pub use model::{Action, Cleaner, Env, Opt, Origin, RegRoot, Search};
