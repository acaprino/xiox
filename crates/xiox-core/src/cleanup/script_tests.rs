//! The Cleanup steps of the shared script (`apps/desktop/fixtures/intent-script.json`) replayed against
//! the real state through the worker seam, and the Cleanup states of the shared fixture read through the
//! Rust view types. The state and its helpers are those of `intent_tests`.

use super::intent_tests::{finish, state_with};
use super::*;
use crate::view::testing::notifier;
use crate::view::*;
use std::sync::atomic::Ordering::SeqCst;
use std::time::Duration;

/// The script's names for the nav ids of the fixture. The mock's list has one id per fixture group
/// (`cat:system`...); the real categories come from `cleaner::CATEGORIES` and the rule groups, so the
/// script's ids stand for the Rust ones here (an id the script has but the nav has not stays as it is).
fn nav_alias(id: &str) -> &str {
    match id {
        "cat:system" => "builtin:Sistema",
        "cat:windows" => "rules:Windows",
        "cat:browser" => "rules:Sviluppo e AI",
        other => other,
    }
}

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

/// The state the script calls `name` (`go`): its own selection, the first four categories.
fn script_state(name: &str) -> CleanupState {
    let mut st = state_with(CleanupWorkers { job: script_job });
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

/// The script's name for what a state looks like.
fn shape(st: &CleanupState) -> &'static str {
    if st.busy() {
        return if st.clean_view == CleanView::Cleaned { "cleaning" } else { "analyzing" };
    }
    if st.confirm_clean {
        return "confirm";
    }
    match st.clean_view {
        CleanView::Empty => "idle",
        CleanView::Analyzed => "analyzed",
        CleanView::Cleaned if st.clean_report.as_ref().is_some_and(|r| r.cancelled) => "cancelled",
        CleanView::Cleaned => "report",
    }
}

/// What the page sends for each intent, and what `CleanupState` is asked to do with it. Over a
/// report the page's «Close» is `release()`.
fn apply(st: &mut CleanupState, intent: CleanupIntent) -> Result<(), String> {
    match intent {
        CleanupIntent::Select { category } => st.select_category(nav_alias(&category)),
        CleanupIntent::Toggle { row } => st.toggle(&row),
        CleanupIntent::SetOnlyOld { on } => st.set_only_old(on),
        CleanupIntent::Search { text } => {
            st.set_search(&text);
            Ok(())
        }
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

/// The check of a category row as the script names it (the list lives in `clean_sel`, whatever
/// category is open).
fn check_of(st: &CleanupState, id: &str) -> &'static str {
    let i: usize = id.strip_prefix("cat:").unwrap().parse().unwrap();
    if st.clean_sel[i] { "on" } else { "off" }
}

/// Replays the Cleanup steps of `apps/desktop/fixtures/intent-script.json` against the real state,
/// through the worker seam. The script is the spec for the transitions; where the real state cannot
/// say the same thing, the step is re-synchronised on the script's expectation, and each kind of
/// re-synchronisation is counted and pinned at the end:
/// - `blocked_accepted`: the script's fixture blocks `cat:6`; no built-in category is ever blocked
///   (egui has no such state), so Rust toggles it (and the replay toggles it back);
/// - `copy_checks`: the fixture's `confirm` view is a copy that shows `cat:4` off after the analysed
///   one had it switched on; the state has one selection, so that check is not compared;
/// - `analyze_over_report`: the script refuses a new analysis over a finished report (its fixture
///   report has Analyze off); Rust accepts it (egui does: the button is on when nothing runs), so the
///   replay answers by rebuilding the report state.
#[test]
fn replays_the_cleanup_steps_of_the_shared_script() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/fixtures/intent-script.json");
    let script: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut st = script_state("idle");
    let mut compared = 0;
    let (mut landed, mut finished, mut cancelled_clean, mut cancelled_analysis) = (0, 0, 0, 0);
    let (mut blocked_accepted, mut copy_checks, mut analyze_over_report) = (0, 0, 0);
    for step in script["steps"].as_array().unwrap() {
        let note = step["note"].as_str().unwrap_or("");
        match step["op"].as_str().unwrap() {
            "go" if step["section"] == "cleanup" => {
                // The open category is the page's memory: it survives a jump to another state.
                let nav = st.nav.clone();
                st = script_state(step["state"].as_str().unwrap());
                st.nav = nav;
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
                    ("analyzed", CleanupIntent::Toggle { row }, true) if row == "cat:6" => {
                        st.toggle("cat:6").unwrap();
                        blocked_accepted += 1;
                    }
                    ("report", CleanupIntent::Analyze, true) => {
                        assert_eq!(shape(&st), "analyzing", "{note}");
                        st.cancel_job().unwrap();
                        finish(&mut st);
                        let nav = st.nav.clone();
                        st = script_state("report");
                        st.nav = nav;
                        analyze_over_report += 1;
                    }
                    _ => {}
                }
                assert_eq!(shape(&st), expect, "{note}: {intent:?}");
                let v = st.view(false);
                if let Some(checks) = step["expect"]["checks"].as_object() {
                    for (id, want) in checks {
                        if shape(&st) == "confirm" && id == "cat:4" {
                            copy_checks += 1;
                        } else {
                            assert_eq!(check_of(&st, id), want, "{note}: check of {id}");
                        }
                    }
                }
                if let Some(active) = step["expect"]["active"].as_str() {
                    assert!(v.nav.iter().any(|n| n.id == nav_alias(active) && n.active), "{note}: active category {active}");
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
    assert!(compared >= 25, "most of the script's intents were compared: {compared}");
    assert_eq!((landed, finished, cancelled_clean, cancelled_analysis), (1, 1, 1, 1), "analyses landed, cleanups finished, cleanups cancelled, analyses cancelled");
    assert_eq!((blocked_accepted, copy_checks, analyze_over_report), (1, 1, 1), "re-synchronisations");
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
