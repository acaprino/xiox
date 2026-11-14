//! The Cleanup steps of the shared script (`apps/desktop/fixtures/intent-script.json`) replayed against
//! the real state through the worker seam, and the Cleanup states of the shared fixture read through the
//! Rust view types. The state and its helpers are those of `intent_tests`.

use super::intent_tests::{cleaner, finish, opt, state_with};
use super::*;
use crate::view::testing::notifier;
use crate::view::*;
use std::sync::atomic::Ordering::SeqCst;
use std::time::Duration;

static FINISH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The worker of the replay: lands what the script's state needs once the replay lets it finish
/// (`FINISH`) or the job is cancelled, as the engine does.
fn script_job(job: CleanJob, out: Outbox<CleanMsg>) {
    for _ in 0..500 {
        if job.files_cancel.is_cancelled() || job.cancel.load(SeqCst) || FINISH.load(SeqCst) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    for &i in &job.cats {
        let found = cleaner::Found { count: 3, bytes: 300, ..Default::default() };
        if job.also_clean {
            out.send(CleanMsg::Cleaned(i, found, cleaner::Cleaned { freed: 30, removed: 3, ..Default::default() }));
        } else {
            out.send(CleanMsg::Analyzed(i, found));
        }
    }
    out.send(CleanMsg::Done);
}

/// The rows of the fixture's list that the script names, as rules: three browsers (Chrome's cache is
/// blocked, Firefox's is on, Opera's cookies are at the Caution level) and one Winapp2 entry.
fn script_rules() -> rules::RuleSet {
    use xiox_rules::Origin;
    let mut cookies = opt("cookies", "Cookies", "");
    cookies.warning = Some("You are signed out of sites.".into());
    let mut chrome = opt("cache", "Cache", "");
    chrome.unsupported = Some("needs the old engine".into());
    let mut firefox = opt("cache", "Cache", "");
    firefox.default_on = true;
    rules::RuleSet {
        cleaners: vec![
            cleaner("chrome", "Google Chrome", Origin::CleanerMl("c"), "Browser", vec![chrome]),
            cleaner("firefox", "Firefox", Origin::CleanerMl("f"), "Browser", vec![firefox]),
            cleaner("opera", "Opera", Origin::CleanerMl("o"), "Browser", vec![cookies]),
            cleaner("w_app", "Some App", Origin::Winapp2, "Utilities", vec![opt("cache", "Some App", "")]),
        ],
        winapp2_version: Some("0.1".into()),
        status: "Winapp2 0.1 loaded".into(),
        warnings: vec![],
        env: xiox_rules::Env::for_rules(),
        sections: vec![("Utilities".into(), vec![3])],
        groups: vec![("Browser".into(), vec![0, 1, 2])],
    }
}

/// The ids the fixture's (mock) list gives to the native items, and the real ones they stand for.
const NATIVE_ALIASES: [(&str, &str); 2] = [("native:cookies", "native:cookies_keep_list"), ("native:winold", "native:windows_old")];

fn real_id(id: &str) -> &str {
    NATIVE_ALIASES.iter().find(|(script, _)| *script == id).map_or(id, |(_, real)| real)
}

/// The state the script calls `name` (`go`): its own selection, the first four categories, the
/// cookie item ticked as the fixture's `analyzed` state has it.
fn script_state(name: &str) -> CleanupState {
    let mut st = state_with(CleanupWorkers { job: script_job });
    st.rules = Some(script_rules());
    // The natives the script names: the clipboard, Windows.old (Caution) and the cookies (Caution), which
    // the replay's worker never runs.
    st.natives = native::all().into_iter().filter(|i| ["clipboard", crate::winold::ID, crate::browserdb::COOKIES_ID].contains(&i.id)).collect();
    st.rule_sel = rules::native_defaults(&st.natives);
    st.rule_sel.insert("native:cookies_keep_list".into());
    st.rule_sel.insert("firefox/cache".into());
    st.clean_sel = (0..cleaner::CATEGORIES.len()).map(|i| i < 4).collect();
    if name == "idle" {
        return st;
    }
    let land = |st: &mut CleanupState| {
        FINISH.store(true, SeqCst);
        finish(st);
        FINISH.store(false, SeqCst);
    };
    st.analyze(notifier()).unwrap();
    land(&mut st);
    if name == "report" {
        st.request_clean().unwrap();
        st.confirm(notifier()).unwrap();
        land(&mut st);
    }
    st
}

/// The script's name for what a state looks like. An analysis that was cut short (cancelled or dead)
/// is `interrupted`.
fn shape(st: &CleanupState) -> &'static str {
    if st.busy() {
        return if st.clean_view == CleanView::Cleaned { "cleaning" } else { "analyzing" };
    }
    if st.confirm_clean {
        return "confirm";
    }
    match st.clean_view {
        CleanView::Empty => "idle",
        CleanView::Analyzed if matches!(st.clean_end, Some(CleanEnd::Cancelled | CleanEnd::Died)) => "interrupted",
        CleanView::Analyzed => "analyzed",
        CleanView::Cleaned if st.clean_report.as_ref().is_some_and(|r| r.cancelled) => "cancelled",
        CleanView::Cleaned => "report",
    }
}

