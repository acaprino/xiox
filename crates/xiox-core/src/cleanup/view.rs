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
/// The hover text of the egui button, as it was.
const KEEP_LIST_HINT: &str = "Opens Documents\\Xiox\\cookie_da_conservare.txt in Notepad (creates it if missing). \
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

fn level_text(l: LevelView) -> &'static str {
    match l {
        LevelView::Recommended => "recommended",
        LevelView::Optional => "optional",
        LevelView::Caution => "caution",
    }
}

fn count_text(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

impl CleanupState {
    /// The whole section for the page. `rev` is 0: `Core` numbers the views it publishes. `admin`
    /// is whether the program runs as administrator (what the built-in categories say of it).
    pub fn view(&self, admin: bool) -> SectionView {
        let busy = self.busy();
        let open = self.open_nav();
        let rows = self.rows(&open, admin);
        let items = rows.iter().filter(|r| r.kind != RowKind::Header).count();
        let nav = self.nav_view(&open);
        let report = self.report_view();
        let analysed = self.clean_view == CleanView::Analyzed && !busy;
        SectionView {
            section: SectionId::Cleanup,
            rev: 0,
            job: self.job_view(),
            status: self.status_view(),
            title: nav.iter().find(|n| n.active).map_or_else(String::new, |n| n.name.clone()),
            nav,
            options: self.options_view(&open),
            subtitle: match (items, analysed) {
                (0, _) => String::new(),
                (n, true) => format!("{} · largest first", count_text(n, "item", "items")),
                (n, false) => count_text(n, "item", "items"),
            },
            columns: [("Item", ColumnKind::Name), ("Level", ColumnKind::Level), ("Files", ColumnKind::Cell), ("Size", ColumnKind::Size)]
                .map(|(title, kind)| ColumnView { title: title.into(), kind })
                .into(),
            bulk: false,
            empty: self.empty_view(&open, rows.is_empty() && report.is_none()),
            rows,
            notices: self.notices_view(),
            actions: self.actions_view(),
            search: (open == WINAPP2).then(|| SearchView { placeholder: "Search".into(), value: self.rule_search.clone() }),
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

    fn nav_view(&self, open: &str) -> Vec<NavItem> {
        let mut nav = Vec::new();
        let mut item = |id: String, name: String, t: Tally| {
            let shown = self.clean_view == CleanView::Analyzed && (!self.busy() || t.answered);
            nav.push(NavItem {
                note: Some(format!("{} · {} selected", count_text(t.items, "item", "items"), t.selected)),
                bytes: shown.then_some(t.bytes),
                active: id == open,
                id,
                name,
            });
        };
        let mut groups: Vec<&str> = Vec::new();
        for c in cleaner::CATEGORIES {
            if !groups.contains(&c.group) {
                groups.push(c.group);
            }
        }
        for g in groups {
            let mut t = Tally::default();
            for (i, _) in cleaner::CATEGORIES.iter().enumerate().filter(|(_, c)| c.group == g) {
                t.add(self.clean_sel[i], self.found_of(i).map(|f| f.bytes));
            }
            item(format!("{BUILTIN}{g}"), g.to_string(), t);
        }
        for (g, nat, idx) in self.rule_groups() {
            let mut t = Tally::default();
            for n in nat {
                let it = &self.natives[n];
                t.add(self.native_on(it), self.native_found_of(&rules::native_key(it)).map(|f| f.bytes));
            }
            for &i in &idx {
                self.tally_cleaner(&mut t, i);
            }
            item(format!("{RULES}{g}"), g, t);
        }
        if let Some(rs) = self.rules.as_ref().filter(|rs| !rs.sections.is_empty()) {
            let mut t = Tally::default();
            for &i in rs.sections.iter().flat_map(|(_, idx)| idx) {
                self.tally_cleaner(&mut t, i);
            }
            item(WINAPP2.into(), rules::WINAPP2_GROUP.into(), t);
        }
        nav
    }

    fn tally_cleaner(&self, t: &mut Tally, i: usize) {
        let Some(rs) = &self.rules else { return };
        let c = &rs.cleaners[i];
        for o in &c.options {
            t.add(self.option_on(c, o), self.rule_found_of(&rules::key(c, o)).map(|f| f.bytes));
        }
    }

    // ---------- rows ----------

    /// The rows of the open category. A cleaning that was cancelled or stopped leaves no analysis to
    /// choose from: the report fills the place of the list.
    fn rows(&self, open: &str, admin: bool) -> Vec<RowView> {
        let dropped = self.clean_view == CleanView::Cleaned && !self.busy() && self.clean_report.as_ref().is_some_and(|r| r.cancelled);
        if dropped {
            return Vec::new();
        }
        if open == WINAPP2 {
            let Some(rs) = &self.rules else { return Vec::new() };
            let mut rows = Vec::new();
            for (section, idx) in winapp2_matches(rs, &self.rule_search) {
                rows.push(RowView { name: format!("{section} ({})", idx.len()), ..blank_row(format!("section:{section}"), RowKind::Header) });
                rows.extend(idx.iter().flat_map(|&i| rs.cleaners[i].options.iter().map(move |o| (i, o))).map(|(i, o)| self.option_row(&rs.cleaners[i], o)));
            }
            return rows;
        }
        if let Some(group) = open.strip_prefix(BUILTIN) {
            return (0..cleaner::CATEGORIES.len()).filter(|&i| cleaner::CATEGORIES[i].group == group).map(|i| self.category_row(i, admin)).collect();
        }
        let Some((_, nat, idx)) = self.rule_groups().into_iter().find(|(g, _, _)| Some(g.as_str()) == open.strip_prefix(RULES)) else {
            return Vec::new();
        };
        let mut rows: Vec<RowView> = nat.iter().map(|&n| self.native_row(&self.natives[n])).collect();
        if let Some(rs) = &self.rules {
            rows.extend(idx.iter().flat_map(|&i| rs.cleaners[i].options.iter().map(move |o| (i, o))).map(|(i, o)| self.option_row(&rs.cleaners[i], o)));
        }
        rows
    }

    fn category_row(&self, i: usize, admin: bool) -> RowView {
        let c = &cleaner::CATEGORIES[i];
        let level = if c.default_on { LevelView::Recommended } else { LevelView::Optional };
        let found = self.found_of(i);
        let mode = if c.kind == cleaner::Kind::RecycleBin { MODE_BIN } else { MODE };
        let mut detail = vec![("item".to_string(), c.name.to_string())];
        if let Some(f) = found {
            detail.push(("files".into(), f.count.to_string()));
            detail.push(("size".into(), sys::fmt_size(f.bytes)));
        }
        detail.extend([("level".to_string(), level_text(level).to_string()), ("mode".into(), mode.into()), ("why".into(), c.help.into())]);
        if let Some(f) = found.filter(|f| f.skipped_dirs > 0 && !admin) {
            let n = f.skipped_dirs as usize;
            detail.push(("skipped".into(), format!("{} skipped: they need administrator rights", count_text(n, "folder", "folders"))));
        }
        RowView {
            check: if self.clean_sel[i] { Check::On } else { Check::Off },
            name: c.name.into(),
            sub: Some(c.help.into()),
            level: Some(level),
            cells: found.map(|f| vec![f.count.to_string()]).unwrap_or_default(),
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
            detail.push(("files".into(), f.files.to_string()));
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
            cells: found.map(|f| vec![if f.files > 0 { f.files.to_string() } else { "—".to_string() }]).unwrap_or_default(),
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
            detail.push(("files".into(), f.count.to_string()));
            detail.push(("size".into(), sys::fmt_size(f.bytes)));
        }
        detail.extend([("level".to_string(), level_text(level).to_string()), ("mode".into(), MODE.into())]);
        detail.extend(description.clone().map(|d| ("why".to_string(), d)));
        detail.extend(o.warning.clone().map(|w| ("warning".to_string(), w)));
        let left = found.filter(|f| f.skipped > 0);
        if let Some(f) = left {
            detail.push(("left".into(), format!("{} left (protected or unreadable)", count_text(f.skipped as usize, "item", "items"))));
        }
        RowView {
            check: if blocked.is_none() && self.rule_sel.contains(&key) { Check::On } else { Check::Off },
            enabled: blocked.is_none(),
            name,
            sub: description,
            level: Some(level),
            cells: found.map(|f| vec![f.count.to_string()]).unwrap_or_default(),
            bytes: found.map(|f| f.bytes),
            muted: blocked.is_some(),
            why: blocked,
            detail,
            lines: left.map(|f| f.skipped_reasons.clone()).unwrap_or_default(),
            ..blank_row(key, RowKind::Item)
        }
    }

    // ---------- the rest of the view ----------

    fn options_view(&self, open: &str) -> Vec<OptionView> {
        let mut v = vec![OptionView::Toggle {
            id: "onlyOld".into(),
            label: format!("Only delete Temp files older than {} hours", cleaner::TEMP_MIN_AGE_HOURS),
            on: self.only_old,
            enabled: self.editable().is_ok(),
            hint: None,
        }];
        let holds_cookies = self
            .rule_groups()
            .iter()
            .any(|(g, nat, _)| Some(g.as_str()) == open.strip_prefix(RULES) && nat.iter().any(|&n| self.natives[n].id == browserdb::COOKIES_ID));
        if holds_cookies {
            v.push(OptionView::Button { id: "keepList".into(), label: "Edit cookie keep list".into(), enabled: true, hint: Some(KEEP_LIST_HINT.into()) });
        }
        if let (true, Some(rs)) = (open == WINAPP2, &self.rules) {
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
                (analysis_title(false, self.clean_end).0.to_string(), format!("{} to delete ({files} files)", sys::fmt_size(bytes)))
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

    fn empty_view(&self, open: &str, nothing: bool) -> Option<EmptyView> {
        if !nothing {
            return None;
        }
        let title = self.nav_view(open).into_iter().find(|n| n.active).map_or_else(String::new, |n| n.name);
        let search = self.rule_search.trim();
        let text = if open == WINAPP2 && !search.is_empty() { format!("Nothing matches “{search}”.") } else { "Nothing to choose in this list.".to_string() };
        Some(EmptyView { title, text })
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
            n => format!("Clean {}", count_text(n, "item", "items")),
        };
        v.push(action("clean", label, "Enter", can && self.clean_view == CleanView::Analyzed, true));
        v
    }

    fn dialog_view(&self) -> Option<DialogView> {
        if !self.confirm_clean {
            return None;
        }
        let button = |id: &str, label: &str, primary| ButtonView { id: id.into(), label: label.into(), enabled: true, primary };
        Some(DialogView {
            id: "confirm".into(),
            title: "Start cleaning?".into(),
            text: vec!["The selected files will be deleted permanently. Files in use are left where they are.".into()],
            summary: Vec::new(),
            rows: Vec::new(),
            warnings_title: (!self.confirm_warnings.is_empty()).then(|| "Warnings for the chosen options".to_string()),
            warnings: self.confirm_warnings.clone(),
            caution_title: None,
            cautions: Vec::new(),
            ack: None,
            error: None,
            buttons: vec![button("continue", "Continue", true), button("cancel", "Cancel", false)],
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
