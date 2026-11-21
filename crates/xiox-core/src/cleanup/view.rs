//! The Cleanup section as a `SectionView`: what the page draws, built from the state.
//!
//! The strings that come from the moved helpers (`analysis_title`, the engine's reasons, the warnings
//! of the options, the native items' labels and the group names of `cleaner::CATEGORIES`) are English
//! at the source, like the sentences written here.

use super::*;
use crate::view::*;
use std::path::Path;

const MODE: &str = "permanently";
const MODE_BIN: &str = "empties the Recycle Bin";
const NEEDS_ADMIN: &str = "Needs administrator: some folders are skipped";
/// What a category's box says when the category holds Caution rows: those are not ticked by it.
const CAUTION_BOX: &str = "Caution items are not ticked by this box: tick them one by one.";
/// The hover text of the «keep list» button.
const KEEP_LIST_HINT: &str = "Opens Documents\\Xiox\\cookie-keep-list.txt in Notepad (creates it if missing). \
                              One domain per line: their cookies stay.";

/// What a nav group holds: its rows, how many of them will run, and what the analysis found there.
#[derive(Default)]
struct Tally {
    items: usize,
    selected: usize,
    bytes: u64,
    answered: bool,
}

impl Tally {
    fn add(&mut self, selected: bool, found: Option<u64>) {
        self.items += 1;
        self.selected += usize::from(selected);
        self.answered |= found.is_some();
        self.bytes += found.unwrap_or(0);
    }
}

/// A row of a category and the number of files it stands for (what the category's line adds up).
pub(super) struct Entry {
    pub row: RowView,
    pub files: u64,
}

/// A category as the list shows it: the rows left by the search.
pub(super) struct Shown {
    pub id: String,
    pub name: String,
    pub entries: Vec<Entry>,
}

/// A row that a category's box, a sub-heading's box and «select all» may tick (the rule all the lists share).
pub(super) use crate::view::eligible;

/// The id of a sub-heading of Winapp2 (a section of its rules): `section:<name>`. Its box ticks the section's rows.
pub(super) const SECTION_ROW: &str = "section:";

/// The label of the clean action and the first line of its confirmation: «Clean 8 items».
fn clean_label(items: usize) -> String {
    format!("Clean {}", sys::plural(items, "item", "items"))
}

/// What the analysis found, as the status line and the confirmation say it: «13.48 GB to delete (46,999 files)».
fn to_delete_text(bytes: u64, files: u64) -> String {
    format!("{} to delete ({})", sys::fmt_size(bytes), sys::plural(files, "file", "files"))
}

fn level_text(l: LevelView) -> &'static str {
    match l {
        LevelView::Recommended => "recommended",
        LevelView::Optional => "optional",
        LevelView::Caution => "caution",
    }
}

impl CleanupState {
    /// The whole section for the page. `rev` is 0: `Core` numbers the views it publishes. `admin`
    /// is whether the program runs as administrator (what the built-in categories say of it). Every
    /// category is in the list, one after another, each led by a `Group` row; the page scrolls to the
    /// category a `NavItem` names, so no category is "open" and `active` is never set.
    pub fn view(&self, admin: bool) -> SectionView {
        let busy = self.busy();
        let report = self.report_view();
        let rows = self.list_rows(self.shown_groups(admin), busy);
        let items = rows.iter().filter(|r| matches!(r.kind, RowKind::Item | RowKind::Child)).count();
        let selected = rows.iter().filter(|r| matches!(r.kind, RowKind::Item | RowKind::Child) && r.check == Check::On).count();
        SectionView {
            section: SectionId::Cleanup,
            rev: 0,
            job: self.job_view(),
            status: self.status_view(),
            nav: self.nav_view(),
            options: self.options_view(),
            title: "Cleanup".into(),
            subtitle: if items == 0 { String::new() } else { format!("{} · {} selected", sys::plural(items, "item", "items"), sys::fmt_count(selected as u64)) },
            columns: [("Item", ColumnKind::Name), ("Level", ColumnKind::Level), ("Files", ColumnKind::Cell), ("Size", ColumnKind::Size)]
                .map(|(title, kind)| ColumnView { title: title.into(), kind })
                .into(),
            bulk: !busy && report.is_none() && rows.iter().any(eligible),
            empty: self.empty_view(rows.is_empty() && report.is_none()),
            rows,
            notices: self.notices_view(),
            actions: self.actions_view(),
            search: (!busy && report.is_none()).then(|| SearchView { placeholder: "Search all categories".into(), value: self.rule_search.clone() }),
            dialog: self.dialog_view(),
            report,
            footer: None,
        }
    }

