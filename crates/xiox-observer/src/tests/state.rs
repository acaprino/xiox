use super::fixtures::{GB, MB, build, item, scan, worktree};
use crate::{AttentionKind, Event, Observer, Params};
use xiox_model::Level;
use xiox_project::{Block, Category, SCHEMA};

const T0: i64 = 1_000;

fn params(grow: u64, worktrees: usize) -> Params {
    Params { grow_threshold: grow, worktree_threshold: worktrees }
}

/// Thresholds that never fire, for the tests that are about something else.
fn quiet() -> Params {
    params(u64::MAX, usize::MAX)
}

fn key_of(name: &str) -> String {
    build(name, None).key
}

fn path_of(i: &xiox_project::Item) -> String {
    i.path.as_ref().unwrap().display().to_string()
}

#[test]
fn baseline_items_are_never_new_even_when_they_grow() {
    let mut o = Observer::new(scan(vec![build("target", Some(GB))]), quiet(), T0);
    assert!(o.report(false).items.iter().all(|i| !i.new));
    let events = o.update(scan(vec![build("target", Some(9 * GB))]), T0 + 10);
    assert!(events.is_empty(), "{events:?}");
    let r = o.report(false);
    assert_eq!(r.items.len(), 1);
    assert!(!r.items[0].new);
    assert_eq!(r.items[0].item.bytes, Some(9 * GB));
    assert_eq!(r.items[0].first_seen_ms, T0);
}

#[test]
fn a_new_item_is_added_with_first_seen_and_new_true() {
    let base = build("target", Some(GB));
    let extra = build("dist", Some(10 * MB));
    let mut o = Observer::new(scan(vec![base.clone()]), quiet(), T0);
    let events = o.update(scan(vec![base, extra.clone()]), T0 + 500);
    assert_eq!(events.len(), 1, "{events:?}");
    let Event::Changed { added, removed, growth_bytes } = &events[0] else { panic!("{events:?}") };
    assert!(removed.is_empty());
    assert_eq!(*growth_bytes, (10 * MB) as i64);
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].item, extra);
    assert!(added[0].new);
    assert_eq!(added[0].first_seen_ms, T0 + 500);
    assert_eq!(added[0].measured_at_ms, Some(T0 + 500));
    let r = o.report(false);
    assert_eq!(r.items.len(), 2);
    assert!(!r.items.iter().find(|i| i.item.key == key_of("target")).unwrap().new);
    assert!(r.items.iter().find(|i| i.item.key == extra.key).unwrap().new);
}

#[test]
fn a_baseline_item_that_disappears_is_removed_and_when_back_is_added_not_new() {
    let a = build("target", Some(GB));
    let b = build("dist", Some(MB));
    let mut o = Observer::new(scan(vec![a.clone(), b.clone()]), quiet(), T0);

    let events = o.update(scan(vec![b.clone()]), T0 + 100);
    assert_eq!(events.len(), 1, "{events:?}");
    let Event::Changed { added, removed, .. } = &events[0] else { panic!("{events:?}") };
    assert!(added.is_empty());
    assert_eq!(removed, &vec![a.key.clone()]);
    // Reported once: the same scan again says nothing.
    assert!(o.update(scan(vec![b.clone()]), T0 + 200).is_empty());

    let events = o.update(scan(vec![a.clone(), b]), T0 + 300);
    assert_eq!(events.len(), 1, "{events:?}");
    let Event::Changed { added, removed, .. } = &events[0] else { panic!("{events:?}") };
    assert!(removed.is_empty());
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].item.key, a.key);
    assert!(!added[0].new, "it was in the baseline");
    assert_eq!(added[0].first_seen_ms, T0);
}

#[test]
fn a_new_item_that_disappears_and_comes_back_is_new_again() {
    let a = build("target", Some(GB));
    let x = build("x", Some(MB));
    let mut o = Observer::new(scan(vec![a.clone()]), quiet(), T0);
    o.update(scan(vec![a.clone(), x.clone()]), T0 + 100);
    o.update(scan(vec![a.clone()]), T0 + 200);
    let events = o.update(scan(vec![a, x]), T0 + 300);
    let Event::Changed { added, .. } = &events[0] else { panic!("{events:?}") };
    assert!(added[0].new);
    assert_eq!(added[0].first_seen_ms, T0 + 300);
}

