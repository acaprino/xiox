//! The fallback for tools the registry does not know: an output directory named by a flag.

use super::{words, write_op};
use crate::provenance::{Arg, RawOp};

const OUTPUT_FLAGS: [&str; 8] = [
    "--target-dir",
    "--basetemp",
    "--out",
    "--output",
    "--out-dir",
    "--outdir",
    "--output-dir",
    "--output-directory",
];

/// A write for each output flag, with its value inline (`--out=x`) or as the next word.
pub(super) fn output_directories(args: &[Arg]) -> Vec<RawOp> {
    let words = words(args);
    let mut ops = Vec::new();
    for (i, word) in words.iter().enumerate() {
        let (flag, inline) = match word.split_once('=') {
            Some((flag, value)) => (flag, Some(value)),
            None => (*word, None),
        };
        if !OUTPUT_FLAGS.contains(&flag) {
            continue;
        }
        let value = inline.or_else(|| words.get(i + 1).copied());
        ops.extend(value.filter(|value| !value.is_empty()).map(write_op));
    }
    ops
}
