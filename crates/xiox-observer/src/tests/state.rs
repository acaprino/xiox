use super::fixtures::{GB, MB, build, item, scan, scan_at, worktree};
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
    // New items are measured again at every check: they may still be growing.
    assert_eq!(o.keys_to_measure(), vec![known.key.clone(), fresh.key.clone()]);

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

#[test]
fn a_new_item_measured_small_then_big_makes_growth_and_fires_attention() {
    let mut o = Observer::new(scan(vec![]), params(GB, usize::MAX), T0);
    // First seen mid-compile: 10 MB.
    let events = o.update(scan(vec![build("target", Some(10 * MB))]), T0 + 1);
    assert!(events.iter().all(|e| matches!(e, Event::Changed { .. })), "{events:?}");
    assert_eq!(o.keys_to_measure(), vec![key_of("target")], "still to be measured at the next check");
    // The next check measures it again: 3 GB.
    let events = o.update(scan(vec![build("target", Some(3 * GB))]), T0 + 2);
    assert_eq!(o.report(false).growth_bytes, (3 * GB) as i64);
    let [Event::Attention { kind: AttentionKind::Growth, value, .. }] = &events[..] else { panic!("{events:?}") };
    assert_eq!(*value, 3 * GB);
}

#[test]
fn growth_counts_a_new_item_inside_another_new_item_once() {
    let mut o = Observer::new(scan(vec![]), quiet(), T0);
    let wt = item(Category::Worktrees, r"D:\Projects\wt", Some(3 * GB), Level::Optional, vec![]);
    let inner = item(Category::Builds, r"D:\Projects\wt\target", Some(GB), Level::Recommended, vec![]);
    o.update(scan(vec![wt, inner]), T0 + 1);
    assert_eq!(o.report(false).growth_bytes, (3 * GB) as i64);

    // A new folder inside a baseline one is new growth, in full; a grown baseline folder inside a
    // baseline one that was not measured again counts its growth.
    let base_wt = item(Category::Worktrees, r"D:\Projects\old", Some(5 * GB), Level::Optional, vec![]);
    let base_inner = item(Category::Builds, r"D:\Projects\old\target", Some(GB), Level::Recommended, vec![]);
    let mut o = Observer::new(scan(vec![base_wt.clone(), base_inner]), quiet(), T0);
    let grown = item(Category::Builds, r"D:\Projects\old\target", Some(2 * GB), Level::Recommended, vec![]);
    let fresh = item(Category::Builds, r"D:\Projects\old\dist", Some(GB), Level::Recommended, vec![]);
    o.update(scan(vec![base_wt, grown, fresh]), T0 + 1);
    assert_eq!(o.report(false).growth_bytes, (2 * GB) as i64);
}

#[test]
fn totals_count_nested_items_once() {
    let wt = item(Category::Worktrees, r"D:\Projects\wt", Some(3 * GB), Level::Recommended, vec![]);
    let inner = item(Category::Builds, r"D:\Projects\wt\target", Some(GB), Level::Recommended, vec![]);
    let outer_build = build("target", Some(2 * GB));
    let nested_build = build(r"target\debug", Some(GB));
    let o = Observer::new(scan(vec![wt, inner, outer_build, nested_build]), quiet(), T0);
    let Event::Started { totals, .. } = o.started() else { panic!() };
    // Recommended: the worktree (3 GB, holds its build) and the outer build (2 GB, holds its subfolder).
    assert_eq!(totals.recommended_bytes, 5 * GB);
    // Per category: the nested build is inside a build of the same category, not counted twice there.
    assert_eq!(totals.by_category.get("builds"), Some(&(3 * GB)));
    assert_eq!(totals.by_category.get("worktrees"), Some(&(3 * GB)));
}

