//! Builder-vs-fixture drift check (tests only).
//!
//! The page is developed and screenshotted against `apps/desktop/fixtures/views.json`; the real
//! views come from the builders in this crate. Two lies of that kind were found by hand (the About
//! text, `18204` in a list and `18,204` in its detail box), so the static strings of every section
//! state are compared here, mechanically: for each fixture state the section's tests can build with
//! a small injected state, the REAL view is built and its labels, column titles, nav ids and names,
//! option labels, action ids/labels/keys in order, search placeholder, empty-state texts, dialog
//! titles, texts and buttons, report titles, notice tones and texts and status/footer texts must
//! equal the fixture's. Dynamic data (rows, counts, sizes, paths) is excluded: numbers, sizes and
//! the plural `s` after a number are normalised away by [`norm`]. A separate scan ([`count_problems`])
//! looks for the two bug classes themselves in any string: a count of 1000 or more with no
//! grouping, and a count that disagrees with its word (`1 files`, `3 file`).
//!
//! Each section's replay of the shared script (`*_tests.rs` of the section) calls [`diff`] at every
//! step that lands on a fixture state, and the section tests build the states the script does not
//! visit.

use crate::view::SectionView;
use serde_json::Value;

/// The shared fixture document.
pub(crate) fn document() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/fixtures/views.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// `views.<section>.<state>` of the fixture.
pub(crate) fn fixture_view(doc: &Value, section: &str, state: &str) -> Value {
    doc["views"][section][state].clone()
}

/// What to leave out of a comparison.
#[derive(Clone, Copy)]
pub(crate) struct Opts {
    /// Which nav ids are compared (the others depend on the rules loaded on the machine).
    pub nav: fn(&str) -> bool,
    /// The title and subtitle (they follow the open category in Cleanup).
    pub heading: bool,
    /// The notices (a replay's worker writes its own).
    pub notices: bool,
    /// The dialog (the fixture has one question of several the section can ask).
    pub dialog: bool,
    /// The job's label (it is the worker's progress text).
    pub job: bool,
    /// The report (an empty cleanup has another report than the fixture's).
    pub report: bool,
}

impl Opts {
    /// Everything compared.
    pub(crate) fn all() -> Opts {
        Opts { nav: |_| true, heading: true, notices: true, dialog: true, job: true, report: true }
    }
}