#[test]
fn no_change_gives_no_events() {
    let s = scan(vec![build("target", Some(GB)), worktree("mukade-wt")]);
    let mut o = Observer::new(s.clone(), Params::default(), T0);
    assert!(o.update(s.clone(), T0 + 1).is_empty());
    assert!(o.update(s, T0 + 2).is_empty());
}

#[test]
fn growth_counts_new_items_and_growth_of_baseline_items_but_not_shrinking() {
    let a = build("a", Some(GB));
    let b = build("b", Some(3 * GB));
    let mut o = Observer::new(scan(vec![a, b]), quiet(), T0);
    assert_eq!(o.report(false).growth_bytes, 0);
    let events = o.update(
        scan(vec![build("a", Some(2 * GB)), build("b", Some(GB)), build("c", Some(500 * MB))]),
        T0 + 1,
    );
    // a grew by 1 GB, b shrank (counts 0), c is new (500 MB).
    let expected = (GB + 500 * MB) as i64;
    assert_eq!(o.report(false).growth_bytes, expected);
    let Event::Changed { growth_bytes, .. } = &events[0] else { panic!("{events:?}") };
    assert_eq!(*growth_bytes, expected);
}

#[test]
fn growth_attention_fires_once_then_rearms_after_dropping_below() {
    let mut o = Observer::new(scan(vec![build("a", Some(GB))]), params(GB, usize::MAX), T0);
    let grown = |bytes| scan(vec![build("a", Some(bytes))]);

    // 3 GB - 1 GB = 2 GB of growth > 1 GB: fires with the value and the threshold.
    let events = o.update(grown(3 * GB), T0 + 1);
    assert_eq!(events.len(), 1, "{events:?}");
    let Event::Attention { kind, value, threshold, .. } = &events[0] else { panic!("{events:?}") };
    assert_eq!((*kind, *value, *threshold), (AttentionKind::Growth, 2 * GB, GB));
    assert_eq!(o.report(false).attention, vec![AttentionKind::Growth]);

    // Still above: nothing more.
    assert!(o.update(grown(4 * GB), T0 + 2).is_empty());
    // Exactly at the threshold is "not exceeded": re-armed, silently.
    assert!(o.update(grown(2 * GB), T0 + 3).is_empty());
    assert!(o.report(false).attention.is_empty());
    // Crossing again fires again.
    let events = o.update(grown(5 * GB), T0 + 4);
    assert!(
        matches!(&events[..], [Event::Attention { kind: AttentionKind::Growth, value, .. }] if *value == 4 * GB),
        "{events:?}"
    );
}

#[test]
fn worktree_attention_counts_only_worktrees_not_in_the_baseline() {
    let baseline: Vec<_> = (0..5).map(|n| worktree(&format!("old-{n}"))).collect();
    let mut o = Observer::new(scan(baseline.clone()), params(u64::MAX, 3), T0);
    assert!(o.update(scan(baseline.clone()), T0 + 1).is_empty());

    let with_new = |n: usize| {
        let mut items = baseline.clone();
        items.extend((0..n).map(|i| worktree(&format!("new-{i}"))));
        scan(items)
    };
    // Three new ones are not more than three.
    let events = o.update(with_new(3), T0 + 2);
    assert!(events.iter().all(|e| matches!(e, Event::Changed { .. })), "{events:?}");
    assert!(o.report(false).attention.is_empty());
    // The fourth is.
    let events = o.update(with_new(4), T0 + 3);
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(matches!(events[0], Event::Changed { .. }));
    let Event::Attention { kind, value, threshold, .. } = &events[1] else { panic!("{events:?}") };
    assert_eq!((*kind, *value, *threshold), (AttentionKind::Worktrees, 4, 3));
    assert_eq!(o.report(false).attention, vec![AttentionKind::Worktrees]);
    // A fifth does not fire again; going back to three re-arms.
    let events = o.update(with_new(5), T0 + 4);
    assert!(events.iter().all(|e| matches!(e, Event::Changed { .. })), "{events:?}");
    o.update(with_new(3), T0 + 5);
    let events = o.update(with_new(4), T0 + 6);
    assert!(events.iter().any(|e| matches!(e, Event::Attention { kind: AttentionKind::Worktrees, .. })), "{events:?}");
}