    /// An analysis on screen: the figures of its rows. After a cleaning they would describe what
    /// was there before, so they go.
    fn found_of(&self, i: usize) -> Option<&cleaner::Found> {
        self.found[i].as_ref().filter(|_| self.clean_view == CleanView::Analyzed)
    }

    fn native_found_of(&self, key: &str) -> Option<&native::NativeFound> {
        self.native_found.iter().find(|(k, _, _)| k == key).map(|(_, _, f)| f).filter(|_| self.clean_view == CleanView::Analyzed)
    }

    fn rule_found_of(&self, key: &str) -> Option<&rules::RuleFound> {
        self.rule_found.iter().find(|(k, _, _)| k == key).map(|(_, _, f)| f).filter(|_| self.clean_view == CleanView::Analyzed)
    }

    /// A native item that will run: chosen and not blocked.
    fn native_on(&self, item: &native::NativeItem) -> bool {
        self.rule_sel.contains(&rules::native_key(item)) && rules::native_blocked(item, &self.processes).is_none()
    }

    fn option_on(&self, c: &xiox_rules::Cleaner, o: &xiox_rules::Opt) -> bool {
        self.rule_sel.contains(&rules::key(c, o)) && rules::blocked(c, o, &self.processes).is_none()
    }

    // ---------- nav ----------

    /// The menu of the page: one item per category, in the order of the list, with what it holds and
    /// what the analysis found there. The search does not touch it, and none is `active` (the page
    /// follows the scroll).
    fn nav_view(&self) -> Vec<NavItem> {
        self.groups()
            .into_iter()
            .map(|g| {
                let mut t = Tally::default();
                match &g.members {
                    Members::Builtin(idx) => {
                        for &i in idx {
                            t.add(self.clean_sel[i], self.found_of(i).map(|f| f.bytes));
                        }
                    }
                    Members::Rules(nat, idx) => {
                        for &n in nat {
                            let it = &self.natives[n];
                            t.add(self.native_on(it), self.native_found_of(&rules::native_key(it)).map(|f| f.bytes));
                        }
                        for &i in idx {
                            self.tally_cleaner(&mut t, i);
                        }
                    }
                    Members::Winapp2 => {
                        if let Some(rs) = &self.rules {
                            for &i in rs.sections.iter().flat_map(|(_, idx)| idx) {
                                self.tally_cleaner(&mut t, i);
                            }
                        }
                    }
                }
                let shown = self.clean_view == CleanView::Analyzed && (!self.busy() || t.answered);
                NavItem {
                    note: Some(format!("{} · {} selected", sys::plural(t.items, "item", "items"), sys::fmt_count(t.selected as u64))),
                    bytes: shown.then_some(t.bytes),
                    active: false,
                    id: g.id,
                    name: g.name,
                }
            })
            .collect()
    }

    fn tally_cleaner(&self, t: &mut Tally, i: usize) {
        let Some(rs) = &self.rules else { return };
        let c = &rs.cleaners[i];
        for o in &c.options {
            t.add(self.option_on(c, o), self.rule_found_of(&rules::key(c, o)).map(|f| f.bytes));
        }
    }

    // ---------- rows ----------