/// The text with its dynamic parts replaced: a number (grouped, with a decimal part or a size unit
/// after it) is `N`, and the plural `s` of the word after it goes. `1.54 GB` and `3` are both `N`;
/// `18,204 files` and `1 file` are both `N file`.
pub(crate) fn norm(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() && !(i > 0 && chars[i - 1].is_alphabetic()) {
            while i < chars.len() && (chars[i].is_ascii_digit() || ((chars[i] == ',' || chars[i] == '.') && chars.get(i + 1).is_some_and(char::is_ascii_digit))) {
                i += 1;
            }
            out.push('N');
            // A size unit after the number belongs to it.
            let rest: String = chars[i..].iter().take(4).collect();
            for unit in [" TB", " GB", " MB", " KB", " B"] {
                if let Some(after) = rest.strip_prefix(unit) {
                    if !after.chars().next().is_some_and(char::is_alphanumeric) {
                        i += unit.len();
                        break;
                    }
                }
            }
            // The word after a number agrees with it: drop a plural `s`.
            if chars.get(i) == Some(&' ') && chars.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic()) {
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_ascii_alphabetic() {
                    j += 1;
                }
                let word: String = chars[i + 1..j].iter().collect();
                let word = if word.len() > 3 && word.ends_with('s') && !word.ends_with("ss") { &word[..word.len() - 1] } else { &word[..] };
                out.push(' ');
                out.push_str(word);
                i = j;
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn s(v: &Value) -> String {
    v.as_str().map(norm).unwrap_or_else(|| if v.is_null() { "∅".into() } else { v.to_string() })
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().map(s).collect()).unwrap_or_default()
}

fn columns(v: &Value) -> Vec<String> {
    v["columns"].as_array().unwrap().iter().map(|c| format!("{}:{}", c["title"].as_str().unwrap(), c["kind"].as_str().unwrap())).collect()
}

fn nav(v: &Value, keep: fn(&str) -> bool) -> Vec<String> {
    v["nav"].as_array().unwrap().iter().filter(|n| keep(n["id"].as_str().unwrap())).map(|n| format!("{}={}", n["id"].as_str().unwrap(), n["name"].as_str().unwrap())).collect()
}

fn options(v: &Value) -> Vec<String> {
    v["options"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["kind"] != "text" || o["tone"] != "error") // "Settings not saved: …" depends on the disk
        .map(|o| {
            let kind = o["kind"].as_str().unwrap();
            let choices = o["choices"].as_array().map(|c| c.iter().map(|c| format!("{}={}", c["id"].as_str().unwrap(), c["label"].as_str().unwrap())).collect::<Vec<_>>().join("|")).unwrap_or_default();
            match kind {
                "text" => format!("text:{}", s(&o["text"])),
                "list" => format!("list:{}:{}:{}:{}:{}", o["id"].as_str().unwrap(), o["label"].as_str().unwrap(), o["addLabel"].as_str().unwrap(), o["removeLabel"].as_str().unwrap(), o["empty"].as_str().unwrap()),
                _ => format!("{kind}:{}:{}:{choices}", o["id"].as_str().unwrap(), o["label"].as_str().unwrap()),
            }
        })
        .collect()
}

fn actions(v: &Value) -> Vec<String> {
    // The verb is compared, not the count after it (`Clean`, `Clean 8 items`: the same button).
    let verb = |label: &Value| s(label).split(" N").next().unwrap_or_default().to_string();
    v["actions"].as_array().unwrap().iter().map(|a| format!("{}|{}|{}|primary={}", a["id"].as_str().unwrap(), verb(&a["label"]), s(&a["key"]), a["primary"])).collect()
}

fn dialog(v: &Value) -> String {
    let d = &v["dialog"];
    if d.is_null() {
        return "none".into();
    }
    let buttons: Vec<String> = d["buttons"].as_array().unwrap().iter().map(|b| format!("{}|{}|primary={}", b["id"].as_str().unwrap(), s(&b["label"]), b["primary"])).collect();
    format!(
        "id={} title={} text={:?} ack={} buttons={:?}",
        d["id"].as_str().unwrap(),
        s(&d["title"]),
        strings(&d["text"]),
        if d["ack"].is_null() { "none" } else { "box" },
        buttons
    )
}

fn empty(v: &Value) -> Option<String> {
    (!v["empty"].is_null()).then(|| format!("{} / {}", s(&v["empty"]["title"]), s(&v["empty"]["text"])))
}

fn report(v: &Value) -> String {
    let r = &v["report"];
    if r.is_null() {
        return "none".into();
    }
    format!("title={} canSave={}", s(&r["title"]), r["canSave"])
}

fn report_notes(v: &Value) -> Vec<String> {
    v["report"]["notes"].as_array().map(|n| n.iter().map(|n| format!("{}:{}", n["tone"].as_str().unwrap(), s(&n["text"]))).collect()).unwrap_or_default()
}

/// A dialog's warnings and cautions headings (they follow what is ticked).
fn dialog_headings(v: &Value) -> (Option<String>, Option<String>) {
    let d = &v["dialog"];
    let h = |k: &str| d[k].as_str().map(norm);
    (h("warningsTitle"), h("cautionTitle"))
}

fn notices(v: &Value) -> Vec<String> {
    v["notices"].as_array().unwrap().iter().map(|n| format!("{}:{}", n["tone"].as_str().unwrap(), s(&n["text"]))).collect()
}

fn job(v: &Value) -> String {
    if v["job"].is_null() { "none".into() } else { s(&v["job"]["label"]) }
}

/// What differs, one line per difference, between the real view and the fixture state.
pub(crate) fn diff(what: &str, real: &SectionView, fixture: &Value, o: Opts) -> Vec<String> {
    let real = serde_json::to_value(real).unwrap();
    let mut out = Vec::new();
    let mut check = |field: &str, r: String, f: String| {
        if r != f {
            out.push(format!("{what}: {field}\n    real:    {r}\n    fixture: {f}"));
        }
    };
    check("section", s(&real["section"]), s(&fixture["section"]));
    if o.heading {
        check("title", s(&real["title"]), s(&fixture["title"]));
        check("subtitle", s(&real["subtitle"]), s(&fixture["subtitle"]));
    }
    check("columns", format!("{:?}", columns(&real)), format!("{:?}", columns(fixture)));
    check("nav", format!("{:?}", nav(&real, o.nav)), format!("{:?}", nav(fixture, o.nav)));
    check("options", format!("{:?}", options(&real)), format!("{:?}", options(fixture)));
    check("actions", format!("{:?}", actions(&real)), format!("{:?}", actions(fixture)));
    check("search", s(&real["search"]["placeholder"]), s(&fixture["search"]["placeholder"]));
    // The reason for an empty list follows the data (a search with no match, a scan with no result):
    // compared where both sides have one.
    if let (Some(r), Some(f)) = (empty(&real), empty(fixture)) {
        check("empty", r, f);
    }
    if o.dialog {
        check("dialog", dialog(&real), dialog(fixture));
        let (rh, fh) = (dialog_headings(&real), dialog_headings(fixture));
        if let (Some(r), Some(f)) = (&rh.0, &fh.0) {
            check("dialog warnings heading", r.clone(), f.clone());
        }
        if let (Some(r), Some(f)) = (&rh.1, &fh.1) {
            check("dialog caution heading", r.clone(), f.clone());
        }
    }
    // How the sizes were measured follows the machine (the disk MFT or folder by folder).
    let status = |v: &Value| format!("{} / {}", s(&v["status"]["badge"]), s(&v["status"]["text"]).replace("folders measured one by one", "sizes from the disk MFT"));
    check("status", status(&real), status(fixture));
    check("footer", s(&real["footer"]), s(&fixture["footer"]));
    if o.job {
        check("job", job(&real), job(fixture));
    }
    if o.report {
        check("report", report(&real), report(fixture));
        // The notes follow what happened (files that stayed): every note the builder writes is one the fixture knows.
        let known = report_notes(fixture);
        for note in report_notes(&real) {
            let expected = if known.contains(&note) { note.clone() } else { format!("(one of {known:?})") };
            check("report note", note, expected);
        }
    }
    if o.notices {
        check("notices", format!("{:?}", notices(&real)), format!("{:?}", notices(fixture)));
    }
    out
}

/// Differences found by [`diff`] that are known and reported (the fixture is the page's, the
/// builder's behaviour stands): the start of the line of the difference, and why. A new difference
/// is never added here without being reported to whoever owns the fixture.
const KNOWN: &[(&str, &str, &str)] = &[
    (
        "registry:scanning: subtitle",
        "No scan yet",
        "the fixture's scanning view says `No scan yet`; the real one counts the issues found so far (`0 issues found`, growing)",
    ),
    (
        "developers:",
        "Also chosen in the other lists",
        "the fixture's confirm dialog says `Also chosen in the other lists (not cleaned now): ...`; the real one says `Does not touch the other sections: ... selected.`",
    ),
];

/// The differences that are not a known one.
pub(crate) fn unknown(all: &[String]) -> Vec<String> {
    all.iter().filter(|d| !KNOWN.iter().any(|(start, contains, _)| d.starts_with(start) && d.contains(contains))).cloned().collect()
}

/// The nouns whose count agrees with them: (singular, plural).
const NOUNS: &[(&str, &str)] = &[
    ("item", "items"),
    ("file", "files"),
    ("folder", "folders"),
    ("rule", "rules"),
    ("issue", "issues"),
    ("session", "sessions"),
    ("cookie", "cookies"),
    ("domain", "domains"),
    ("worktree", "worktrees"),
    ("event", "events"),
];

/// The bug classes in one string: a count of 1000 or more without its grouping commas, and a count
/// that disagrees with the word after it.
pub(crate) fn count_problems(text: &str) -> Vec<String> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut out = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        let number = token.trim_matches(|c: char| !c.is_ascii_digit() && c != ',');
        if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit() || c == ',') || !number.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let value: u64 = number.replace(',', "").parse().unwrap_or(0);
        let word = tokens.get(i + 1).map(|w| w.trim_matches(|c: char| !c.is_ascii_alphabetic())).unwrap_or("");
        let counted = NOUNS.iter().find(|(one, many)| word == *one || word == *many);
        if let Some((one, many)) = counted {
            if value >= 1000 && !number.contains(',') {
                out.push(format!("`{text}`: {number} is not grouped"));
            }
            if (value == 1 && word == *many) || (value != 1 && word == *one) {
                out.push(format!("`{text}`: {number} {word} (should be `{}`)", if value == 1 { one } else { many }));
            }
        }
    }
    out
}