#[test]
fn report_carries_the_shape_of_analyze_json_from_the_latest_scan() {
    let mut first = scan(vec![build("a", Some(GB))]);
    first.scanned_at_ms = 111;
    first.elapsed_ms = 7;
    first.branches = vec![xiox_project::StaleBranch { name: "old".into(), merged: true, worktree_missing: false }];
    let mut o = Observer::new(first, quiet(), T0);
    let j = serde_json::to_value(o.report(false)).unwrap();
    assert_eq!(j["branches"][0]["name"], "old");
    assert_eq!(j["search"], "bounded-walk");
    assert_eq!((j["scannedAtMs"].as_i64(), j["elapsedMs"].as_u64()), (Some(111), Some(7)));

    let mut next = scan(vec![build("a", Some(GB))]);
    next.scanned_at_ms = 222;
    next.elapsed_ms = 9;
    next.search = xiox_project::SearchMode::ParentOnly;
    o.update(next, T0 + 1);
    let j = serde_json::to_value(o.report(false)).unwrap();
    assert_eq!(j["branches"], serde_json::json!([]));
    assert_eq!(j["search"], "parent-only");
    assert_eq!((j["scannedAtMs"].as_i64(), j["elapsedMs"].as_u64()), (Some(222), Some(9)));
}

#[test]
fn a_repeated_key_in_the_first_scan_keeps_the_first_item_as_baseline() {
    let mut o = Observer::new(scan(vec![build("a", Some(GB)), build("a", Some(5 * GB))]), quiet(), T0);
    assert_eq!(o.report(false).items.len(), 1);
    o.update(scan(vec![build("a", Some(2 * GB))]), T0 + 1);
    assert_eq!(o.report(false).growth_bytes, GB as i64, "baseline is the first item's 1 GB");
}

