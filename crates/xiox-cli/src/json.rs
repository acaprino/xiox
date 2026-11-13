//! `--json`: the scan as one JSON object (schema 1).

use dc_project::ProjectScan;

/// The `ProjectScan` serialization: `{"schema":1,"project":…,"items":[…],…}`. Field names are
/// camelCase, enum values kebab-case; every text in it is English.
pub fn scan_json(scan: &ProjectScan) -> serde_json::Value {
    serde_json::to_value(scan).expect("a ProjectScan always serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdata::*;

    #[test]
    fn json_has_schema_1_and_camel_case_kebab_case_fields() {
        let v = scan_json(&full_scan());
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
