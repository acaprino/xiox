//! How safe an item is to remove. The engine's three levels, plus `danger` for what only an
//! explicit confirmation of that one item may remove.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Level {
    Recommended,
    Optional,
    Caution,
    Danger,
}

/// The one wording of the `danger` level, in the listing and in the guide.
pub const DANGER_TEXT: &str = "danger: explicit confirmation required";

impl From<xiox_model::Level> for Level {
    fn from(level: xiox_model::Level) -> Level {
        match level {
            xiox_model::Level::Recommended => Level::Recommended,
            xiox_model::Level::Optional => Level::Optional,
            xiox_model::Level::Caution => Level::Caution,
        }
    }
}
