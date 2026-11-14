//! The one file the window writes: the report the person asked to save.

use std::path::Path;

/// Writes `text` to `path` (a file the person picked in the save dialog) and says how it went, in
/// the words the report shows: `("Report saved to <path>", true)` or `("Report not saved: <why>", false)`.
pub fn write_report(path: &Path, text: &str) -> (String, bool) {
    match std::fs::write(path, text) {
        Ok(()) => (format!("Report saved to {}", path.display()), true),
        Err(e) => (format!("Report not saved: {e}"), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_report_says_where_or_why_not() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Xiox report.txt");
        let (text, ok) = write_report(&file, "freed 1 MB");
        assert!(ok);
        assert_eq!(text, format!("Report saved to {}", file.display()));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "freed 1 MB");
        // A folder that is not there: nothing is created, and the reason is said.
        let missing = dir.path().join("no such folder").join("r.txt");
        let (text, ok) = write_report(&missing, "x");
        assert!(!ok && text.starts_with("Report not saved: "), "{text}");
        assert!(!missing.exists());
    }
}
