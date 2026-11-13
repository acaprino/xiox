//! The Developers section as a `SectionView`: what the page draws, built from the state.
//!
//! The strings that come from the moved helpers (`clean_button`, `confirm_text`, `locked_why`,
//! `fmt_size`, the engine's reasons...) are Italian until Task 18 translates them at the source;
//! the sentences written here are English.

use super::*;
use crate::view::*;

const CHOSEN: &str = "to the Recycle Bin";
const PERMANENT: &str = "permanently";

impl DevState {
    /// The whole section for the page. `rev` is 0: `Core` numbers the views it publishes.
    /// (`admin` is not shown by this section: the status bar of the window says it.)
    pub fn view(&self, _admin: bool) -> SectionView {
        let busy = self.busy();
        let (selected, total, bytes) = self.selection();
        let rows = self.rows();
        SectionView {
            section: SectionId::Developers,
            rev: 0,
            job: self.job_view(),
            status: self.status_view(selected, total, bytes),
            nav: self.nav_view(),
            options: self.options_view(),
            title: self.section.name().to_string(),
            subtitle: self.subtitle(),
            columns: columns(self.section),
            bulk: self.view == View::Scanned && !busy,
            empty: self.empty_view(rows.is_empty()),
            rows,
            notices: self.notices_view(),
            actions: self.actions_view(selected),
            search: None,
            dialog: self.dialog_view(),
            report: self.report_view(),
            footer: self.footer(selected, bytes),
        }
    }