/// What the page sends for each intent, and what `CleanupState` is asked to do with it. Over a
/// report the page's «Close» is `release()`.
fn apply(st: &mut CleanupState, intent: CleanupIntent) -> Result<(), String> {
    match intent {
        CleanupIntent::Toggle { row } => st.toggle(&row),
        CleanupIntent::SelectAll { on } => st.select_all(on),
        CleanupIntent::SetOnlyOld { on } => st.set_only_old(on),
        // The script's own note: a search is outside the table, the list is not filtered by the mock.
        // The filtering the real list does is tested in `intent_tests`; here it would hide the rows the
        // later steps tick.
        CleanupIntent::Search { .. } => Ok(()),
        CleanupIntent::Analyze => st.analyze(notifier()),
        CleanupIntent::OpenConfirm => st.request_clean(),
        CleanupIntent::Confirm => st.confirm(notifier()),
        CleanupIntent::CloseDialog if st.confirm_clean => {
            st.close_confirm();
            Ok(())
        }
        CleanupIntent::CloseDialog if st.clean_view == CleanView::Cleaned && !st.busy() => {
            st.release();
            Ok(())
        }
        CleanupIntent::CloseDialog => Err(NO_DIALOG.into()),
        CleanupIntent::Cancel => st.cancel_job(),
        // The real one starts an editor: the script never sends it, and a replay must not either.
        CleanupIntent::OpenKeepList => Err("not replayed".into()),
    }
}

/// Builder-vs-fixture drift: the static strings of the view the state builds against the fixture
/// state `name` (see `crate::drift`). The nav groups that come from the rules on the machine and the
/// heading (the subtitle counts the rows) are not compared.
fn drift_check(doc: &serde_json::Value, st: &CleanupState, name: &str, out: &mut Vec<String>, seen: &mut std::collections::BTreeSet<String>) {
    let opts = crate::drift::Opts { nav: |id| id.starts_with("builtin:") || id == "winapp2", heading: false, ..crate::drift::Opts::all() };
    out.extend(crate::drift::diff(&format!("cleanup:{name}"), &st.view(false), &crate::drift::fixture_view(doc, "cleanup", name), opts));
    out.extend(crate::drift::view_problems(&format!("real cleanup:{name}"), &serde_json::to_value(st.view(false)).unwrap()));
    seen.insert(name.to_string());
}

/// The check of a row of the list (an item or a category's box) as the script names it.
fn check_of(st: &CleanupState, id: &str) -> &'static str {
    let real = real_id(id);
    match st.view(false).rows.iter().find(|r| r.id == real).unwrap_or_else(|| panic!("no row {id}")).check {
        Check::On => "on",
        Check::Off => "off",
        Check::Mixed => "mixed",
        Check::None => "none",
    }
}