#[test]
fn unmeasured_new_item_is_listed_for_measuring_and_counts_when_measured() {
    let known = build("target", Some(GB));
    let wt = worktree("old-wt");
    let mut o = Observer::new(scan(vec![known.clone(), wt.clone()]), quiet(), T0);
    // Only the known `builds` folder is re-measured, never a baseline worktree.
    assert_eq!(o.keys_to_measure(), vec![known.key.clone()]);

    let fresh = build("fresh", None);
    o.update(scan(vec![known.clone(), wt.clone(), fresh.clone()]), T0 + 1);
    let mut keys = o.keys_to_measure();
    keys.sort();
    let mut expected = vec![known.key.clone(), fresh.key.clone()];
    expected.sort();
    assert_eq!(keys, expected);
    assert_eq!(o.report(false).growth_bytes, 0, "not measured yet");
    assert_eq!(o.report(false).items.iter().find(|i| i.item.key == fresh.key).unwrap().measured_at_ms, None);

    // The caller measured it (and the known folder, which did not change).
    o.update(scan(vec![known.clone(), wt.clone(), build("fresh", Some(3 * GB))]), T0 + 2);
    assert_eq!(o.report(false).growth_bytes, (3 * GB) as i64);
    let fresh_view = o.report(false).items.into_iter().find(|i| i.item.key == fresh.key).unwrap();
    assert_eq!(fresh_view.measured_at_ms, Some(T0 + 2));
    assert_eq!(o.keys_to_measure(), vec![known.key.clone()], "measured: not asked again");

    // A light re-check brings no bytes: the last measurement stays.
    o.update(scan(vec![build("target", None), wt, build("fresh", None)]), T0 + 3);
    assert_eq!(o.report(false).growth_bytes, (3 * GB) as i64);
    let fresh_view = o.report(false).items.into_iter().find(|i| i.item.key == fresh.key).unwrap();
    assert_eq!((fresh_view.item.bytes, fresh_view.measured_at_ms), (Some(3 * GB), Some(T0 + 2)));
}

#[test]
fn a_baseline_build_first_measured_later_takes_that_as_its_baseline() {
    let mut o = Observer::new(scan(vec![build("target", None)]), quiet(), T0);
    assert_eq!(o.keys_to_measure().len(), 1);
    o.update(scan(vec![build("target", Some(2 * GB))]), T0 + 1);
    assert_eq!(o.report(false).growth_bytes, 0);
    o.update(scan(vec![build("target", Some(3 * GB))]), T0 + 2);
    assert_eq!(o.report(false).growth_bytes, GB as i64);
}