    /// The categories as the list shows them, in order: each with the rows the search leaves. A
    /// cleaning that was cancelled or stopped leaves no analysis to choose from: the report fills the
    /// place of the list. A category the search leaves empty is not there; the rows of a category
    /// are kept when the search is in their name or in their description (Winapp2: in the name of
    /// the section, of the cleaner or of one of its options, as before).
    pub(super) fn shown_groups(&self, admin: bool) -> Vec<Shown> {
        let dropped = self.clean_view == CleanView::Cleaned && !self.busy() && self.clean_report.as_ref().is_some_and(|r| r.cancelled);
        if dropped {
            return Vec::new();
        }
        let needle = self.rule_search.trim().to_lowercase();
        let hit = |text: &str| text.to_lowercase().contains(&needle);
        let keep = |r: &RowView| needle.is_empty() || hit(&r.name) || r.sub.as_deref().is_some_and(hit);
        let mut shown = Vec::new();
        for g in self.groups() {
            let mut entries: Vec<Entry> = match &g.members {
                Members::Builtin(idx) => idx
                    .iter()
                    .map(|&i| Entry { row: self.category_row(i, admin), files: self.found_of(i).map_or(0, |f| f.count) })
                    .collect(),
                Members::Rules(nat, idx) => {
                    let mut entries: Vec<Entry> = nat
                        .iter()
                        .map(|&n| {
                            let item = &self.natives[n];
                            Entry { row: self.native_row(item), files: self.native_found_of(&rules::native_key(item)).map_or(0, |f| f.files) }
                        })
                        .collect();
                    if let Some(rs) = &self.rules {
                        entries.extend(
                            idx.iter().flat_map(|&i| rs.cleaners[i].options.iter().map(move |o| (i, o))).map(|(i, o)| self.option_entry(&rs.cleaners[i], o)),
                        );
                    }
                    entries
                }
                Members::Winapp2 => {
                    let Some(rs) = &self.rules else { continue };
                    let mut entries = Vec::new();
                    for (section, idx) in winapp2_matches(rs, &self.rule_search) {
                        entries.push(Entry {
                            row: RowView { name: format!("{section} ({})", sys::fmt_count(idx.len() as u64)), ..blank_row(format!("{SECTION_ROW}{section}"), RowKind::Header) },
                            files: 0,
                        });
                        entries.extend(
                            idx.iter().flat_map(|&i| rs.cleaners[i].options.iter().map(move |o| (i, o))).map(|(i, o)| self.option_entry(&rs.cleaners[i], o)),
                        );
                    }
                    entries
                }
            };
            if !matches!(g.members, Members::Winapp2) {
                entries.retain(|e| keep(&e.row));
            }
            if entries.iter().any(|e| e.row.kind != RowKind::Header) {
                shown.push(Shown { id: g.id, name: g.name, entries });
            }
        }
        shown
    }

    fn option_entry(&self, c: &xiox_rules::Cleaner, o: &xiox_rules::Opt) -> Entry {
        Entry { row: self.option_row(c, o), files: self.rule_found_of(&rules::key(c, o)).map_or(0, |f| f.count) }
    }

    /// The flat list the page draws: a `Group` row, then the rows of its category, each sub-heading with its box.
    pub(super) fn list_rows(&self, shown: Vec<Shown>, busy: bool) -> Vec<RowView> {
        let mut rows = Vec::new();
        for g in shown {
            rows.push(self.group_row(&g, busy));
            rows.extend(g.entries.into_iter().map(|e| e.row));
        }
        crate::view::box_headers(&mut rows, busy);
        rows
    }

    /// The line of a category: its box (what it can tick: on, off or some), and once the analysis has
    /// answered there, the files and bytes of its rows. Rows at the Caution level are not ticked by
    /// the box, and the line says so.
    fn group_row(&self, g: &Shown, busy: bool) -> RowView {
        let check = crate::view::box_of(g.entries.iter().map(|e| &e.row));
        let analysed = g.entries.iter().any(|e| e.row.bytes.is_some());
        let caution = g.entries.iter().any(|e| e.row.kind == RowKind::Item && e.row.level == Some(LevelView::Caution));
        RowView {
            check,
            enabled: check != Check::None && !busy,
            name: g.name.clone(),
            cells: if analysed { vec![sys::fmt_count(g.entries.iter().map(|e| e.files).sum())] } else { Vec::new() },
            bytes: analysed.then(|| g.entries.iter().filter_map(|e| e.row.bytes).sum()),
            why: caution.then(|| CAUTION_BOX.to_string()),
            ..blank_row(g.id.clone(), RowKind::Group)
        }
    }