/// Replays the Cleanup steps of `apps/desktop/fixtures/intent-script.json` against the real state,
/// through the worker seam. The script is the spec for the transitions; where the real state cannot
/// say the same thing, the step is re-synchronised on the script's expectation, and each kind of
/// re-synchronisation is counted and pinned at the end (one now):
/// - `copy_checks`: the fixture's `confirm` view is a copy that shows `cat:4` off after the analysed
///     one had it switched on (twice: the refused tick and the refused «None»); the state has one
///     selection, so that check is not compared.
/// Not re-synchronised any more (the script follows Rust): no built-in category is ever blocked, a
/// new analysis over a finished report is accepted, and a cancelled analysis ends `interrupted`.
#[test]
fn replays_the_cleanup_steps_of_the_shared_script() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/fixtures/intent-script.json");
    let script: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut st = script_state("idle");
    let doc = crate::drift::document();
    let (mut drift, mut drift_states) = (Vec::<String>::new(), std::collections::BTreeSet::<String>::new());
    let mut compared = 0;
    let (mut landed, mut finished, mut cancelled_clean, mut cancelled_analysis) = (0, 0, 0, 0);
    let (mut copy_checks, mut interrupted) = (0, 0);
    for step in script["steps"].as_array().unwrap() {
        let note = step["note"].as_str().unwrap_or("");
        match step["op"].as_str().unwrap() {
            "go" if step["section"] == "cleanup" => {
                if st.busy() {
                    st.cancel_job().unwrap();
                    finish(&mut st);
                }
                st = script_state(step["state"].as_str().unwrap());
                if shape(&st) == step["state"].as_str().unwrap() {
                    drift_check(&doc, &st, shape(&st), &mut drift, &mut drift_states);
                }
            }
            "tick" => {
                for pushed in step["pushed"].as_array().unwrap().iter().filter(|p| p["section"] == "cleanup") {
                    let to = pushed["state"].as_str().unwrap();
                    match (shape(&st), to) {
                        ("analyzing", "analyzed") | ("cleaning", "report") => {
                            FINISH.store(true, SeqCst);
                            finish(&mut st);
                            FINISH.store(false, SeqCst);
                            assert_eq!(shape(&st), to, "{note}");
                            if to == "analyzed" { landed += 1 } else { finished += 1 }
                        }
                        _ => {}
                    }
                }
            }
            "intent" if step["section"] == "cleanup" => {
                let intent: CleanupIntent = serde_json::from_value(step["intent"].clone()).unwrap();
                let expect = step["expect"]["state"].as_str().unwrap();
                let was = shape(&st);
                let result = apply(&mut st, intent.clone());
                match (was, &intent, result.is_ok()) {
                    ("cleaning" | "analyzing", CleanupIntent::Cancel, true) => {
                        // The flags are up: the worker answers (a cancelled report, or the analysis so far).
                        finish(&mut st);
                        if was == "cleaning" { cancelled_clean += 1 } else { cancelled_analysis += 1 }
                    }
                    _ => {}
                }
                assert_eq!(shape(&st), expect, "{note}: {intent:?}");
                let v = st.view(false);
                drift_check(&doc, &st, expect, &mut drift, &mut drift_states);
                if expect == "interrupted" {
                    // What was analysed stays, with the error notice and Clean still on.
                    assert_eq!(v.notices.iter().map(|n| n.tone).collect::<Vec<_>>(), vec![Tone::Error], "{note}: the interrupted notice");
                    assert!(v.actions.iter().any(|a| a.id.starts_with("clean") && a.enabled), "{note}: Clean stays enabled");
                    interrupted += 1;
                }
                if let Some(checks) = step["expect"]["checks"].as_object() {
                    for (id, want) in checks {
                        if shape(&st) == "confirm" && id == "cat:4" {
                            copy_checks += 1;
                        } else {
                            assert_eq!(check_of(&st, id), want, "{note}: check of {id}");
                        }
                    }
                }
                for (key, got) in [("dialog", v.dialog.is_some()), ("report", v.report.is_some())] {
                    if let Some(want) = step["expect"][key].as_bool() {
                        assert_eq!(got, want, "{note}: {key}");
                    }
                }
                if step["expect"]["fraction"] == 0 {
                    assert_eq!(v.job.as_ref().map(|j| j.fraction), Some(0.0), "{note}: a job starts at 0");
                }
                compared += 1;
            }
            _ => {}
        }
    }
    assert!(crate::drift::unknown(&drift).is_empty(), "the views the builders make differ from the fixture:\n{}", crate::drift::unknown(&drift).join("\n"));
    for want in ["idle", "analyzing", "analyzed", "confirm", "cleaning", "report", "cancelled", "interrupted"] {
        assert!(drift_states.contains(want), "cleanup:{want} was compared with the fixture (compared: {drift_states:?})");
    }
    assert!(compared >= 25, "most of the script's intents were compared: {compared}");
    assert_eq!((landed, finished, cancelled_clean, cancelled_analysis), (1, 1, 1, 1), "analyses landed, cleanups finished, cleanups cancelled, analyses cancelled");
    assert_eq!(copy_checks, 2, "re-synchronisations");
    assert_eq!(interrupted, 4, "the cancelled analysis and the three steps that stay on it");
}

/// The fixture the TypeScript mock serves for Cleanup has to read through the Rust mirror of
/// `types.ts`, field for field: no missing key and no unknown one, at any depth.
#[test]
fn every_cleanup_fixture_state_reads_through_the_contract() {
    fn objects(v: &serde_json::Value, at: String, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(m) => {
                out.push(at.clone());
                for (k, child) in m {
                    objects(child, format!("{at}/{}", k.replace('~', "~0").replace('/', "~1")), out);
                }
            }
            serde_json::Value::Array(a) => a.iter().enumerate().for_each(|(i, child)| objects(child, format!("{at}/{i}"), out)),
            _ => {}
        }
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/fixtures/views.json");
    let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let states = doc["views"]["cleanup"].as_object().unwrap();
    let names: Vec<&str> = states.keys().map(String::as_str).collect();
    for want in ["idle", "analyzing", "analyzed", "confirm", "cleaning", "report", "cancelled"] {
        assert!(names.contains(&want), "cleanup:{want} is in the fixture");
    }
    let mut probes = 0;
    for (state, view) in states {
        let read = |v: &serde_json::Value| serde_json::from_value::<SectionView>(v.clone());
        let parsed = read(view).unwrap_or_else(|e| panic!("cleanup:{state}: {e}"));
        assert_eq!(parsed.section, SectionId::Cleanup);
        let mut at = Vec::new();
        objects(view, String::new(), &mut at);
        for ptr in at {
            let object = view.pointer(&ptr).unwrap().as_object().unwrap();
            for key in object.keys() {
                let mut broken = view.clone();
                broken.pointer_mut(&ptr).unwrap().as_object_mut().unwrap().remove(key);
                assert!(read(&broken).is_err(), "cleanup:{state}: `{ptr}` without `{key}` must not read");
                probes += 1;
            }
            let mut extra = view.clone();
            extra.pointer_mut(&ptr).unwrap().as_object_mut().unwrap().insert("bogus".into(), serde_json::json!(1));
            assert!(read(&extra).is_err(), "cleanup:{state}: `{ptr}` with an unknown key must not read");
            probes += 1;
        }
    }
    assert!(probes > 500, "{probes} probes");
}
