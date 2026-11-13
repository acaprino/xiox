//! Motore di Xiox senza interfaccia: i moduli che pulizia, registro, servizi e
//! sviluppatori condividono, indipendenti da egui.

pub mod account;
pub mod browserdb;
pub mod cleaner;
pub mod dev;
pub mod envpath;
#[cfg(test)]
mod live_checks;
pub mod native;
pub mod notify;
pub mod netcreds;
pub mod regbackup;
pub mod registry;
pub mod rules;
pub mod services;
pub mod sys;
pub mod sysclean;
pub mod winold;
pub mod wipe;