    fn category_row(&self, i: usize, admin: bool) -> RowView {
        let c = &cleaner::CATEGORIES[i];
        let level = if c.default_on { LevelView::Recommended } else { LevelView::Optional };
        let found = self.found_of(i);
        let mode = if c.kind == cleaner::Kind::RecycleBin { MODE_BIN } else { MODE };
        let mut detail = vec![("item".to_string(), c.name.to_string())];
        if let Some(f) = found {
            detail.push(("files".into(), sys::fmt_count(f.count)));
            detail.push(("size".into(), sys::fmt_size(f.bytes)));
        }
        detail.extend([("level".to_string(), level_text(level).to_string()), ("mode".into(), mode.into()), ("why".into(), c.help.into())]);
        if let Some(f) = found.filter(|f| f.skipped_dirs > 0 && !admin) {
            let n = f.skipped_dirs as usize;
            detail.push(("skipped".into(), format!("{} skipped: they need administrator rights", sys::plural(n, "folder", "folders"))));
        }
        RowView {
            check: if self.clean_sel[i] { Check::On } else { Check::Off },
            name: c.name.into(),
            sub: Some(c.help.into()),
            level: Some(level),
            cells: found.map(|f| vec![sys::fmt_count(f.count)]).unwrap_or_default(),
            bytes: found.map(|f| f.bytes),
            why: (c.needs_admin && !admin).then(|| NEEDS_ADMIN.to_string()),
            detail,
            ..blank_row(format!("cat:{i}"), RowKind::Item)
        }
    }

    fn native_row(&self, item: &native::NativeItem) -> RowView {
        let key = rules::native_key(item);
        let blocked = rules::native_blocked(item, &self.processes);
        let level = match (item.warning, item.default_on) {
            (Some(_), _) => LevelView::Caution,
            (None, true) => LevelView::Recommended,
            (None, false) => LevelView::Optional,
        };
        let found = self.native_found_of(&key);
        let mut detail = vec![("item".to_string(), item.label.to_string())];
        if let Some(f) = found {
            detail.push(("files".into(), sys::fmt_count(f.files as u64)));
            detail.push(("size".into(), sys::fmt_size(f.bytes)));
        }
        detail.extend([("level".to_string(), level_text(level).to_string()), ("mode".into(), MODE.into()), ("why".into(), item.description.into())]);
        if let Some(w) = item.warning {
            detail.push(("warning".into(), w.into()));
        }
        RowView {
            check: if blocked.is_none() && self.rule_sel.contains(&key) { Check::On } else { Check::Off },
            enabled: blocked.is_none(),
            name: item.label.into(),
            sub: Some(item.description.into()),
            level: Some(level),
            // Channels, DNS and the clipboard are not files: the number is in the detail.
            cells: found.map(|f| vec![if f.files > 0 { sys::fmt_count(f.files as u64) } else { "—".to_string() }]).unwrap_or_default(),
            bytes: found.map(|f| f.bytes),
            muted: blocked.is_some(),
            why: blocked.or_else(|| item.warning.map(str::to_string)),
            detail,
            lines: found.map(|f| f.detail.clone()).unwrap_or_default(),
            ..blank_row(key, RowKind::Item)
        }
    }