    fn mode_text(&self) -> &'static str {
        match self.settings.remove_mode {
            RemoveMode::Recycle => CHOSEN,
            RemoveMode::Permanent => PERMANENT,
        }
    }

    fn job_view(&self) -> Option<JobView> {
        if !self.busy() {
            return None;
        }
        Some(match &self.tick {
            Some(t) => JobView {
                label: if self.cancelling() { "Cancelling…" } else { "Cleaning…" }.into(),
                fraction: tick_fraction(t),
                detail: Some(tick_text(t)),
            },
            None => JobView { label: self.text.clone(), fraction: self.progress, detail: None },
        })
    }

    fn status_view(&self, selected: usize, total: usize, bytes: u64) -> StatusView {
        let busy = self.busy();
        let (badge, text) = match (self.view, &self.scan, &self.cleaned) {
            (View::Empty, _, _) if busy => ("Analyzing".to_string(), String::new()),
            (View::Empty, _, _) => ("Ready".to_string(), "No analysis yet".to_string()),
            (View::Cleaned, _, _) if busy => (if self.cancelling() { "Cancelling" } else { "Cleaning" }.to_string(), String::new()),
            (View::Scanned, Some(scan), _) => {
                let how = if scan.backend == "mft" { "sizes from the disk MFT" } else { "folders measured one by one" };
                (
                    "Analysis done".to_string(),
                    format!("{} to free · {selected} of {total} selected · {} · {how}", sys::fmt_size(bytes), fmt_elapsed(scan.elapsed_ms)),
                )
            }
            (View::Scanned, None, _) => ("Analysis done".to_string(), String::new()),
            (View::Cleaned, _, c) => (
                cleaned_title(c.as_ref(), false).to_string(),
                c.as_ref().map_or_else(String::new, |c| format!("Space freed: {}", sys::fmt_size(c.report.freed_bytes))),
            ),
        };
        StatusView { badge, text }
    }

    fn nav_view(&self) -> Vec<NavItem> {
        [(Section::Projects, "projects"), (Section::Worktrees, "worktrees"), (Section::Sessions, "sessions")]
            .into_iter()
            .map(|(s, id)| {
                let picked = self
                    .scan
                    .as_ref()
                    .map_or(0, |scan| section_stats(scan, s, &self.sel_artifacts, &self.sel_worktrees, &self.sel_sessions).0);
                let bytes = self.scan.as_ref().map(|scan| match s {
                    Section::Projects => scan.projects.iter().map(|r| r.reclaimable_bytes).sum(),
                    Section::Worktrees => scan.worktrees.iter().map(|r| r.reclaimable_bytes).sum(),
                    Section::Sessions => scan.archived.iter().map(|a| a.bytes).sum(),
                });
                NavItem {
                    id: id.into(),
                    name: s.name().into(),
                    note: (picked > 0).then(|| selected_text(s, picked)),
                    bytes,
                    active: self.section == s,
                }
            })
            .collect()
    }

    fn options_view(&self) -> Vec<OptionView> {
        let enabled = !self.busy();
        let s = &self.settings;
        let mut v = vec![
            OptionView::Number {
                id: "inactiveDays".into(),
                label: "Days of inactivity".into(),
                value: s.inactive_days,
                min: 1,
                max: 365,
                enabled,
            },
            OptionView::Choice {
                id: "removeMode".into(),
                label: "Deletion".into(),
                value: match RemoveModeView::from(s.remove_mode) {
                    RemoveModeView::Recycle => "recycle",
                    RemoveModeView::Permanent => "permanent",
                }
                .into(),
                choices: vec![
                    ChoiceItem { id: "recycle".into(), label: "To the Recycle Bin (recoverable)".into() },
                    ChoiceItem { id: "permanent".into(), label: "Permanently".into() },
                ],
                enabled,
            },
            OptionView::Text {
                text: "The Recycle Bin frees no space until you empty it. “Recycle Bin” in Cleanup empties it.".into(),
                tone: Tone::Info,
                hint: None,
            },
            OptionView::Toggle { id: "scanRoots".into(), label: "Also analyze these folders".into(), on: s.scan_roots, enabled, hint: None },
            OptionView::List {
                id: "roots".into(),
                label: "Extra folders".into(),
                items: s.roots.iter().map(|r| r.display().to_string()).collect(),
                selected: self.root_sel.filter(|i| *i < s.roots.len()),
                enabled,
                add_label: "Add folder…".into(),
                remove_label: "Remove".into(),
                empty: "No folders.".into(),
            },
        ];
        if let Some(e) = &self.settings_err {
            v.push(OptionView::Text { text: format!("Settings not saved: {e}"), tone: Tone::Error, hint: None });
        }
        v
    }

    /// The line under the title: what the boxes of the open list mean (only with an analysis).
    fn subtitle(&self) -> String {
        if self.view != View::Scanned {
            return String::new();
        }
        match self.section {
            Section::Projects => "Each folder has its own box: “Recommended” is suggested for inactive projects, the others are up to you.",
            Section::Worktrees => "Worktrees are removed with git and always permanently; those with unsaved changes stay.",
            Section::Sessions => "",
        }
        .into()
    }

    fn rows(&self) -> Vec<RowView> {
        let Some(scan) = self.scan.as_ref().filter(|_| self.view == View::Scanned) else { return Vec::new() };
        match self.section {
            Section::Projects => self.project_rows(scan),
            Section::Worktrees => self.worktree_rows(scan),
            Section::Sessions => self.session_rows(scan),
        }
    }

    fn project_rows(&self, scan: &DevScan) -> Vec<RowView> {
        let mode = self.mode_text();
        let mut rows = Vec::new();
        for r in &scan.projects {
            let free = status_allows(r);
            let locked = (!free).then(|| locked_why(r));
            let check = if r.artifacts.is_empty() {
                Check::None
            } else {
                match tri_state(r, &self.sel_artifacts) {
                    Tri::All => Check::On,
                    Tri::None => Check::Off,
                    Tri::Some => Check::Mixed,
                }
            };
            let date = r.last_activity_ms.map_or_else(|| "—".to_string(), fmt_date);
            let mut detail = vec![
                ("item".to_string(), r.name.clone()),
                ("folder".to_string(), r.root.display().to_string()),
                ("last use".to_string(), date.clone()),
                ("size".to_string(), sys::fmt_size(r.reclaimable_bytes)),
            ];
            if let Some(why) = locked.clone().or_else(|| (!r.reason.is_empty()).then(|| r.reason.clone())) {
                detail.push(("why".into(), why));
            }
            let id = format!("p:{}", r.id);
            rows.push(RowView {
                check,
                enabled: free,
                name: r.name.clone(),
                sub: (!r.reason.is_empty()).then(|| r.reason.clone()),
                cells: vec![date],
                bytes: Some(r.reclaimable_bytes),
                muted: !free,
                why: locked.clone(),
                detail,
                ..blank_row(id.clone(), RowKind::Item)
            });
            for a in &r.artifacts {
                let mut detail = vec![
                    ("item".to_string(), format!("{} · {}", r.name, a.label)),
                    ("folder".to_string(), a.path.display().to_string()),
                    ("size".to_string(), sys::fmt_size(a.size.effective_bytes())),
                    ("level".to_string(), level_word(a.level).to_string()),
                    ("mode".to_string(), mode.to_string()),
                ];
                if !a.note.is_empty() {
                    detail.push(("why".into(), a.note.clone()));
                }
                rows.push(RowView {
                    parent: Some(id.clone()),
                    check: if self.sel_artifacts.contains(&a.id) { Check::On } else { Check::Off },
                    enabled: free,
                    name: a.label.clone(),
                    sub: (!a.note.is_empty()).then(|| a.note.clone()),
                    level: Some(level_view(a.level)),
                    bytes: Some(a.size.effective_bytes()),
                    muted: !free,
                    why: locked.clone(),
                    detail,
                    ..blank_row(format!("a:{}", a.id), RowKind::Child)
                });
            }
        }
        rows
    }

    fn worktree_rows(&self, scan: &DevScan) -> Vec<RowView> {
        let repo = |r: &ProjectReport| r.git.main_worktree.as_deref().map_or_else(|| "—".to_string(), folder_name);
        let mut rows = Vec::new();
        for (i, r) in scan.worktrees.iter().enumerate() {
            let on = selectable(r);
            let nothing = nothing_to_remove(r);
            // Why the box is off: nothing to take out, or the worktree is in use / blocked.
            let blocked = (!on).then(|| nothing.map(str::to_string).unwrap_or_else(|| locked_why(r)));
            let mut detail = vec![
                ("item".to_string(), r.name.clone()),
                ("folder".to_string(), r.root.display().to_string()),
                ("size".to_string(), sys::fmt_size(r.reclaimable_bytes)),
                ("mode".to_string(), "permanently (git)".to_string()),
            ];
            if let Some(why) = blocked.clone().or_else(|| (!r.reason.is_empty()).then(|| r.reason.clone())) {
                detail.push(("why".into(), why));
            }
            rows.push(RowView {
                check: if self.sel_worktrees.get(i).copied().unwrap_or(false) { Check::On } else { Check::Off },
                enabled: on,
                name: r.name.clone(),
                sub: Some(r.root.display().to_string()),
                cells: vec![repo(r), nothing.map_or_else(|| r.reason.clone(), str::to_string)],
                bytes: Some(r.reclaimable_bytes),
                muted: !on,
                why: blocked,
                detail,
                ..blank_row(format!("w:{}", r.id), RowKind::Item)
            });
        }
        if !scan.other_worktrees.is_empty() {
            rows.push(RowView {
                name: "Other worktrees (created by hand, not removed from here)".into(),
                ..blank_row("h:others".into(), RowKind::Header)
            });
            for r in &scan.other_worktrees {
                rows.push(RowView {
                    name: r.name.clone(),
                    sub: Some(r.root.display().to_string()),
                    cells: vec![repo(r), r.reason.clone()],
                    bytes: Some(r.reclaimable_bytes),
                    muted: true,
                    detail: vec![
                        ("item".to_string(), r.name.clone()),
                        ("folder".to_string(), r.root.display().to_string()),
                        ("size".to_string(), sys::fmt_size(r.reclaimable_bytes)),
                    ],
                    ..blank_row(format!("o:{}", r.id), RowKind::Item)
                });
            }
        }
        rows
    }

    fn session_rows(&self, scan: &DevScan) -> Vec<RowView> {
        let mode = self.mode_text();
        scan.archived
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let free = s.blocked.is_none();
                let (bytes, size_text) = if s.file.is_some() { (Some(s.bytes), None) } else { (None, Some(session_size(s))) };
                let mut detail = vec![
                    ("item".to_string(), session_label(s)),
                    ("size".to_string(), session_size(s)),
                    ("mode".to_string(), mode.to_string()),
                ];
                if let Some(why) = &s.blocked {
                    detail.push(("why".into(), why.clone()));
                }
                RowView {
                    check: if self.sel_sessions.get(i).copied().unwrap_or(false) { Check::On } else { Check::Off },
                    enabled: free,
                    name: session_title(s),
                    cells: vec![
                        agent_label(s.agent).to_string(),
                        s.cwd.as_deref().map_or_else(|| "—".to_string(), folder_name),
                        s.updated_ms.map_or_else(|| "—".to_string(), fmt_date),
                    ],
                    bytes,
                    size_text,
                    muted: !free,
                    why: s.blocked.clone(),
                    detail,
                    ..blank_row(format!("s:{}:{}", session_agent_id(s.agent), s.id), RowKind::Item)
                }
            })
            .collect()
    }

    fn empty_view(&self, no_rows: bool) -> Option<EmptyView> {
        match self.view {
            View::Empty if self.busy() => Some(EmptyView {
                title: "Analyzing".into(),
                text: "The analysis can take a minute: meanwhile you can use the other sections.".into(),
            }),
            View::Empty => Some(EmptyView {
                title: "Developer cleanup".into(),
                text: "Press “Analyze”: the projects come from the Claude Code, Codex, OpenCode and Pi sessions.".into(),
            }),
            View::Scanned if no_rows => {
                let failed = self.scan.as_ref().is_some_and(|s| analysis_failed(s));
                Some(EmptyView { title: self.section.name().into(), text: empty_text(self.section, failed).into() })
            }
            _ => None,
        }
    }

    fn notices_view(&self) -> Vec<NoticeView> {
        let warn = |text: String, more: Vec<String>| NoticeView { tone: Tone::Warn, text, more };
        let error = |text: String| NoticeView { tone: Tone::Error, text, more: Vec::new() };
        let mut v = Vec::new();
        match (self.view, &self.scan, &self.cleaned) {
            (View::Scanned, Some(scan), _) => {
                if let Some(n) = &self.notice {
                    v.push(warn(n.clone(), Vec::new()));
                }
                let ordered = ordered_warnings(&scan.warnings);
                for w in ordered.iter().take(2) {
                    let text = if w.as_str() == ANALYSIS_FAILED {
                        format!("{w} The projects and worktrees were not analyzed: press “Analyze” to try again.")
                    } else {
                        w.to_string()
                    };
                    v.push(warn(text, Vec::new()));
                }
                // Past two, the rest hide behind the second one.
                if ordered.len() > 2 {
                    if let Some(last) = v.last_mut() {
                        last.more = ordered[2..].iter().map(|w| w.to_string()).collect();
                    }
                }
            }
            (View::Cleaned, _, Some(c)) if !self.busy() => {
                for e in &c.errors {
                    v.push(error(e.clone()));
                }
                if !c.errors.is_empty() {
                    v.push(error("That part was not touched. Press “Analyze” to analyze again before retrying.".into()));
                }
                if c.interrupted {
                    v.push(error(
                        "An internal error stopped the cleanup: other items may have been touched without appearing here. Press “Analyze” to see what is left."
                            .into(),
                    ));
                }
            }
            _ => {}
        }
        if self.view == View::Empty {
            if let Some(e) = &self.error {
                v.push(error(e.clone()));
            }
        }
        v
    }

    fn actions_view(&self, selected: usize) -> Vec<ActionView> {
        let busy = self.busy();
        let mut v = vec![ActionView { id: "analyze".into(), label: "Analyze".into(), key: Some("a".into()), enabled: !busy, primary: false }];
        if busy && self.cancel.is_some() {
            let cancelling = self.cancelling();
            v.push(ActionView {
                id: "cancel".into(),
                label: if cancelling { "Cancelling…" } else { "Cancel" }.into(),
                key: Some("Esc".into()),
                enabled: !cancelling,
                primary: false,
            });
        }
        v.push(ActionView {
            id: "clean".into(),
            label: clean_button(self.section, selected),
            key: Some("Enter".into()),
            enabled: self.can_clean(),
            primary: true,
        });
        v
    }

    fn dialog_view(&self) -> Option<DialogView> {
        let c = self.confirm.as_ref()?;
        let ready = confirm_ready(c);
        let mut text = vec![confirm_text(c.section, shown_mode(c)).to_string()];
        text.extend(c.others.clone());
        if !c.shortfall.is_empty() {
            text.push(shortfall_text(&c.shortfall));
        }
        let button = |id: &str, label: String, enabled: bool, primary: bool| ButtonView { id: id.into(), label, enabled, primary };
        // With a Recycle Bin too small there is one way forward: «Permanently».
        let mut buttons = if c.shortfall.is_empty() {
            vec![button("clean", confirm_button(c), ready, true)]
        } else {
            vec![button("permanent", "Permanently".into(), ready, true)]
        };
        buttons.push(button("cancel", "Cancel".into(), true, false));
        Some(DialogView {
            id: "confirm".into(),
            title: confirm_title(c.section).into(),
            text,
            summary: c.summary.clone(),
            rows: c.rows.clone(),
            warnings_title: None,
            warnings: Vec::new(),
            caution_title: (!c.cautions.is_empty()).then(|| caution_title(c.cautions.len())),
            cautions: c.cautions.clone(),
            ack: (!c.cautions.is_empty()).then_some(c.ack),
            error: c.error.clone(),
            buttons,
        })
    }

    fn report_view(&self) -> Option<ReportView> {
        let c = self.cleaned.as_ref().filter(|_| self.view == View::Cleaned && !self.busy())?;
        let r = &c.report;
        let mut notes = Vec::new();
        if r.cancelled {
            notes.push(NoticeView {
                tone: Tone::Info,
                text: "Cancelled: the items not yet started stayed where they were.".into(),
                more: Vec::new(),
            });
        }
        if c.is_empty() && c.errors.is_empty() && !c.interrupted {
            notes.push(NoticeView { tone: Tone::Info, text: "Nothing to clean.".into(), more: Vec::new() });
        }
        let lines = |list: &[(PathBuf, String)]| -> Vec<ReportLine> {
            list.iter()
                .map(|(path, why)| ReportLine { item: c.label(path), why: why.clone(), path: path.display().to_string() })
                .collect()
        };
        Some(ReportView {
            title: cleaned_title(Some(c), false).into(),
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
            items: Vec::new(),
            can_save: !c.is_empty(),
            saved: self.saved.as_ref().map(|(text, ok)| SavedView { text: text.clone(), ok: *ok }),
        })
    }

    fn footer(&self, selected: usize, bytes: u64) -> Option<String> {
        self.scan.as_ref().filter(|_| self.view == View::Scanned).map(|_| {
            format!("Selected in {}: {} · {}", self.section.name(), sys::fmt_size(bytes), selection_text(self.section, selected))
        })
    }
}

fn columns(s: Section) -> Vec<ColumnView> {
    let col = |title: &str, kind| ColumnView { title: title.into(), kind };
    match s {
        Section::Projects => vec![
            col("Project · folder", ColumnKind::Name),
            col("Last use", ColumnKind::Cell),
            col("Level", ColumnKind::Level),
            col("Size", ColumnKind::Size),
        ],
        Section::Worktrees => vec![
            col("Worktree", ColumnKind::Name),
            col("Repository", ColumnKind::Cell),
            col("State", ColumnKind::Cell),
            col("Size", ColumnKind::Size),
        ],
        Section::Sessions => vec![
            col("Title", ColumnKind::Name),
            col("Harness", ColumnKind::Cell),
            col("Project", ColumnKind::Cell),
            col("Date", ColumnKind::Cell),
            col("Size", ColumnKind::Size),
        ],
    }
}

fn level_view(l: Level) -> LevelView {
    match l {
        Level::Recommended => LevelView::Recommended,
        Level::Optional => LevelView::Optional,
        Level::Caution => LevelView::Caution,
    }
}

/// A row with nothing filled in: the callers set what the row has.
fn blank_row(id: String, kind: RowKind) -> RowView {
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
