//! `--json`: the scan (`analyze`) or the scan and the outcome (`clean`) as one JSON object (schema 1).

use dc_project::{CleanOutcome, ProjectScan};

/// The `ProjectScan` serialization: `{"schema":1,"project":…,"items":[…],…}`. Field names are
/// camelCase, enum values kebab-case; every text in it is English.
///
/// `Err` when a path is not valid Unicode (JSON cannot carry it).
pub fn scan_json(scan: &ProjectScan) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::to_value(scan)
}

/// The result of `clean --json`: `{"schema":1,"scan":<scan_json>,"outcome":<outcome>|null}`.
/// `outcome` is `null` when nothing was run. Its items are objects
/// (`{"key","path","category","outcome":{"type":"removed"|"skipped"|"failed",…}}`), not the
/// tuples of `CleanOutcome`, so the shape is stable and readable by name.
///
/// `Err` when a path is not valid Unicode (JSON cannot carry it).
pub fn outcome_json(scan: &ProjectScan, outcome: Option<&CleanOutcome>) -> Result<serde_json::Value, serde_json::Error> {
    // `to_value` (not `json!`, which panics) so that a path JSON cannot carry is an `Err`.
    use serde_json::{Map, Value, to_value};
    let outcome = match outcome {
        None => Value::Null,
        Some(o) => {
            let mut items = Vec::with_capacity(o.items.len());
            for (key, path, category, result) in &o.items {
                let mut item = Map::new();
                item.insert("key".into(), to_value(key)?);
                item.insert("path".into(), to_value(path)?);
                item.insert("category".into(), to_value(category)?);
                item.insert("outcome".into(), to_value(result)?);
                items.push(Value::Object(item));
            }
            let mut m = Map::new();
            m.insert("schema".into(), to_value(o.schema)?);
            m.insert("mode".into(), to_value(o.mode)?);
            m.insert("dryRun".into(), to_value(o.dry_run)?);
            m.insert("items".into(), Value::Array(items));
            m.insert("bytesFreed".into(), to_value(o.bytes_freed)?);
            m.insert("recycledBytes".into(), to_value(o.recycled_bytes)?);
            m.insert("elapsedMs".into(), to_value(o.elapsed_ms)?);
            Value::Object(m)
        }
    };
    let mut m = Map::new();
    m.insert("schema".into(), to_value(1)?);
    m.insert("scan".into(), scan_json(scan)?);
    m.insert("outcome".into(), outcome);
    Ok(Value::Object(m))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdata::*;
    use dc_model::{Level, RemoveMode};
    use dc_project::{Category, Outcome};
    use std::path::PathBuf;

    fn outcome_of_one(path: Option<PathBuf>, o: Outcome) -> CleanOutcome {
        CleanOutcome {
            schema: 1,
            mode: RemoveMode::Recycle,
            dry_run: false,
            items: vec![("k".into(), path, Category::Builds, o)],
            bytes_freed: 7,
            recycled_bytes: 7,
            elapsed_ms: 3,
        }
    }

    #[test]
    fn outcome_json_wraps_scan_and_outcome_with_named_fields() {
        let scan = scan(vec![item(Category::Builds, "D:\\p\\target", Some(7), Level::Recommended, vec![], dc_project::Why::NameOnly)]);
        let o = outcome_of_one(Some(PathBuf::from("D:\\p\\target")), Outcome::Removed { bytes_freed: 7 });
        let v = outcome_json(&scan, Some(&o)).unwrap();
        assert_eq!(v["schema"], 1);
        assert_eq!(v["scan"]["schema"], 1);
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
        let v = outcome_json(&scan, Some(&skipped)).unwrap();
        assert!(v["outcome"]["items"][0]["path"].is_null());
        assert_eq!(v["outcome"]["items"][0]["outcome"]["type"], "skipped");
        assert_eq!(v["outcome"]["items"][0]["outcome"]["reason"], "no longer there");
        assert!(!v.to_string().contains("bytes_freed"));
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
        let v = outcome_json(&scan(vec![]), None).unwrap();
        assert_eq!(v["schema"], 1);
        assert!(v["outcome"].is_null());
        assert!(v["scan"].is_object());
    }

    #[test]
    fn json_has_schema_1_and_camel_case_kebab_case_fields() {
        let v = scan_json(&full_scan()).unwrap();
        assert_eq!(v["schema"], 1);
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