#[test]
fn clean_command_quotes_for_the_windows_argument_parser() {
    let mut o = Observer::new(scan_at(r"D:\My Projects\mukade", vec![]), quiet(), T0);
    let spaced = item(Category::Builds, r"D:\My Projects\mukade\my target", Some(MB), Level::Recommended, vec![]);
    o.update(scan_at(r"D:\My Projects\mukade", vec![spaced]), T0 + 1);
    assert_eq!(
        o.report(false).clean_command.as_deref(),
        Some(r#"xiox clean "D:\My Projects\mukade" --include "D:\My Projects\mukade\my target""#)
    );

    // A root or path ending in a backslash: the backslash must not swallow the closing quote
    // (backslashes before a closing quote are doubled).
    let mut o = Observer::new(scan_at(r"D:\", vec![]), quiet(), T0);
    let trailing = item(Category::Builds, r"D:\a\b\", Some(MB), Level::Recommended, vec![]);
    o.update(scan_at(r"D:\", vec![trailing]), T0 + 1);
    let cmd = o.report(false).clean_command.unwrap();
    assert_eq!(cmd, r#"xiox clean "D:\\" --include "D:\a\b\\""#);
}

#[test]
fn clean_command_leaves_out_an_item_that_holds_a_blocked_item() {
    let mut o = Observer::new(scan(vec![]), quiet(), T0);
    let outer = build("outer", Some(MB));
    let blocked_inner = item(
        Category::Worktrees,
        r"D:\Projects\mukade\outer\wt",
        None,
        Level::Optional,
        vec![Block::DirtyGit],
    );
    let fine = build("fine", Some(MB));
    o.update(scan(vec![outer, blocked_inner, fine.clone()]), T0 + 1);
    let cmd = o.report(false).clean_command.unwrap();
    assert_eq!(cmd.matches("--include").count(), 1, "{cmd}");
    assert!(cmd.contains(&path_of(&fine)) && !cmd.contains("outer"), "{cmd}");

    // Only the one that holds a blocked item: no command at all.
    let mut o = Observer::new(scan(vec![]), quiet(), T0);
    let inner = item(Category::Builds, r"D:\Projects\mukade\outer\x", None, Level::Optional, vec![Block::Unreachable]);
    o.update(scan(vec![build("outer", Some(MB)), inner]), T0 + 1);
    assert_eq!(o.report(false).clean_command, None);
}

/// The same folder (same key) as a worktree: what a check sees once git has registered it.
fn as_worktree(copy: &xiox_project::Item) -> xiox_project::Item {
    let mut wt = item(
        Category::Worktrees,
        &copy.path.as_ref().unwrap().display().to_string(),
        copy.bytes,
        Level::Recommended,
        vec![],
    );
    wt.key = copy.key.clone();
    wt
}

fn as_copy(path: &str) -> xiox_project::Item {
    item(Category::Copies, path, Some(MB), Level::Caution, vec![])
}

#[test]
fn a_category_change_of_the_same_key_is_added_again_in_its_new_category() {
    let copy = as_copy(r"D:\Projects\mukade-spike");
    let mut o = Observer::new(scan(vec![build("target", Some(GB))]), quiet(), T0);
    let events = o.update(scan(vec![build("target", Some(GB)), copy.clone()]), T0 + 1);
    let Event::Changed { added, .. } = &events[0] else { panic!("{events:?}") };
    assert_eq!(added[0].item.category, Category::Copies);

    let wt = as_worktree(&copy);
    let events = o.update(scan(vec![build("target", Some(GB)), wt.clone()]), T0 + 2);
    assert_eq!(events.len(), 1, "{events:?}");
    let Event::Changed { added, removed, .. } = &events[0] else { panic!("{events:?}") };
    assert!(removed.is_empty(), "{removed:?}");
    assert_eq!(added.len(), 1, "{added:?}");
    assert_eq!(added[0].item.key, copy.key);
    assert_eq!(added[0].item.category, Category::Worktrees);
    assert!(added[0].new);
    assert_eq!(added[0].first_seen_ms, T0 + 1, "first seen stays when it was first seen");
    // The report agrees, and nothing more is said when nothing changes any more.
    let r = o.report(false);
    assert_eq!(r.items.iter().find(|i| i.item.key == copy.key).unwrap().item.category, Category::Worktrees);
    assert!(o.update(scan(vec![build("target", Some(GB)), wt]), T0 + 3).is_empty());
}

#[test]
fn a_baseline_item_that_changes_category_is_added_again_and_stays_not_new() {
    let copy = as_copy(r"D:\Projects\mukade-old");
    let mut o = Observer::new(scan(vec![copy.clone()]), quiet(), T0);
    let events = o.update(scan(vec![as_worktree(&copy)]), T0 + 1);
    let Event::Changed { added, removed, .. } = &events[0] else { panic!("{events:?}") };
    assert!(removed.is_empty());
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].item.category, Category::Worktrees);
    assert!(!added[0].new);
    assert_eq!(added[0].first_seen_ms, T0);
}

#[test]
fn a_new_copy_that_becomes_a_worktree_counts_for_the_worktree_threshold_and_the_clean_command() {
    let copy = as_copy(r"D:\Projects\mukade-spike");
    let mut o = Observer::new(scan(vec![]), params(u64::MAX, 1), T0);
    let events = o.update(scan(vec![copy.clone(), worktree("other")]), T0 + 1);
    assert!(events.iter().all(|e| matches!(e, Event::Changed { .. })), "one new worktree is not more than one: {events:?}");
    assert_eq!(o.report(false).clean_command, None, "a copy is caution: not in the command");

    let events = o.update(scan(vec![as_worktree(&copy), worktree("other")]), T0 + 2);
    let Event::Attention { kind, value, threshold, clean_command } = events.last().unwrap() else { panic!("{events:?}") };
    assert_eq!((*kind, *value, *threshold), (AttentionKind::Worktrees, 2, 1));
    let command = clean_command.as_deref().expect("the recommended worktree is in the command");
    assert!(command.contains("mukade-spike") && !command.contains("other"), "{command}");
    assert_eq!(events.iter().filter(|e| matches!(e, Event::Attention { .. })).count(), 1, "{events:?}");
    assert_eq!(o.report(false).attention, vec![AttentionKind::Worktrees]);
}

#[test]
fn a_later_measure_that_gives_nothing_keeps_the_last_bytes_and_their_time() {
    // A build folder that outgrows the measuring caps after the start: the check measures it
    // again and gets `None`. The report keeps the last measured value (and when it was measured).
    let mut o = Observer::new(scan(vec![build("target", Some(GB))]), quiet(), T0);
    o.update(scan(vec![build("target", Some(GB + 10 * MB))]), T0 + 10);
    o.update(scan(vec![build("target", None)]), T0 + 20);
    let r = o.report(false);
    assert_eq!(r.items[0].item.bytes, Some(GB + 10 * MB));
    assert_eq!(r.items[0].measured_at_ms, Some(T0 + 10));
    assert_eq!(r.growth_bytes, (10 * MB) as i64);
}
