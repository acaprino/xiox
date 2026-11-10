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
pub use collect::{
    collect, collect_ctl, collect_with_excludes, overlaps, present, reg_exists_real, running,
    CollectCtl, Collected, ExcludeRule, RegItem,
};
pub use embedded::embedded;
#[cfg(windows)]
pub use safedelete::{delete_checked, delete_checked_with};