    fn option_row(&self, c: &xiox_rules::Cleaner, o: &xiox_rules::Opt) -> RowView {
        let key = rules::key(c, o);
        let blocked = rules::blocked(c, o, &self.processes);
        let name = rules::row_label(c, o);
        let description = [&o.description, &c.description].into_iter().find(|d| !d.is_empty()).cloned();
        let level = match (&o.warning, o.default_on) {
            (Some(_), _) => LevelView::Caution,
            (None, true) => LevelView::Recommended,
            (None, false) => LevelView::Optional,
        };
        let found = self.rule_found_of(&key);
        let mut detail = vec![("item".to_string(), name.clone())];
        if let Some(f) = found {
            detail.push(("files".into(), sys::fmt_count(f.count)));
            detail.push(("size".into(), sys::fmt_size(f.bytes)));
        }
        detail.extend([("level".to_string(), level_text(level).to_string()), ("mode".into(), MODE.into())]);
        detail.extend(description.clone().map(|d| ("why".to_string(), d)));
        detail.extend(o.warning.clone().map(|w| ("warning".to_string(), w)));
        let left = found.filter(|f| f.skipped > 0);
        if let Some(f) = left {
            detail.push(("left".into(), format!("{} left (protected or unreadable)", sys::plural(f.skipped as usize, "item", "items"))));
        }
        RowView {
            check: if blocked.is_none() && self.rule_sel.contains(&key) { Check::On } else { Check::Off },
            enabled: blocked.is_none(),
            name,
            sub: description,
            level: Some(level),
            cells: found.map(|f| vec![sys::fmt_count(f.count)]).unwrap_or_default(),
            bytes: found.map(|f| f.bytes),
            muted: blocked.is_some(),
            why: blocked,
            detail,
            lines: left.map(|f| f.skipped_reasons.clone()).unwrap_or_default(),
            ..blank_row(key, RowKind::Item)
        }
    }

    // ---------- the rest of the view ----------

    fn options_view(&self) -> Vec<OptionView> {
        let mut v = vec![OptionView::Toggle {
            id: "onlyOld".into(),
            label: format!("Only delete Temp files older than {} hours", cleaner::TEMP_MIN_AGE_HOURS),
            on: self.only_old,
            enabled: self.editable().is_ok(),
            hint: None,
        }];
        if self.natives.iter().any(|i| i.id == browserdb::COOKIES_ID) {
            v.push(OptionView::Button { id: "keepList".into(), label: "Edit cookie keep list".into(), enabled: true, hint: Some(KEEP_LIST_HINT.into()) });
        }
        // Winapp2's own line: when no copy came (offline at the first start) it is the only place that says why.
        if let Some(rs) = &self.rules {
            let (text, why) = rules::status_line(rs, self.winapp2_update.as_ref());
            let hint: Vec<&str> = why.as_deref().into_iter().chain(rs.warnings.iter().map(String::as_str)).collect();
            v.push(OptionView::Text { text, tone: Tone::Info, hint: (!hint.is_empty()).then(|| hint.join("\n")) });
        }
        v
    }

    fn job_view(&self) -> Option<JobView> {
        if !self.busy() {
            return None;
        }
        let label = if self.cancelling() {
            "Cancelling…"
        } else if self.clean_view == CleanView::Cleaned {
            "Cleaning…"
        } else {
            "Analyzing…"
        };
        Some(JobView { label: label.into(), fraction: self.clean_progress, detail: (!self.clean_text.is_empty()).then(|| self.clean_text.clone()) })
    }

    /// What the analysis found: (bytes, files) over the categories, the native items and the options.
    fn found_totals(&self) -> (u64, u64) {
        let cats = self.found.iter().flatten();
        let bytes = cats.clone().map(|f| f.bytes).sum::<u64>()
            + self.native_found.iter().map(|r| r.2.bytes).sum::<u64>()
            + self.rule_found.iter().map(|r| r.2.bytes).sum::<u64>();
        let files = cats.map(|f| f.count).sum::<u64>()
            + self.native_found.iter().map(|r| r.2.files).sum::<u64>()
            + self.rule_found.iter().map(|r| r.2.count).sum::<u64>();
        (bytes, files)
    }

    fn freed_so_far(&self) -> u64 {
        self.cleaned.iter().flatten().map(|c| c.freed).sum::<u64>()
            + self.native_cleaned.iter().map(|r| r.2.freed).sum::<u64>()
            + self.rule_cleaned.iter().map(|r| r.2.freed).sum::<u64>()
    }

