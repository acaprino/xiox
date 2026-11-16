//! The view contract: what Rust tells the page (`SectionView`) and what the page tells Rust
//! (the intents). `apps/desktop/src/types.ts` is the source of truth; every type here mirrors it
//! field for field.
//!
//! Rust is snake_case, JSON is camelCase; string unions are unit enums (kebab-free, camelCase);
//! tagged unions use `kind`. `(A, B)` is a two-element array, `Option<T>` is `T | null`. There is no
//! `#[serde(default)]`: a missing field is an error, and so is an unknown one.
//!
//! `CmdError` (`{ kind: 'refused' | 'failed', message }`) is not here: it belongs to the Tauri crate.

use serde::{Deserialize, Serialize};

pub use crate::notify::SectionId;

/// What a section tells when its worker ends without an answer (it died): the job is over and
/// the page is told, never left with a spinner.
pub const JOB_DIED: &str = "The job was interrupted by an internal error.";

/// `T | null` in the contract: the key has to be there. (serde takes a missing `Option` for
/// `null` unless a field says how to read it.)
fn required<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d)
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Tone {
    Info,
    Ok,
    Warn,
    Error,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LevelView {
    Recommended,
    Optional,
    Caution,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Check {
    On,
    Off,
    Mixed,
    None,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RowKind {
    Item,
    /// A sub-heading inside a group (the sections of Winapp2): no check box.
    Header,
    Child,
    /// A category of a list that shows all its categories one after another (Cleanup): its check
    /// box ticks what the category holds. Its `id` is the id of the category's `NavItem`, which the
    /// page scrolls to.
    Group,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ColumnKind {
    Name,
    Cell,
    Level,
    Size,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppInfo {
    pub version: String,
    pub admin: bool,
    pub ram_total_bytes: u64,
    #[serde(deserialize_with = "required")]
    pub profile_mismatch: Option<String>,
    pub fake: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AboutView {
    pub version: String,
    pub winapp2: String,
    pub attribution: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobView {
    pub label: String,
    pub fraction: f32,
    #[serde(deserialize_with = "required")]
    pub detail: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusView {
    pub badge: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavItem {
    pub id: String,
    pub name: String,
    #[serde(deserialize_with = "required")]
    pub note: Option<String>,
    #[serde(deserialize_with = "required")]
    pub bytes: Option<u64>,
    pub active: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChoiceItem {
    pub id: String,
    pub label: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum OptionView {
    Toggle { id: String, label: String, on: bool, enabled: bool, #[serde(deserialize_with = "required")] hint: Option<String> },
    Number { id: String, label: String, value: u32, min: u32, max: u32, enabled: bool },
    Choice { id: String, label: String, value: String, choices: Vec<ChoiceItem>, enabled: bool },
    List {
        id: String,
        label: String,
        items: Vec<String>,
        #[serde(deserialize_with = "required")] selected: Option<usize>,
        enabled: bool,
        add_label: String,
        remove_label: String,
        empty: String,
    },
    Button { id: String, label: String, enabled: bool, #[serde(deserialize_with = "required")] hint: Option<String> },
    Meter { id: String, label: String, used_bytes: u64, total_bytes: u64, tone: Tone },
    Text { text: String, tone: Tone, #[serde(deserialize_with = "required")] hint: Option<String> },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColumnView {
    pub title: String,
    pub kind: ColumnKind,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RowView {
    pub id: String,
    pub kind: RowKind,
    #[serde(deserialize_with = "required")]
    pub parent: Option<String>,
    pub check: Check,
    pub enabled: bool,
    pub name: String,
    #[serde(deserialize_with = "required")]
    pub sub: Option<String>,
    #[serde(deserialize_with = "required")]
    pub level: Option<LevelView>,
    pub cells: Vec<String>,
    #[serde(deserialize_with = "required")]
    pub bytes: Option<u64>,
    #[serde(deserialize_with = "required")]
    pub size_text: Option<String>,
    pub muted: bool,
    #[serde(deserialize_with = "required")]
    pub why: Option<String>,
    pub detail: Vec<(String, String)>,
    pub lines: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoticeView {
    pub tone: Tone,
    pub text: String,
    pub more: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionView {
    pub id: String,
    pub label: String,
    #[serde(deserialize_with = "required")]
    pub key: Option<String>,
    pub enabled: bool,
    pub primary: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ButtonView {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub primary: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DialogView {
    pub id: String,
    pub title: String,
    pub text: Vec<String>,
    pub summary: Vec<String>,
    /// The primary button deletes for good: the page makes it the danger variant and starts the focus on Cancel.
    pub permanent: bool,
    pub rows: Vec<(String, String)>,
    #[serde(deserialize_with = "required")]
    pub warnings_title: Option<String>,
    pub warnings: Vec<String>,
    #[serde(deserialize_with = "required")]
    pub caution_title: Option<String>,
    pub cautions: Vec<(String, String)>,
    #[serde(deserialize_with = "required")]
    pub ack: Option<bool>,
    #[serde(deserialize_with = "required")]
    pub error: Option<String>,
    pub buttons: Vec<ButtonView>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportLine {
    pub item: String,
    pub why: String,
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportItem {
    pub name: String,
    pub freed_bytes: u64,
    pub removed: u64,
    pub left: u64,
    #[serde(deserialize_with = "required")]
    pub note: Option<String>,
    pub lines: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedView {
    pub text: String,
    pub ok: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportView {
    pub title: String,
    pub freed_bytes: u64,
    pub recycled_bytes: u64,
    pub files: u64,
    pub dirs: u64,
    pub elapsed_ms: u64,
    pub cancelled: bool,
    pub notes: Vec<NoticeView>,
    pub by_category: Vec<(String, u64)>,
    pub skipped: Vec<ReportLine>,
    pub failed: Vec<ReportLine>,
    pub items: Vec<ReportItem>,
    pub can_save: bool,
    #[serde(deserialize_with = "required")]
    pub saved: Option<SavedView>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EmptyView {
    pub title: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchView {
    pub placeholder: String,
    pub value: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SectionView {
    pub section: SectionId,
    pub rev: u64,
    #[serde(deserialize_with = "required")]
    pub job: Option<JobView>,
    pub status: StatusView,
    pub nav: Vec<NavItem>,
    pub options: Vec<OptionView>,
    pub title: String,
    pub subtitle: String,
    pub columns: Vec<ColumnView>,
    pub rows: Vec<RowView>,
    pub bulk: bool,
    #[serde(deserialize_with = "required")]
    pub empty: Option<EmptyView>,
    pub notices: Vec<NoticeView>,
    pub actions: Vec<ActionView>,
    #[serde(deserialize_with = "required")]
    pub search: Option<SearchView>,
    #[serde(deserialize_with = "required")]
    pub dialog: Option<DialogView>,
    #[serde(deserialize_with = "required")]
    pub report: Option<ReportView>,
    #[serde(deserialize_with = "required")]
    pub footer: Option<String>,
}

// ---------- intents (page -> Rust) ----------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum CleanupIntent {
    Toggle { row: String },
    SelectAll { on: bool },
    SetOnlyOld { on: bool },
    Search { text: String },
    Analyze,
    OpenConfirm,
    Confirm,
    CloseDialog,
    Cancel,
    OpenKeepList,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum RegistryIntent {
    ToggleCategory { index: usize },
    Toggle { row: String },
    SelectAll { on: bool },
    Scan,
    OpenFix,
    Fix { backup: bool },
    CloseDialog,
}

/// What a services confirmation does: `disable` | `manual` | `restore`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ServiceAction {
    Disable,
    Manual,
    Restore,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum ServicesIntent {
    SetAll { all: bool },
    Search { text: String },
    Toggle { row: String },
    SelectAll { on: bool },
    Refresh,
    OpenConfirm { action: ServiceAction },
    Confirm,
    CloseDialog,
}

/// The three lists of Developers: `projects` | `worktrees` | `sessions`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DevCategory {
    Projects,
    Worktrees,
    Sessions,
}

/// How Developers removes things: `recycle` | `permanent`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RemoveModeView {
    Recycle,
    Permanent,
}

impl From<RemoveModeView> for xiox_model::RemoveMode {
    fn from(m: RemoveModeView) -> Self {
        match m {
            RemoveModeView::Recycle => xiox_model::RemoveMode::Recycle,
            RemoveModeView::Permanent => xiox_model::RemoveMode::Permanent,
        }
    }
}

impl From<xiox_model::RemoveMode> for RemoveModeView {
    fn from(m: xiox_model::RemoveMode) -> Self {
        match m {
            xiox_model::RemoveMode::Recycle => RemoveModeView::Recycle,
            xiox_model::RemoveMode::Permanent => RemoveModeView::Permanent,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum DevIntent {
    Select { category: DevCategory },
    Toggle { row: String },
    SelectAll { on: bool },
    SetInactiveDays { days: u32 },
    SetRemoveMode { mode: RemoveModeView },
    SetScanRoots { on: bool },
    SelectRoot { index: Option<usize> },
    RemoveRoot,
    Analyze,
    OpenConfirm,
    SetAck { on: bool },
    Confirm,
    Permanent,
    CloseDialog,
    Cancel,
}

/// A row with nothing filled in: the section views set what their rows have (struct update).
pub(crate) fn blank_row(id: String, kind: RowKind) -> RowView {
    RowView {
        id,
        kind,
        parent: None,
        check: Check::None,
        enabled: true,
        name: String::new(),
        sub: None,
        level: None,
        cells: Vec::new(),
        bytes: None,
        size_text: None,
        muted: false,
        why: None,
        detail: Vec::new(),
        lines: Vec::new(),
    }
}

/// Helpers the intent tests of every section share.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::notify::{NoNotifier, SharedNotifier};
    use std::sync::Arc;

    pub fn notifier() -> SharedNotifier {
        Arc::new(NoNotifier)
    }

    /// Polls `done` until it holds (the workers of the tests answer from their own thread).
    pub fn wait(mut done: impl FnMut() -> bool) {
        for _ in 0..1000 {
            if done() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the worker did not answer in 10 s");
    }

    /// The refusal of an intent that has to be refused.
    pub fn err<T>(r: Result<T, String>) -> String {
        match r {
            Err(e) => e,
            Ok(_) => panic!("expected a refusal"),
        }
    }

    pub fn action<'a>(v: &'a SectionView, id: &str) -> &'a ActionView {
        v.actions.iter().find(|a| a.id == id).unwrap_or_else(|| panic!("no action {id}"))
    }

    pub fn button<'a>(d: &'a DialogView, id: &str) -> &'a ButtonView {
        d.buttons.iter().find(|b| b.id == id).unwrap_or_else(|| panic!("no button {id}"))
    }

    pub fn row<'a>(v: &'a SectionView, id: &str) -> &'a RowView {
        v.rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(id: &str) -> RowView {
        RowView {
            id: id.into(),
            kind: RowKind::Item,
            parent: None,
            check: Check::Mixed,
            enabled: true,
            name: "acme".into(),
            sub: Some("D:\\Projects\\acme".into()),
            level: Some(LevelView::Caution),
            cells: vec!["2026-11-10".into()],
            bytes: Some(1024),
            size_text: None,
            muted: false,
            why: None,
            detail: vec![("item".into(), "acme".into())],
            lines: vec![],
        }
    }

    /// One of everything: every kind of option, a dialog, a report.
    fn sample() -> SectionView {
        SectionView {
            section: SectionId::Developers,
            rev: 7,
            job: Some(JobView { label: "Cleaning…".into(), fraction: 0.5, detail: None }),
            status: StatusView { badge: "Analysis done".into(), text: "1 to free".into() },
            nav: vec![NavItem { id: "projects".into(), name: "Projects".into(), note: None, bytes: Some(10), active: true }],
            options: vec![
                OptionView::Toggle { id: "t".into(), label: "T".into(), on: true, enabled: true, hint: None },
                OptionView::Number { id: "n".into(), label: "N".into(), value: 30, min: 1, max: 365, enabled: true },
                OptionView::Choice {
                    id: "c".into(),
                    label: "C".into(),
                    value: "recycle".into(),
                    choices: vec![ChoiceItem { id: "recycle".into(), label: "Recycle".into() }],
                    enabled: false,
                },
                OptionView::List {
                    id: "l".into(),
                    label: "L".into(),
                    items: vec!["D:\\a".into()],
                    selected: Some(0),
                    enabled: true,
                    add_label: "Add".into(),
                    remove_label: "Remove".into(),
                    empty: "None".into(),
                },
                OptionView::Button { id: "b".into(), label: "B".into(), enabled: true, hint: Some("h".into()) },
                OptionView::Meter { id: "m".into(), label: "M".into(), used_bytes: 1, total_bytes: 2, tone: Tone::Warn },
                OptionView::Text { text: "t".into(), tone: Tone::Error, hint: None },
            ],
            title: "Projects".into(),
            subtitle: "sub".into(),
            columns: vec![ColumnView { title: "Size".into(), kind: ColumnKind::Size }],
            rows: vec![RowView { kind: RowKind::Group, ..row("g:a") }, row("p:a")],
            bulk: true,
            empty: Some(EmptyView { title: "t".into(), text: "x".into() }),
            notices: vec![NoticeView { tone: Tone::Info, text: "n".into(), more: vec!["m".into()] }],
            actions: vec![ActionView { id: "analyze".into(), label: "Analyze".into(), key: Some("a".into()), enabled: true, primary: false }],
            search: Some(SearchView { placeholder: "Search".into(), value: String::new() }),
            dialog: Some(DialogView {
                id: "confirm".into(),
                title: "Clean?".into(),
                text: vec!["t".into()],
                summary: vec!["s".into()],
                permanent: true,
                rows: vec![("a".into(), "1 KB".into())],
                warnings_title: Some("Warnings".into()),
                warnings: vec!["w".into()],
                caution_title: Some("1 folder at the Caution level:".into()),
                cautions: vec![("a".into(), "careful".into())],
                ack: Some(false),
                error: None,
                buttons: vec![ButtonView { id: "clean".into(), label: "Clean".into(), enabled: false, primary: true }],
            }),
            report: Some(ReportView {
                title: "Cleanup finished".into(),
                freed_bytes: 10,
                recycled_bytes: 0,
                files: 1,
                dirs: 2,
                elapsed_ms: 3,
                cancelled: false,
                notes: vec![],
                by_category: vec![("Projects".into(), 10)],
                skipped: vec![ReportLine { item: "i".into(), why: "w".into(), path: "p".into() }],
                failed: vec![],
                items: vec![ReportItem { name: "n".into(), freed_bytes: 1, removed: 2, left: 3, note: None, lines: vec![] }],
                can_save: true,
                saved: Some(SavedView { text: "saved".into(), ok: true }),
            }),
            footer: Some("f".into()),
        }
    }

    #[test]
    fn views_round_trip_through_json() {
        let v = sample();
        let json = serde_json::to_value(&v).unwrap();
        let back: SectionView = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(back, v);
        // camelCase keys, lower-case section, kind-tagged options, two-element arrays for pairs.
        assert_eq!(json["section"], "developers");
        assert!(json["rows"][0].get("sizeText").is_some());
        assert!(json["dialog"].get("warningsTitle").is_some());
        assert!(json["dialog"].get("cautionTitle").is_some());
        assert_eq!(json["options"][3]["kind"], "list");
        assert!(json["options"][3].get("addLabel").is_some());
        assert!(json["options"][5].get("usedBytes").is_some());
        assert_eq!(json["rows"][0]["check"], "mixed");
        assert_eq!(json["rows"][0]["kind"], "group");
        assert_eq!(json["rows"][1]["kind"], "item");
        assert_eq!(json["rows"][0]["level"], "caution");
        assert_eq!(json["columns"][0]["kind"], "size");
        assert_eq!(json["rows"][0]["detail"], json!([["item", "acme"]]));
        assert_eq!(json["report"]["byCategory"], json!([["Projects", 10]]));
        // A missing field and an unknown one are both errors.
        let mut missing = json.clone();
        missing.as_object_mut().unwrap().remove("footer");
        assert!(serde_json::from_value::<SectionView>(missing).is_err());
        let mut extra = json;
        extra["bogus"] = json!(1);
        assert!(serde_json::from_value::<SectionView>(extra).is_err());
    }

    /// The fixture the TypeScript mock serves is written against `types.ts`: it has to read
    /// through the Rust mirror of it, field for field (no missing key, no unknown one).
    #[test]
    fn the_shared_fixture_reads_through_the_contract() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/fixtures/views.json");
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let _: AppInfo = serde_json::from_value(doc["app"].clone()).unwrap();
        let _: AboutView = serde_json::from_value(doc["about"].clone()).unwrap();
        let mut n = 0;
        for (section, states) in doc["views"].as_object().unwrap() {
            for (state, view) in states.as_object().unwrap() {
                let v: SectionView = serde_json::from_value(view.clone()).unwrap_or_else(|e| panic!("{section}:{state}: {e}"));
                let again: SectionView = serde_json::from_value(serde_json::to_value(&v).unwrap()).unwrap();
                assert_eq!(again, v, "{section}:{state} does not round-trip");
                n += 1;
            }
        }
        assert!(n >= 20, "{n} fixture views");
    }

    #[test]
    fn intents_use_the_page_spelling() {
        let intent: DevIntent = serde_json::from_value(json!({ "kind": "setRemoveMode", "mode": "permanent" })).unwrap();
        assert_eq!(intent, DevIntent::SetRemoveMode { mode: RemoveModeView::Permanent });
        let intent: DevIntent = serde_json::from_value(json!({ "kind": "select", "category": "worktrees" })).unwrap();
        assert_eq!(intent, DevIntent::Select { category: DevCategory::Worktrees });
        let intent: DevIntent = serde_json::from_value(json!({ "kind": "selectRoot", "index": null })).unwrap();
        assert_eq!(intent, DevIntent::SelectRoot { index: None });
        let intent: DevIntent = serde_json::from_value(json!({ "kind": "setInactiveDays", "days": 90 })).unwrap();
        assert_eq!(intent, DevIntent::SetInactiveDays { days: 90 });
        assert_eq!(serde_json::to_value(DevIntent::OpenConfirm).unwrap(), json!({ "kind": "openConfirm" }));
        let intent: ServicesIntent = serde_json::from_value(json!({ "kind": "openConfirm", "action": "restore" })).unwrap();
        assert_eq!(intent, ServicesIntent::OpenConfirm { action: ServiceAction::Restore });
        assert_eq!(serde_json::to_value(RegistryIntent::Fix { backup: true }).unwrap(), json!({ "kind": "fix", "backup": true }));
        assert_eq!(serde_json::to_value(CleanupIntent::SetOnlyOld { on: true }).unwrap(), json!({ "kind": "setOnlyOld", "on": true }));
        // Cleanup and Services select all the way Registry and Developers do; Cleanup has no `select` any more.
        assert_eq!(serde_json::to_value(CleanupIntent::SelectAll { on: true }).unwrap(), json!({ "kind": "selectAll", "on": true }));
        let intent: ServicesIntent = serde_json::from_value(json!({ "kind": "selectAll", "on": false })).unwrap();
        assert_eq!(intent, ServicesIntent::SelectAll { on: false });
        assert!(serde_json::from_value::<CleanupIntent>(json!({ "kind": "select", "category": "builtin:System" })).is_err());
        assert!(serde_json::from_value::<DevIntent>(json!({ "kind": "format-c" })).is_err());
    }
}