/// Every string of a JSON value, with where it is.
pub(crate) fn walk(v: &Value, at: &str, out: &mut Vec<(String, String)>) {
    match v {
        Value::String(t) => out.push((at.to_string(), t.clone())),
        Value::Array(a) => a.iter().enumerate().for_each(|(i, c)| walk(c, &format!("{at}[{i}]"), out)),
        Value::Object(m) => m.iter().for_each(|(k, c)| walk(c, &format!("{at}.{k}"), out)),
        _ => {}
    }
}

/// The count problems of a whole view (list cells that are a bare ungrouped number count too).
pub(crate) fn view_problems(what: &str, v: &Value) -> Vec<String> {
    let mut strings = Vec::new();
    walk(v, what, &mut strings);
    let mut out = Vec::new();
    for (at, text) in &strings {
        out.extend(count_problems(text).into_iter().map(|p| format!("{at}: {p}")));
        if at.contains(".cells[") && text.len() >= 4 && text.chars().all(|c| c.is_ascii_digit()) {
            out.push(format!("{at}: `{text}` is a count of 1000 or more without grouping"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_hides_numbers_sizes_and_plural_endings_only() {
        assert_eq!(norm("Clean 8 items"), norm("Clean 1 item"));
        assert_eq!(norm("1.54 GB to delete (46,999 files)"), "N to delete (N file)");
        assert_eq!(norm("Winapp2: version 260915"), "Winapp2: version N");
        assert_eq!(norm("5 selected · 2 of 7 sessions"), "N selected · N of N session");
        assert_ne!(norm("Clean items"), norm("Clean file"));
        assert_eq!(norm("Fixed 1 issue. 1 not fixed"), "Fixed N issue. N not fixed");
    }

    /// The scan finds the two bug classes (and only them).
    #[test]
    fn count_problems_find_ungrouped_numbers_and_wrong_plurals() {
        assert!(count_problems("(46,999 files)").is_empty());
        assert!(count_problems("1 file, 0 files, 2 items").is_empty());
        assert!(count_problems("Winapp2: version 260915").is_empty());
        assert_eq!(count_problems("(46999 files)").len(), 1);
        assert_eq!(count_problems("1 items left").len(), 1);
        assert_eq!(count_problems("3 folder removed").len(), 1);
        assert_eq!(view_problems("v", &serde_json::json!({"rows": [{"cells": ["18204"]}]})).len(), 1);
        assert!(view_problems("v", &serde_json::json!({"rows": [{"cells": ["18,204", "512"]}]})).is_empty());
    }

    /// The fixture itself carries neither bug class, anywhere.
    #[test]
    fn the_fixture_has_no_ungrouped_counts_and_no_wrong_plurals() {
        let doc = document();
        let problems = view_problems("fixture", &doc);
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
