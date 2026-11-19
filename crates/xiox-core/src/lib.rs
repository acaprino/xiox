//! Motore di Xiox senza interfaccia: i moduli che pulizia, registro e sviluppatori
//! condividono, indipendenti dall'interfaccia.

pub mod account;
pub mod app;
pub mod browserdb;
pub mod cleaner;
pub mod cleanup;
pub mod dev;
pub mod envpath;
#[cfg(test)]
mod live_checks;
#[cfg(test)]
mod drift;
pub mod native;
pub mod notify;
pub mod netcreds;
pub mod pump;
pub mod regbackup;
pub mod regstate;
pub mod registry;
pub mod rules;
pub mod sys;
pub mod sysclean;
pub mod winold;
pub mod view;
pub mod wipe;