    fn status_view(&self) -> StatusView {
        let (busy, cancelling) = (self.busy(), self.cancelling());
        let (badge, text) = match self.clean_view {
            CleanView::Empty => ("Ready".to_string(), "Pick what to clean, then analyze".to_string()),
            CleanView::Analyzed if busy => (if cancelling { "Cancelling" } else { "Analyzing" }.to_string(), String::new()),
            CleanView::Analyzed => {
                let (bytes, files) = self.found_totals();
                (analysis_title(false, self.clean_end).0.to_string(), to_delete_text(bytes, files))
            }
            CleanView::Cleaned if busy => {
                (if cancelling { "Cancelling" } else { "Cleaning" }.to_string(), format!("{} freed", sys::fmt_size(self.freed_so_far())))
            }
            CleanView::Cleaned => {
                let freed = self.clean_report.as_ref().map_or_else(|| self.freed_so_far(), |r| r.freed_bytes);
                (self.report_title().to_string(), format!("{} freed", sys::fmt_size(freed)))
            }
        };
        StatusView { badge, text }
    }

    fn report_title(&self) -> &'static str {
        match (self.clean_end, self.clean_report.as_ref().is_some_and(|r| r.cancelled)) {
            (Some(CleanEnd::Died), _) => "Cleanup stopped",
            (_, true) => "Cleanup cancelled",
            _ => "Cleanup done",
        }
    }

    /// The analysis that was cut short says so; a finished one has nothing to add.
    fn notices_view(&self) -> Vec<NoticeView> {
        if self.clean_view != CleanView::Analyzed || self.busy() {
            return Vec::new();
        }
        let note = analysis_title(false, self.clean_end).1;
        note.map(|text| NoticeView { tone: Tone::Error, text: text.into(), more: Vec::new() }).into_iter().collect()
    }

    fn empty_view(&self, nothing: bool) -> Option<EmptyView> {
        if !nothing {
            return None;
        }
        let search = self.rule_search.trim();
        Some(if search.is_empty() {
            EmptyView { title: "Cleanup".into(), text: "Nothing to choose in this list.".into() }
        } else {
            EmptyView { title: "No matches".into(), text: format!("Nothing matches “{search}”.") }
        })
    }

    /// Selected built-in categories, native items and options that will run: the number on «Clean».
    fn clean_count(&self) -> usize {
        self.clean_sel.iter().filter(|b| **b).count()
            + rules::active_natives(&self.natives, &self.rule_sel, &self.processes).len()
            + self.rules.as_ref().map_or(0, |rs| rules::active_choices(&rs.cleaners, &self.rule_sel, &self.processes).len())
    }

    fn actions_view(&self) -> Vec<ActionView> {
        let busy = self.busy();
        let can = !busy && !self.confirm_clean && self.any_selected();
        let action = |id: &str, label: String, key: &str, enabled, primary| ActionView { id: id.into(), label, key: Some(key.into()), enabled, primary };
        let mut v = vec![action("analyze", "Analyze".into(), "a", can, false)];
        if busy {
            let cancelling = self.cancelling();
            v.push(action("cancel", if cancelling { "Cancelling…" } else { "Cancel" }.into(), "Esc", !cancelling, false));
        }
        let label = match self.clean_count() {
            0 => "Clean".to_string(),
            n => clean_label(n),
        };
        v.push(action("clean", label, "Enter", can && self.clean_view == CleanView::Analyzed, true));
        v
    }

    fn dialog_view(&self) -> Option<DialogView> {
        if !self.confirm_clean {
            return None;
        }
        let button = |id: &str, label: &str, primary| ButtonView { id: id.into(), label: label.into(), enabled: true, primary };
        let (bytes, files) = self.found_totals();
        Some(DialogView {
            id: "confirm".into(),
            title: "Start cleaning?".into(),
            text: vec!["The selected files will be deleted permanently. Files in use are left where they are.".into()],
            summary: vec![clean_label(self.clean_count()), to_delete_text(bytes, files)],
            permanent: true,
            rows: Vec::new(),
            warnings_title: (!self.confirm_warnings.is_empty()).then(|| "Warnings for the chosen options".to_string()),
            warnings: self.confirm_warnings.clone(),
            caution_title: None,
            cautions: Vec::new(),
            ack: None,
            error: None,
            // The confirming button says what it does and to what, as the action that opened the dialog does: "Clean 8 items".
            buttons: vec![button("continue", &clean_label(self.clean_count()), true), button("cancel", "Cancel", false)],
        })
    }

    // ---------- the report ----------

    /// The line of a file or an item that stayed: a file names the category that listed it, an item
    /// (the report keeps the label of those as a path without a folder) names itself.
    fn report_line(&self, path: &Path, why: &str) -> ReportLine {
        if !path.is_absolute() {
            return ReportLine { item: path.display().to_string(), why: why.into(), path: String::new() };
        }
        let owner = self.cleaned.iter().enumerate().find(|(_, c)| c.as_ref().is_some_and(|c| c.failures.iter().any(|(p, _)| p == path)));
        let item = owner.map_or_else(|| path.display().to_string(), |(i, _)| cleaner::CATEGORIES[i].name.to_string());
        ReportLine { item, why: why.into(), path: path.display().to_string() }
    }

    /// One item per category, native item and option that answered: freed, removed, left, the first note.
    fn report_items(&self) -> Vec<ReportItem> {
        let item = |name: &str, freed: u64, removed: u64, left: u64, note: Option<String>, lines: Vec<String>| ReportItem {
            name: name.into(),
            freed_bytes: freed,
            removed,
            left,
            note,
            lines,
        };
        let mut items = Vec::new();
        for (i, c) in self.cleaned.iter().enumerate().filter_map(|(i, c)| c.as_ref().map(|c| (i, c))) {
            items.push(item(cleaner::CATEGORIES[i].name, c.freed, c.removed, c.failed, None, Vec::new()));
        }
        for (_, label, c) in &self.native_cleaned {
            let lines: Vec<String> = c.done.iter().chain(&c.notes).cloned().collect();
            items.push(item(label, c.freed, c.removed, c.failed, c.notes.first().cloned(), lines));
        }
        for (_, label, c) in &self.rule_cleaned {
            let mut lines = c.notes.clone();
            lines.extend(c.backup.iter().map(|b| format!("Registry backup: {}", b.display())));
            items.push(item(label, c.freed, c.removed, c.failed, lines.first().cloned(), lines));
        }
        items
    }

    fn report_view(&self) -> Option<ReportView> {
        let r = self.clean_report.as_ref().filter(|_| self.clean_view == CleanView::Cleaned && !self.busy())?;
        let died = self.clean_end == Some(CleanEnd::Died);
        let note = |tone, text: &str| NoticeView { tone, text: text.into(), more: Vec::new() };
        let mut notes = Vec::new();
        if died {
            notes.push(note(
                Tone::Error,
                "An internal error stopped the cleanup: other items may have been touched without appearing here. Press “Analyze” to see what is left.",
            ));
        } else if r.cancelled {
            notes.push(note(Tone::Info, "Cancelled: the items not started were left where they were."));
        }
        if !died && !r.failed.is_empty() {
            notes.push(note(Tone::Warn, "Some files could not be removed. They are listed below."));
        }
        let lines = |list: &[(std::path::PathBuf, String)]| -> Vec<ReportLine> { list.iter().map(|(p, why)| self.report_line(p, why)).collect() };
        Some(ReportView {
            title: self.report_title().into(),
            freed_bytes: r.freed_bytes,
            recycled_bytes: r.recycled_bytes,
            files: r.files,
            dirs: r.dirs,
            elapsed_ms: r.elapsed_ms,
            cancelled: r.cancelled,
            notes,
            by_category: r.by_category.clone(),
            skipped: lines(&r.skipped),
            failed: lines(&r.failed),
            items: self.report_items(),
            can_save: true,
            saved: self.clean_saved.as_ref().map(|(text, ok)| SavedView { text: text.clone(), ok: *ok }),
        })
    }
}
