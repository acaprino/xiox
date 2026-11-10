//! Regole di pulizia esterne (Winapp2, CleanerML): modello, espansione dei
//! percorsi e percorsi protetti.
//!
//! # Contratto per Task 7 (pulizia)
//!
//! - File e cartelle si cancellano **solo** con
//!   [`safedelete::delete_checked_with`], con un solo [`Guard`] per pulizia;
//!   le cartelle di `Collected::dirs_to_remove` dopo i file, nell'ordine dato.
//! - Prima di cancellare una chiave o un valore di registro si rifà
//!   [`regguard::reg_allowed`]; le chiavi si aprono con `REG_OPTION_OPEN_LINK`
//!   (un collegamento simbolico del registro non si segue).
//! - Un `Collected` con `cancelled` non si pulisce.
//! - Rinviati: segnaposto di cloud (reparse point dei file su richiesta) e
//!   byte dei collegamenti fisici (oggi contati per intero).

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