#[test]
fn report_marks_new_items_and_builds_clean_command_from_new_recommended_unblocked_items() {
    let old = build("target", Some(GB));
    let mut o = Observer::new(scan(vec![old.clone()]), quiet(), T0);
    assert_eq!(o.report(false).clean_command, None, "nothing new yet");

    let new_rec = build("dist", Some(MB));
    let new_opt = item(Category::Builds, r"D:\Projects\mukade\cache", Some(MB), Level::Optional, vec![]);
    let new_blocked = item(Category::Worktrees, r"D:\Projects\wt", None, Level::Recommended, vec![Block::DirtyGit]);
    o.update(scan(vec![old.clone(), new_rec.clone(), new_opt.clone(), new_blocked.clone()]), T0 + 1);

    let r = o.report(false);
    let new: Vec<&str> = r.items.iter().filter(|i| i.new).map(|i| i.item.key.as_str()).collect();
    assert_eq!(new, vec![new_rec.key.as_str(), new_opt.key.as_str(), new_blocked.key.as_str()]);
    assert_eq!(
        r.clean_command.as_deref(),
        Some(format!(r#"xiox clean "D:\Projects\mukade" --include "{}""#, path_of(&new_rec)).as_str())
    );
    assert!(!r.clean_command_truncated);
    assert_eq!(r.schema, SCHEMA);
    assert_eq!((r.started_at_ms, r.checked_at_ms), (T0, T0 + 1));
    assert_eq!(r.totals.items, 4);
}

#[test]
fn clean_command_skips_blocked_and_too_shallow_items_and_truncates_at_25() {
    let mut o = Observer::new(scan(vec![]), quiet(), T0);
    let mut items: Vec<_> = (0..30).map(|n| build(&format!("b{n:02}"), Some(MB))).collect();
    // Directly under the drive: `clean` would refuse it.
    items.push(item(Category::Logs, r"D:\app.log", Some(MB), Level::Recommended, vec![]));
    items.push(item(
        Category::Builds,
        r"D:\Projects\mukade\blocked",
        Some(MB),
        Level::Recommended,
        vec![Block::DirtyGit],
    ));
    o.update(scan(items), T0 + 1);

    let r = o.report(false);
    let cmd = r.clean_command.unwrap();
    assert!(cmd.starts_with(r#"xiox clean "D:\Projects\mukade" --include "#), "{cmd}");
    assert_eq!(cmd.matches("--include").count(), 25, "{cmd}");
    assert!(r.clean_command_truncated);
    assert!(cmd.contains("b00") && cmd.contains("b24") && !cmd.contains("b25"), "{cmd}");
    assert!(!cmd.contains("app.log") && !cmd.contains("blocked"), "{cmd}");

    // Exactly 25 fit.
    let mut o = Observer::new(scan(vec![]), quiet(), T0);
    o.update(scan((0..25).map(|n| build(&format!("b{n:02}"), None)).collect()), T0 + 1);
    let r = o.report(false);
    assert_eq!(r.clean_command.unwrap().matches("--include").count(), 25);
    assert!(!r.clean_command_truncated);
}

#[test]
fn report_stale_flag_is_passed_through() {
    let o = Observer::new(scan(vec![build("target", Some(GB))]), quiet(), T0);
    assert!(!o.report(false).stale);
    assert!(o.report(true).stale);
}

#[test]
fn totals_sum_measured_bytes_by_category_and_recommended() {
    let items = vec![
        build("a", Some(GB)),
        build("b", Some(MB)),
        build("unmeasured", None),
        item(Category::Builds, r"D:\Projects\mukade\opt", Some(4 * MB), Level::Optional, vec![]),
        worktree("wt"),
    ];
    let o = Observer::new(scan(items), quiet(), T0);
    let Event::Started { totals, .. } = o.started() else { panic!() };
    assert_eq!(totals.items, 5);
    assert_eq!(totals.by_category.get("builds"), Some(&(GB + 5 * MB)));
    assert!(!totals.by_category.contains_key("worktrees"), "nothing measured there: {:?}", totals.by_category);
    assert_eq!(totals.recommended_bytes, GB + MB);
}

#[test]
fn events_serialize_with_kebab_case_event_tag() {
    let mut o = Observer::new(scan(vec![build("a", Some(GB))]), params(GB, 3), T0);

    let started = serde_json::to_value(o.started()).unwrap();
    assert_eq!(started["event"], "started");
    assert_eq!(started["project"]["name"], "mukade");
    assert_eq!(started["params"]["growThreshold"], GB);
    assert_eq!(started["params"]["worktreeThreshold"], 3);
    assert_eq!(started["totals"]["byCategory"]["builds"], GB);
    assert_eq!(started["totals"]["recommendedBytes"], GB);
    assert_eq!(started["totals"]["items"], 1);

    let events = o.update(scan(vec![build("a", Some(3 * GB)), build("n", Some(MB))]), T0 + 5);
    let changed = serde_json::to_value(&events[0]).unwrap();
    assert_eq!(changed["event"], "changed");
    assert_eq!(changed["growthBytes"], 2 * GB + MB);
    assert_eq!(changed["removed"], serde_json::json!([]));
    let added = &changed["added"][0];
    assert_eq!(added["key"], key_of("n"));
    assert_eq!(added["category"], "builds");
    assert_eq!(added["new"], true);
    assert_eq!(added["firstSeenMs"], T0 + 5);
    assert_eq!(added["measuredAtMs"], T0 + 5);

    let attention = serde_json::to_value(&events[1]).unwrap();
    assert_eq!(attention["event"], "attention");
    assert_eq!(attention["kind"], "growth");
    assert_eq!(attention["value"], 2 * GB + MB);
    assert_eq!(attention["threshold"], GB);
    assert!(attention["cleanCommand"].is_string(), "{attention}");

    let report = serde_json::to_value(Event::Report(o.report(true))).unwrap();
    assert_eq!(report["event"], "report");
    assert_eq!(report["schema"], SCHEMA);
    assert_eq!(report["stale"], true);
    assert_eq!(report["attention"], serde_json::json!(["growth"]));
    assert_eq!(report["cleanCommandTruncated"], false);
    assert_eq!(report["startedAtMs"], T0);
    assert_eq!(report["checkedAtMs"], T0 + 5);
    assert_eq!(report["items"].as_array().unwrap().len(), 2);

    let error = serde_json::to_value(Event::Error { message: "boom".into() }).unwrap();
    assert_eq!(error, serde_json::json!({"event": "error", "message": "boom"}));
}
