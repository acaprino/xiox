//! `--json`: the scan (`analyze`) or the scan and the outcome (`clean`) as one JSON object (schema 2).

use xiox_project::{Chosen, CleanOutcome, ProjectScan, SCHEMA};

/// The `ProjectScan` serialization: `{"schema":2,"project":…,"items":[…],…}`. Field names are
/// camelCase, enum values kebab-case; every text in it is English.
///
/// `Err` when a path is not valid Unicode (JSON cannot carry it).
pub fn scan_json(scan: &ProjectScan) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::to_value(scan)
}

/// The result of `clean --json`:
/// `{"schema":2,"scan":<scan_json>,"chosen":<Chosen>|null,"outcome":<CleanOutcome>|null}`.
/// `chosen` is what the selection took, with the `danger` plans confirmed and pending; `outcome`
/// is `null` when nothing was run, and its items are `OutcomeItem` objects
/// (`{"key","path","category","method","outcome":{"type":"removed"|"skipped"|"failed",…}}`).
///
/// `Err` when a path is not valid Unicode (JSON cannot carry it).
pub fn outcome_json(
    scan: &ProjectScan,
    chosen: Option<&Chosen>,
    outcome: Option<&CleanOutcome>,
) -> Result<serde_json::Value, serde_json::Error> {
    // `to_value` (not `json!`, which panics) so that a path JSON cannot carry is an `Err`.
    use serde_json::{Map, Value, to_value};
    let mut m = Map::new();
    m.insert("schema".into(), to_value(SCHEMA)?);
    m.insert("scan".into(), scan_json(scan)?);
    m.insert("chosen".into(), to_value(chosen)?);
    m.insert("outcome".into(), to_value(outcome)?);
    Ok(Value::Object(m))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdata::*;
    use xiox_model::RemoveMode;
    use xiox_project::Level;
    use xiox_project::{Category, Outcome};
    use std::path::PathBuf;

    fn outcome_of_one(path: Option<PathBuf>, o: Outcome) -> CleanOutcome {
        CleanOutcome {
            schema: SCHEMA,
            mode: RemoveMode::Recycle,
            dry_run: false,
            items: vec![xiox_project::OutcomeItem {
                key: "k".into(),
                path,
                category: Category::Builds,
                method: xiox_project::Method::RecycleBin,
                outcome: o,
            }],
            bytes_freed: 7,
            recycled_bytes: 7,
            elapsed_ms: 3,
        }
    }

    #[test]
    fn outcome_json_wraps_scan_and_outcome_with_named_fields() {
        let scan = scan(vec![item(Category::Builds, "D:\\p\\target", Some(7), Level::Recommended, vec![], xiox_project::Why::NameOnly)]);
        let o = outcome_of_one(Some(PathBuf::from("D:\\p\\target")), Outcome::Removed { bytes_freed: 7 });
        let v = outcome_json(&scan, None, Some(&o)).unwrap();
        assert_eq!(v["schema"], 2);
        assert_eq!(v["scan"]["schema"], 2);
        assert_eq!(v["outcome"]["mode"], "recycle");
        assert_eq!(v["outcome"]["dryRun"], false);
        assert_eq!(v["outcome"]["bytesFreed"], 7);
        assert_eq!(v["outcome"]["recycledBytes"], 7);
        let item = &v["outcome"]["items"][0];
        assert_eq!(item["key"], "k");
        assert_eq!(item["path"], "D:\\p\\target");
        assert_eq!(item["category"], "builds");
        assert_eq!(item["outcome"]["type"], "removed");
        assert_eq!(item["outcome"]["bytesFreed"], 7);

        let skipped = outcome_of_one(None, Outcome::Skipped { reason: "no longer there".into() });
        let v = outcome_json(&scan, None, Some(&skipped)).unwrap();
        assert!(v["outcome"]["items"][0]["path"].is_null());
        assert_eq!(v["outcome"]["items"][0]["outcome"]["type"], "skipped");
        assert_eq!(v["outcome"]["items"][0]["outcome"]["reason"], "no longer there");
        assert!(!v.to_string().contains("bytes_freed"));
    }

    #[test]
    fn clean_json_is_schema_2_with_method_and_confirmations() {
        use xiox_project::{Chosen, Confirmation, Method};
        let bench = danger_scratch();
        let plan = Confirmation { key: bench.key.clone(), risks: bench.risks.clone(), method: Method::RecycleBin };
        let chosen = Chosen { keys: vec![bench.key.clone()], confirmed: vec![plan], ..Chosen::default() };
        let o = outcome_of_one(bench.path.clone(), Outcome::Removed { bytes_freed: 7 });
        let v = outcome_json(&danger_scan(), Some(&chosen), Some(&o)).unwrap();
        assert_eq!(v["schema"], SCHEMA);
        assert_eq!(v["chosen"]["confirmed"][0]["method"], "recycle-bin");
        assert_eq!(v["chosen"]["confirmed"][0]["risks"][0], "no-git");
        assert!(v["chosen"]["pending"].as_array().is_some_and(Vec::is_empty), "{v}");
        let item = v["outcome"]["items"][0].as_object().unwrap();
        let mut keys: Vec<&str> = item.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["category", "key", "method", "outcome", "path"]);
        assert_eq!(item["method"], "recycle-bin");
    }

    #[test]
    fn the_new_blocks_and_notes_have_stable_kebab_case_names() {
        let v = scan_json(&every_variant_scan()).unwrap();
        let text = v.to_string();
        for name in ["unsaved-work-unknown", "sessions-not-read", "logs-not-checked", "container-root"] {
            assert!(text.contains(&format!("\"{name}\"")), "{name} missing in {text}");
        }
        for bad in ["UnsavedWorkUnknown", "SessionsNotRead", "LogsNotChecked", "ContainerRoot"] {
            assert!(!text.contains(bad), "{bad} in {text}");
        }
    }

    #[test]
    fn outcome_json_is_null_when_nothing_was_run() {
        let v = outcome_json(&scan(vec![]), None, None).unwrap();
        assert_eq!(v["schema"], 2);
        assert!(v["outcome"].is_null());
        assert!(v["chosen"].is_null());
        assert!(v["scan"].is_object());
    }

    #[test]
    fn json_has_schema_1_and_camel_case_kebab_case_fields() {
        let v = scan_json(&full_scan()).unwrap();
        assert_eq!(v["schema"], 2);
        assert_eq!(v["project"]["name"], "mukade");
        assert_eq!(v["project"]["isRepo"], true);
        assert_eq!(v["search"], "bounded-walk");
        assert_eq!(v["scannedAtMs"], 1_700_000_000_000i64);
        let items = v["items"].as_array().unwrap();
        assert_eq!(items[0]["category"], "builds");
        assert_eq!(items[0]["level"], "recommended");
        assert_eq!(items[0]["why"]["type"], "catalog-folder");
        assert_eq!(items[0]["why"]["labelEn"], "Rust build");
        let wt = items.iter().find(|i| i["category"] == "worktrees" && i["blocked"].as_array().unwrap().len() == 2).unwrap();
        assert_eq!(wt["blocked"][0], "dirty-git");
        assert_eq!(wt["blocked"][1], "unpushed-commits");
        assert_eq!(wt["target"]["type"], "worktree");
        assert_eq!(v["branches"][0]["worktreeMissing"], true);
        assert_eq!(v["notes"][0]["type"], "prunable-worktree");
        // No snake_case key anywhere.
        let text = v.to_string();
        for bad in ["is_repo", "scanned_at_ms", "label_en", "worktree_missing"] {
            assert!(!text.contains(bad), "{bad} in {text}");
        }
    }
}
