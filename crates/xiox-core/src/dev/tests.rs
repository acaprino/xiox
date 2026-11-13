use super::*;
use xiox_model::{Artifact, ArtifactKind, BlockReason, GitState, SizeInfo};
use std::path::PathBuf;

fn report(id: &str, status: ProjectStatus, preselected: bool, bytes: u64) -> ProjectReport {
    ProjectReport {
        id: id.into(),
        root: PathBuf::from(format!(r"D:\p\{id}")),
        name: id.into(),
        ecosystems: vec![],
        agents: vec![],
        last_activity_ms: None,
        last_activity_source: None,
        inactive_days: None,
        status,
        blocked_by: vec![],
        reason: String::new(),
        artifacts: vec![Artifact {
            id: xiox_model::path_key(&PathBuf::from(format!(r"D:\p\{id}\target"))),
            path: PathBuf::from(format!(r"D:\p\{id}\target")),
            kind: ArtifactKind::RustTarget,
            label: "Build Rust (target)".into(),
            size: SizeInfo { logical_bytes: bytes, allocated_bytes: None, file_count: 1 },
            verified: true,
            class: xiox_model::Class::Build,
            level: xiox_model::Level::Recommended,
            note: String::new(),
            ambiguous: false,
            preselected,
        }],
        reclaimable_bytes: bytes,
        preselected,
        is_agent_worktree: false,
        git: GitState::default(),
    }
}

/// L'id della cartella `target` del progetto `id` di [`report`].
fn tid(id: &str) -> String {
    xiox_model::path_key(&PathBuf::from(format!(r"D:\p\{id}\target")))
}

fn ids(list: &[&str]) -> HashSet<String> {
    list.iter().map(|id| tid(id)).collect()
}

fn session(id: &str, blocked: Option<&str>, file: bool, bytes: u64) -> ArchivedSession {
    ArchivedSession {
        agent: Agent::Codex,
        id: id.into(),
        title: Some(format!("  titolo\n{id}  ")),
        cwd: Some(PathBuf::from(r"D:\Projects\demo")),
        updated_ms: None,
        file: file.then(|| PathBuf::from(format!(r"C:\s\{id}.jsonl"))),
        bytes,
        blocked: blocked.map(str::to_string),
    }
}

#[test]
fn only_free_projects_are_selectable() {
    assert!(selectable(&report("a", ProjectStatus::Inactive, true, 1)));
    assert!(selectable(&report("b", ProjectStatus::Recent, false, 1)));
    assert!(selectable(&report("c", ProjectStatus::NoSessions, false, 1)));
    for st in [ProjectStatus::InUse, ProjectStatus::Blocked, ProjectStatus::Unreachable] {
        assert!(!selectable(&report("x", st, false, 1)), "{st:?}");
    }
    let mut blocked = report("d", ProjectStatus::Recent, false, 1);
    blocked.blocked_by.push(BlockReason::DirtyGit);
    assert!(!selectable(&blocked));
    for st in [ProjectStatus::InUse, ProjectStatus::Blocked, ProjectStatus::Unreachable] {
        let mut r = report("x", st, false, 1);
        r.reason = "modifiche non salvate".into();
        let why = locked_why(&r);
        assert!(why.starts_with("Il progetto "), "{why}");
        assert_eq!(why.contains("modifiche non salvate"), st != ProjectStatus::InUse, "{why}");
    }
    let mut empty = report("e", ProjectStatus::Inactive, false, 0);
    empty.artifacts.clear();
    assert!(!selectable(&empty), "un progetto senza artefatti non ha nulla da togliere");
    assert_eq!(nothing_to_remove(&empty), Some("Nulla da togliere"));
    empty.is_agent_worktree = true;
    empty.git.main_worktree = Some(PathBuf::from(r"D:\repo"));
    empty.git.dirty = Some(false);
    assert!(selectable(&empty), "un worktree pulito si toglie intero");
    assert_eq!(nothing_to_remove(&empty), None);
}

#[test]
fn a_worktree_with_nothing_to_remove_is_greyed_out_with_the_reason() {
    let mut dirty = report("d", ProjectStatus::Inactive, true, 40);
    dirty.is_agent_worktree = true;
    dirty.artifacts.clear();
    dirty.git.main_worktree = Some(PathBuf::from(r"D:\repo"));
    dirty.git.dirty = Some(true);
    let mut unknown = dirty.clone();
    unknown.id = "u".into();
    unknown.git.dirty = None;
    let mut clean = dirty.clone();
    clean.id = "c".into();
    clean.git.dirty = Some(false);
    // Lo stesso giudizio di xiox_engine::build_plan: piano vuoto per i primi due.
    for (r, want) in [(&dirty, false), (&unknown, false), (&clean, true)] {
        let plan = dc_engine_plan_len(r);
        assert_eq!(selectable(r), want, "{}", r.id);
        assert_eq!(plan > 0, want, "{}", r.id);
    }
    assert_eq!(nothing_to_remove(&dirty), Some("Modifiche non salvate: il worktree resta"));
    assert_eq!(nothing_to_remove(&unknown), Some("Stato di git non leggibile: il worktree resta"));
    // Un worktree in uso lo dice già il suo stato.
    let mut busy = dirty.clone();
    busy.status = ProjectStatus::InUse;
    assert_eq!(nothing_to_remove(&busy), None);

    let rs = vec![dirty, unknown, clean];
    assert_eq!(preselect_reports(&rs), vec![false, false, true]);
    let mut sel = vec![false; 3];
    select_reports(&rs, &mut sel, true);
    assert_eq!(sel, vec![false, false, true]);
    assert_eq!(report_stats(&rs, &[true, true, true]), (1, 3, 40), "i byte dei worktree che restano non contano");
}

fn dc_engine_plan_len(r: &ProjectReport) -> usize {
    let scan = DevScan { worktrees: vec![r.clone()], ..Default::default() };
    let sel = xiox_app::selection_for_projects(&scan, &[r.id.clone()]);
    xiox_app::build_plan(&scan, &sel, RemoveMode::Recycle).unwrap().items.len()
}

#[test]
fn preselection_follows_the_engine_but_never_a_blocked_row() {
    let mut odd = report("c", ProjectStatus::Blocked, true, 5);
    odd.preselected = true;
    let rs = vec![report("a", ProjectStatus::Inactive, true, 1), report("b", ProjectStatus::Recent, false, 2), odd];
    assert_eq!(preselect_reports(&rs), vec![true, false, false]);
}

#[test]
fn sessions_start_unticked_and_blocked_never_chosen() {
    let ss = vec![session("1", None, false, 0), session("2", Some("sessione in uso"), true, 10), session("3", None, true, 7)];
    let mut st = state();
    st.set_scan(DevScan { archived: ss.clone(), ..Default::default() });
    assert_eq!(st.sel_sessions, vec![false, false, false], "dopo l'analisi nessuna sessione è spuntata");
    st.set_section(Section::Sessions);
    assert!(!st.can_clean(), "nulla scelto: «Avvia pulizia» resta spento");
    assert_eq!(st.selection(), (0, 3, 0));
    // «Tutti» spunta le non bloccate.
    select_sessions(&ss, &mut st.sel_sessions, true);
    assert_eq!(st.sel_sessions, vec![true, false, true]);
    assert!(st.can_clean());
    // Anche se la casella risultasse spuntata, una sessione bloccata non viene mai scelta.
    let sel = vec![true, true, false];
    let ids: Vec<&str> = chosen_sessions(&ss, &sel).map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec!["1"]);
    assert_eq!(session_stats(&ss, &[true, true, true]), (2, 3, 7));
}

#[test]
fn select_all_and_none_skip_what_cannot_go() {
    let rs = vec![report("a", ProjectStatus::Inactive, true, 1), report("b", ProjectStatus::InUse, false, 2)];
    let mut sel = vec![false, false];
    select_reports(&rs, &mut sel, true);
    assert_eq!(sel, vec![true, false]);
    select_reports(&rs, &mut sel, false);
    assert_eq!(sel, vec![false, false]);
    let ss = vec![session("1", Some("x"), true, 1), session("2", None, true, 1)];
    let mut ssel = vec![false, false];
    select_sessions(&ss, &mut ssel, true);
    assert_eq!(ssel, vec![false, true]);
}

/// Un progetto (più uno bloccato), un worktree d'agente e due sessioni.
fn three_sections() -> DevScan {
    let mut wt = report("w", ProjectStatus::Recent, false, 30);
    wt.is_agent_worktree = true;
    DevScan {
        projects: vec![report("a", ProjectStatus::Inactive, true, 100), report("b", ProjectStatus::Blocked, false, 50)],
        worktrees: vec![wt],
        archived: vec![session("1", None, false, 0), session("2", None, true, 4)],
        scanned_at_ms: xiox_app::now_ms(),
        ..Default::default()
    }
}

#[test]
fn an_old_analysis_does_not_clean_projects_or_worktrees() {
    let at = 1_800_000_000_000;
    let mut st = state();
    st.set_scan(DevScan { scanned_at_ms: at, ..three_sections() });
    st.sel_worktrees = vec![true];
    let late = at + xiox_app::MAX_SCAN_AGE_MS + 1;
    for section in [Section::Projects, Section::Worktrees] {
        st.section = section;
        st.open_confirm_at(late);
        assert!(st.confirm.is_none(), "{section:?}: nessun dialogo");
        assert_eq!(st.notice.as_deref(), Some(STALE_SCAN));
    }
    assert_eq!(STALE_SCAN, "L'analisi è troppo vecchia: premi «Analizza» di nuovo.");

    // Entro la mezz'ora la conferma si apre e l'avviso sparisce.
    st.section = Section::Projects;
    st.open_confirm_at(at + xiox_app::MAX_SCAN_AGE_MS);
    assert!(st.confirm.take().is_some());
    assert!(st.notice.is_none());

    // Le sessioni si ricontrollano dal vivo prima di cancellarle: l'età non le ferma.
    st.section = Section::Sessions;
    st.open_confirm_at(late);
    assert!(st.confirm.take().is_some());
    assert!(st.notice.is_none());

    // Un'analisi di età ignota vale come vecchia; una nuova analisi toglie l'avviso.
    st.set_scan(DevScan { scanned_at_ms: 0, ..three_sections() });
    st.section = Section::Worktrees;
    st.sel_worktrees = vec![true];
    st.open_confirm_at(at);
    assert!(st.confirm.is_none() && st.notice.is_some());
    st.set_scan(three_sections());
    assert!(st.notice.is_none());
}

#[test]
fn a_dialog_left_open_past_half_an_hour_does_not_clean() {
    let t = 1_800_000_000_000;
    let mut st = state();
    st.set_scan(DevScan { scanned_at_ms: t, ..three_sections() });
    st.sel_worktrees = vec![true];
    for section in [Section::Projects, Section::Worktrees] {
        st.set_section(section);
        st.open_confirm_at(t);
        let c = st.confirm.take().expect("aperta in tempo");
        assert!(st.accept_confirm(c, t + 31 * 60 * 1000).is_none(), "{section:?}");
        assert_eq!(st.notice.as_deref(), Some(STALE_SCAN));
        assert!(!st.busy(), "nessuna pulizia avviata");
        // Cambiare sezione toglie l'avviso.
        st.set_section(Section::Sessions);
        assert!(st.notice.is_none());
    }
    st.set_section(Section::Projects);
    st.open_confirm_at(t);
    let c = st.confirm.take().unwrap();
    assert!(st.accept_confirm(c, t + 29 * 60 * 1000).is_some());
    // Le sessioni si ricontrollano dal vivo: l'età non le ferma.
    st.set_section(Section::Sessions);
    st.open_confirm_at(t);
    let c = st.confirm.take().unwrap();
    assert!(st.accept_confirm(c, t + 31 * 60 * 1000).is_some());
    assert!(st.notice.is_none());
}

#[test]
fn stats_count_only_the_selectable_rows_of_one_section() {
    let scan = three_sections();
    let (sp, sw, ss) = (ids(&["a", "b"]), [true], [true, true]);
    // La cartella del progetto bloccato non conta anche se risultasse spuntata.
    assert_eq!(section_stats(&scan, Section::Projects, &sp, &sw, &ss), (1, 2, 100));
    assert_eq!(section_stats(&scan, Section::Worktrees, &sp, &sw, &ss), (1, 1, 30));
    assert_eq!(section_stats(&scan, Section::Sessions, &sp, &sw, &ss), (2, 2, 4));
    assert_eq!(section_stats(&scan, Section::Projects, &ids(&["b"]), &sw, &ss), (0, 2, 0));
    assert_eq!(selection_text(Section::Projects, 1), "1 cartella");
    assert_eq!(selection_text(Section::Projects, 2), "2 cartelle");
    assert_eq!(selection_text(Section::Worktrees, 3), "3 worktree");
    assert_eq!(selection_text(Section::Sessions, 665), "665 sessioni");
}

#[test]
fn the_action_list_holds_only_the_visible_section() {
    let scan = three_sections();
    let (sp, sw, ss) = (ids(&["a"]), [true], [true, true]);
    let p = section_action(&scan, Section::Projects, &sp, &sw, &ss);
    assert_eq!(p, Action { artifacts: vec![tid("a")], ..Default::default() });
    let w = section_action(&scan, Section::Worktrees, &sp, &sw, &ss);
    assert_eq!(w, Action { worktrees: vec!["w".into()], ..Default::default() });
    let s = section_action(&scan, Section::Sessions, &sp, &sw, &ss);
    assert_eq!(s, Action { sessions: vec![(Agent::Codex, "1".into()), (Agent::Codex, "2".into())], ..Default::default() });
    let sel = p.selection(&scan);
    assert_eq!((sel.artifacts, sel.worktrees), (vec![tid("a")], vec![]));
    // Nulla scelto nella sezione visibile: niente da fare, qualunque cosa ci sia nelle altre.
    assert_eq!(section_action(&scan, Section::Worktrees, &sp, &[false], &ss), Action::default());
    assert_eq!(section_action(&scan, Section::Sessions, &sp, &sw, &[false, false]), Action::default());
}

#[test]
fn the_confirmation_names_the_section_and_lists_only_its_items() {
    let mut st = state();
    st.set_scan(three_sections());
    assert_eq!((&st.sel_artifacts, &st.sel_worktrees, &st.sel_sessions), (&ids(&["a"]), &vec![false], &vec![false, false]));
    st.sel_worktrees = vec![true];
    st.sel_sessions = vec![true, true];

    st.section = Section::Sessions;
    st.open_confirm();
    let c = st.confirm.take().unwrap();
    assert!(c.plan.is_none(), "nessun progetto né worktree nella conferma delle sessioni");
    assert_eq!(c.sessions.len(), 2);
    assert!(c.rows.iter().all(|(label, _)| label.starts_with("Codex · ")), "{:?}", c.rows);
    assert_eq!(confirm_title(c.section), "Cancellare le sessioni archiviate selezionate?");
    assert_eq!(confirm_button(&c), "Cancella 2 sessioni");

    st.section = Section::Projects;
    st.open_confirm();
    let c = st.confirm.take().unwrap();
    assert!(c.sessions.is_empty(), "nessuna sessione nella conferma dei progetti");
    let plan = c.plan.as_ref().unwrap();
    assert!(!plan.items.is_empty() && plan.items.iter().all(|i| i.project_id == "a"), "{:?}", plan.items);
    assert_eq!(c.rows.len(), plan.items.len());
    assert_eq!(confirm_title(c.section), "Pulire le cartelle selezionate?");
    assert_eq!(confirm_button(&c), "Pulisci 1 cartella");

    st.section = Section::Worktrees;
    st.open_confirm();
    let c = st.confirm.take().unwrap();
    assert!(c.sessions.is_empty());
    let plan = c.plan.as_ref().unwrap();
    assert!(!plan.items.is_empty() && plan.items.iter().all(|i| i.project_id == "w"), "{:?}", plan.items);
    assert_eq!(confirm_title(c.section), "Rimuovere i worktree selezionati?");
}

#[test]
fn confirmation_texts_follow_section_and_mode() {
    assert_eq!(
        confirm_text(Section::Sessions, RemoveMode::Recycle),
        "Una copia viene messa nel Cestino prima della cancellazione, quando il file esiste."
    );
    assert_eq!(confirm_text(Section::Sessions, RemoveMode::Permanent), "Le sessioni vengono cancellate definitivamente.");
    assert_eq!(
        confirm_text(Section::Worktrees, RemoveMode::Recycle),
        "I worktree vengono rimossi definitivamente con git, anche con «Nel Cestino»; le cartelle di build dentro un worktree vanno nel Cestino."
    );
    assert_eq!(confirm_text(Section::Worktrees, RemoveMode::Permanent), "Gli elementi selezionati verranno eliminati definitivamente.");
    assert_eq!(
        confirm_text(Section::Projects, RemoveMode::Recycle),
        "Gli elementi selezionati vanno nel Cestino."
    );
    assert_eq!(confirm_text(Section::Projects, RemoveMode::Permanent), "Gli elementi selezionati verranno eliminati definitivamente.");
    let one = Confirm {
        scan: Arc::new(DevScan::default()),
        section: Section::Sessions,
        sel: Selection::default(),
        plan: None,
        cautions: vec![],
        ack: false,
        shortfall: vec![],
        sessions: vec![(Agent::Pi, "x".into())],
        rows: vec![],
        summary: vec![],
        error: None,
        mode: RemoveMode::Recycle,
        selected: 1,
        others: None,
    };
    assert_eq!(confirm_button(&one), "Cancella 1 sessione");
    let three = Confirm { section: Section::Projects, sessions: vec![], selected: 3, ..one };
    assert_eq!(confirm_button(&three), "Pulisci 3 cartelle");
}

#[test]
fn selected_text_and_clean_button_name_the_section() {
    assert_eq!(selected_text(Section::Projects, 1), "1 cartella selezionata");
    assert_eq!(selected_text(Section::Projects, 3), "3 cartelle selezionate");
    assert_eq!(selected_text(Section::Worktrees, 1), "1 worktree selezionato");
    assert_eq!(selected_text(Section::Worktrees, 2), "2 worktree selezionati");
    assert_eq!(selected_text(Section::Sessions, 1), "1 sessione selezionata");
    assert_eq!(selected_text(Section::Sessions, 5), "5 sessioni selezionate");

    assert_eq!(clean_button(Section::Projects, 0), "Pulisci");
    assert_eq!(clean_button(Section::Worktrees, 0), "Rimuovi");
    assert_eq!(clean_button(Section::Sessions, 0), "Cancella");
    assert_eq!(clean_button(Section::Projects, 1), "Pulisci 1 cartella");
    assert_eq!(clean_button(Section::Projects, 3), "Pulisci 3 cartelle");
    assert_eq!(clean_button(Section::Worktrees, 1), "Rimuovi 1 worktree");
    assert_eq!(clean_button(Section::Worktrees, 2), "Rimuovi 2 worktree");
    assert_eq!(clean_button(Section::Sessions, 1), "Cancella 1 sessione");
    assert_eq!(clean_button(Section::Sessions, 5), "Cancella 5 sessioni");
}

#[test]
fn the_confirmation_says_what_it_leaves_out() {
    let mut st = state();
    st.set_scan(three_sections());
    st.sel_worktrees = vec![true];
    assert_eq!((st.sel_artifacts.len(), st.sel_sessions.clone()), (1, vec![false, false]));
    st.sel_sessions = vec![true, true];

    // Progetti: le righe scelte in Worktree e Sessioni restano fuori dal piano e sono nominate.
    st.set_section(Section::Projects);
    st.open_confirm();
    let c = st.confirm.take().unwrap();
    assert_eq!(c.others.as_deref(), Some("Non tocca le altre sezioni: 1 worktree selezionato e 2 sessioni selezionate."));
    assert_eq!(c.selected, 1);
    assert_eq!(confirm_button(&c), "Pulisci 1 cartella");
    let plan = c.plan.as_ref().unwrap();
    assert!(!plan.items.is_empty() && plan.items.iter().all(|i| i.project_id == "a"), "{:?}", plan.items);
    assert!(c.sessions.is_empty());

    // Sessioni: Progetti e Worktree, nell'ordine.
    st.set_section(Section::Sessions);
    st.open_confirm();
    let c = st.confirm.take().unwrap();
    assert_eq!(c.others.as_deref(), Some("Non tocca le altre sezioni: 1 cartella selezionata e 1 worktree selezionato."));
    assert_eq!(confirm_button(&c), "Cancella 2 sessioni");

    // Una sola altra sezione con qualcosa di scelto: una sola voce.
    st.set_section(Section::Worktrees);
    st.sel_sessions = vec![false, false];
    st.open_confirm();
    let c = st.confirm.take().unwrap();
    assert_eq!(c.others.as_deref(), Some("Non tocca le altre sezioni: 1 cartella selezionata."));
    assert_eq!(confirm_button(&c), "Rimuovi 1 worktree");

    // Le altre sezioni vuote: nessuna nota.
    st.sel_artifacts.clear();
    st.open_confirm();
    let c = st.confirm.take().unwrap();
    assert_eq!(c.others, None);
}

#[test]
fn labels() {
    assert_eq!(agent_label(Agent::Opencode), "OpenCode");
    assert_eq!(agent_label(Agent::Claude), "Claude Code");
    let s = session("abc", None, false, 0);
    assert_eq!(session_title(&s), "titolo abc");
    assert_eq!(session_label(&s), "Codex · titolo abc");
    assert_eq!(session_size(&s), "solo riga nel database");
    assert_eq!(session_size(&session("x", None, true, 2048)), "2 KB");
    let mut untitled = s.clone();
    untitled.title = Some("   ".into());
    assert_eq!(session_title(&untitled), "(senza titolo)");
    assert_eq!(section_label(Section::Sessions, Some(664)), "Sessioni archiviate (664)");
    assert_eq!(section_label(Section::Worktrees, None), "Worktree");
    assert_eq!(plural(1, "cartella", "cartelle"), "1 cartella");
    assert_eq!(plural(0, "cartella", "cartelle"), "0 cartelle");
    assert_eq!(fmt_elapsed(58_400), "Analisi in 58 s");
    assert_eq!(fmt_elapsed(125_000), "Analisi in 2 min 5 s");
}

#[test]
fn plan_labels_name_the_project_and_the_artifact() {
    let mut wt = report("wt", ProjectStatus::Inactive, true, 9);
    wt.is_agent_worktree = true;
    let scan = DevScan { projects: vec![report("app", ProjectStatus::Inactive, true, 1)], worktrees: vec![wt], ..Default::default() };
    let item = |path: &str, id: &str, action| PlanItem {
        path: PathBuf::from(path),
        kind: ArtifactKind::RustTarget,
        action,
        expected_bytes: 0,
        project_id: id.into(),
        reason: String::new(),
    };
    assert_eq!(plan_label(&scan, &item(r"D:\p\app\target", "app", PlanAction::RemoveDir)), "app · Build Rust (target)");
    let remove_wt = PlanAction::GitWorktreeRemove { repo: PathBuf::from(r"D:\repo") };
    assert_eq!(plan_label(&scan, &item(r"D:\p\wt", "wt", remove_wt)), "wt · worktree intero");
    assert_eq!(plan_label(&scan, &item(r"D:\x\node_modules", "?", PlanAction::RemoveDir)), "node_modules · node_modules");
}

#[test]
fn dates_are_day_month_year() {
    // Mezzogiorno UTC: lo stesso giorno in ogni fuso tra -11 e +11.
    assert_eq!(fmt_date(1_762_776_000_000), "10/11/2025");
    assert_eq!(fmt_date(1_709_208_000_000), "29/02/2024");
}

#[test]
fn a_failed_analysis_is_not_an_empty_one() {
    let ok = DevScan { warnings: vec!["Sessioni di Pi illeggibili".into()], ..Default::default() };
    assert!(!analysis_failed(&ok));
    assert_eq!(empty_text(Section::Projects, false), "Nessun progetto trovato.");
    let failed = DevScan { warnings: vec!["altro".into(), ANALYSIS_FAILED.into()], ..Default::default() };
    assert!(analysis_failed(&failed));
    assert_eq!(ordered_warnings(&failed.warnings), vec![ANALYSIS_FAILED, "altro"]);
    for s in [Section::Projects, Section::Worktrees] {
        assert!(empty_text(s, true).contains("non riuscita"), "{s:?}");
    }
    assert_eq!(empty_text(Section::Sessions, true), "Nessuna sessione archiviata.");
}

#[test]
fn sections_from_the_command_line() {
    assert_eq!(Section::parse("projects"), Some(Section::Projects));
    assert_eq!(Section::parse("worktree"), Some(Section::Worktrees));
    assert_eq!(Section::parse("sessions"), Some(Section::Sessions));
    assert_eq!(Section::parse("boh"), None);
}

fn state() -> DevState {
    DevState::with_settings(xiox_app::default_settings(), Section::Projects, false)
}

#[test]
fn scan_result_preselects_and_release_frees_it() {
    let mut st = state();
    let (tx, rx) = channel();
    st.job = Some(rx);
    tx.send(DevMsg::Step("Lettura delle sessioni…".into(), 0.05)).unwrap();
    st.poll();
    assert!(st.busy());
    assert_eq!(st.text, "Lettura delle sessioni…");

    // Mentre il lavoro è in corso release() non tocca nulla.
    st.release();
    assert!(st.busy());

    let scan = DevScan {
        projects: vec![report("a", ProjectStatus::Inactive, true, 10), report("b", ProjectStatus::Recent, false, 5)],
        archived: vec![session("1", None, false, 0), session("2", Some("sessione in uso"), true, 3)],
        ..Default::default()
    };
    tx.send(DevMsg::Scanned(scan)).unwrap();
    st.poll();
    assert!(!st.busy());
    assert_eq!(st.view, View::Scanned);
    assert_eq!(st.sel_artifacts, ids(&["a"]));
    assert_eq!(st.sel_sessions, vec![false, false], "le sessioni partono non spuntate");
    assert_eq!(st.selection(), (1, 2, 10));

    st.release();
    assert!(st.scan.is_none());
    assert_eq!(st.view, View::Empty);
    assert!(st.sel_artifacts.is_empty() && st.sel_sessions.is_empty());
}

#[test]
fn a_dead_job_is_reported_not_left_spinning() {
    let mut st = state();
    let (tx, rx) = channel::<DevMsg>();
    st.job = Some(rx);
    drop(tx);
    st.poll();
    assert!(!st.busy());
    assert_eq!(st.error.as_deref(), Some("Il lavoro si è interrotto per un errore interno."));
    assert_eq!(st.view, View::Empty);
}

#[test]
fn a_dead_cleanup_discards_the_scan_and_disables_the_button() {
    let mut st = state();
    st.set_scan(three_sections());
    assert!(st.can_clean());
    // Come dopo «Continua»: vista dell'esito, lavoro in corso, poi il thread muore senza esito.
    st.view = View::Cleaned;
    let (tx, rx) = channel::<DevMsg>();
    st.job = Some(rx);
    assert!(!st.can_clean());
    drop(tx);
    st.poll();
    assert!(!st.busy());
    assert!(st.scan.is_none());
    assert!(st.sel_artifacts.is_empty() && st.sel_worktrees.is_empty() && st.sel_sessions.is_empty());
    assert!(!st.can_clean());
    assert_eq!(st.error.as_deref(), Some(CLEANUP_DIED));
    assert!(CLEANUP_DIED.contains("potrebbero essere già stati rimossi"));
    assert_eq!(st.view, View::Empty);
}

#[test]
fn a_panic_keeps_the_outcomes_collected_so_far() {
    let mut out = DevCleaned::new();
    guarded(&mut out, |o| {
        add_session_outcome(&mut o.report, "Codex · a", &ItemOutcome::Removed { bytes_freed: 7 }, RemoveMode::Permanent);
    });
    assert!(!out.interrupted);
    guarded(&mut out, |o| {
        add_session_outcome(&mut o.report, "Codex · b", &ItemOutcome::Skipped { reason: "in uso".into() }, RemoveMode::Permanent);
        panic!("prova");
    });
    assert!(out.interrupted);
    assert_eq!(out.report.skipped.len(), 1);
    assert_eq!(out.report.freed_bytes, 7);
    assert_eq!(cleaned_title(Some(&out), false), "Pulizia interrotta");
}

#[test]
fn the_confirmation_uses_the_chosen_mode_and_keeps_its_scan() {
    let mut st = state();
    st.set_scan(three_sections());
    for mode in [RemoveMode::Permanent, RemoveMode::Recycle] {
        st.settings.remove_mode = mode;
        st.open_confirm();
        let c = st.confirm.take().unwrap();
        assert_eq!(c.mode, mode);
        assert!(Arc::ptr_eq(&c.scan, st.scan.as_ref().unwrap()));
    }
    // Una nuova analisi arrivata dopo l'apertura non cambia la conferma già aperta.
    st.open_confirm();
    let old = st.scan.clone().unwrap();
    st.set_scan(three_sections());
    let c = st.confirm.as_ref().unwrap();
    assert!(Arc::ptr_eq(&c.scan, &old) && !Arc::ptr_eq(&c.scan, st.scan.as_ref().unwrap()));
}

#[test]
fn cleaning_result_drops_the_scan() {
    let mut st = state();
    st.set_scan(DevScan { projects: vec![report("a", ProjectStatus::Inactive, true, 10)], ..Default::default() });
    let (tx, rx) = channel();
    st.job = Some(rx);
    st.tick = Some(Tick { done_bytes: 0, total_bytes: 10, done_items: 0, total_items: 1 });
    st.cancel = Some(Cancel::default());
    tx.send(DevMsg::Tick(Tick { done_bytes: 5, total_bytes: 10, done_items: 0, total_items: 1 })).unwrap();
    st.poll();
    assert_eq!(st.tick.map(|t| t.done_bytes), Some(5));
    let mut c = DevCleaned::new();
    c.report.freed_bytes = 10;
    c.report.by_category = vec![("Build Rust (target)".into(), 10)];
    tx.send(DevMsg::Cleaned(c)).unwrap();
    st.poll();
    assert!(!st.busy());
    assert!(st.tick.is_none() && st.cancel.is_none());
    assert!(st.scan.is_none());
    assert_eq!(st.view, View::Cleaned);
    assert_eq!(st.cleaned.as_ref().map(|c| c.report.freed_bytes), Some(10));
    assert_eq!(st.selection(), (0, 0, 0));
}

/// Un progetto inattivo con tre cartelle: target consigliato e preselezionato,
/// node_modules facoltativo, dist al livello Attenzione.
fn levels_project() -> ProjectReport {
    let mut r = report("web", ProjectStatus::Inactive, true, 100);
    let base = r.artifacts[0].clone();
    let mk = |dir: &str, label: &str, class, level, note: &str, bytes| {
        let path = PathBuf::from(format!(r"D:\p\web\{dir}"));
        Artifact {
            id: path_key(&path),
            path,
            label: label.into(),
            class,
            level,
            note: note.into(),
            preselected: false,
            size: SizeInfo { logical_bytes: bytes, allocated_bytes: None, file_count: 1 },
            ..base.clone()
        }
    };
    r.artifacts.push(mk(
        "node_modules",
        "Dipendenze Node (node_modules)",
        xiox_model::Class::Dependency,
        Level::Optional,
        "Si riscarica con npm install (o pnpm, yarn, bun install)",
        50,
    ));
    r.artifacts.push(mk(
        "dist",
        "Build JavaScript (dist)",
        xiox_model::Class::Build,
        Level::Caution,
        "Potrebbe contenere sorgenti: git non la ignora",
        20,
    ));
    r.artifacts[1].kind = ArtifactKind::NodeModules;
    r.artifacts[2].kind = ArtifactKind::Catalog;
    r.reclaimable_bytes = 170;
    r
}

#[test]
fn the_project_checkbox_has_three_states() {
    let r = levels_project();
    let mut sel = preselect_artifacts(std::slice::from_ref(&r));
    assert_eq!(sel, ids(&["web"]), "solo target è preselezionato");
    assert_eq!(tri_state(&r, &sel), Tri::Some);
    // «alcune» → tutte
    toggle_project(&r, &mut sel);
    assert_eq!(tri_state(&r, &sel), Tri::All);
    assert_eq!(sel.len(), 3);
    // «tutte» → nessuna
    toggle_project(&r, &mut sel);
    assert_eq!(tri_state(&r, &sel), Tri::None);
    assert!(sel.is_empty());
    // «nessuna» → tutte
    toggle_project(&r, &mut sel);
    assert_eq!(tri_state(&r, &sel), Tri::All);

    // Un progetto in uso non cambia e non conta, ma le sue cartelle restano elencate.
    let mut busy = levels_project();
    busy.status = ProjectStatus::InUse;
    let mut none = HashSet::new();
    toggle_project(&busy, &mut none);
    assert!(none.is_empty());
    assert!(preselect_artifacts(std::slice::from_ref(&busy)).is_empty());
    assert_eq!(artifact_stats(std::slice::from_ref(&busy), &sel), (0, 3, 0));
}

#[test]
fn the_total_follows_each_folder() {
    let rs = vec![levels_project()];
    let mut sel = preselect_artifacts(&rs);
    assert_eq!(artifact_stats(&rs, &sel), (1, 3, 100));
    set_artifact(&mut sel, &rs[0].artifacts[1], true);
    assert_eq!(artifact_stats(&rs, &sel), (2, 3, 150));
    set_artifact(&mut sel, &rs[0].artifacts[0], false);
    assert_eq!(artifact_stats(&rs, &sel), (1, 3, 50));
    select_all_artifacts(&rs, &mut sel, true);
    assert_eq!(artifact_stats(&rs, &sel), (3, 3, 170));
    select_all_artifacts(&rs, &mut sel, false);
    assert_eq!(artifact_stats(&rs, &sel), (0, 3, 0));
}

#[test]
fn levels_have_their_words() {
    assert_eq!(level_word(Level::Recommended), "Consigliato");
    assert_eq!(level_word(Level::Optional), "Facoltativo");
    assert_eq!(level_word(Level::Caution), "Attenzione");
}

fn levels_state() -> DevState {
    let mut st = state();
    st.set_scan(DevScan { projects: vec![levels_project()], scanned_at_ms: xiox_app::now_ms(), ..Default::default() });
    st
}

#[test]
fn a_caution_folder_needs_the_warnings_read() {
    let mut st = levels_state();
    // Solo target: niente avvisi, «Pulisci» subito.
    st.open_confirm_with(xiox_app::now_ms(), |_| vec![]);
    let c = st.confirm.take().unwrap();
    assert!(c.cautions.is_empty());
    assert!(confirm_ready(&c));
    assert_eq!(c.plan.as_ref().unwrap().items.len(), 1, "solo la cartella scelta");

    // Con dist: elencata con il suo avviso, «Pulisci» spento finché non si legge.
    st.sel_artifacts.insert(tid("web").replace("target", "dist"));
    st.open_confirm_with(xiox_app::now_ms(), |_| vec![]);
    let mut c = st.confirm.take().unwrap();
    assert_eq!(c.plan.as_ref().unwrap().items.len(), 2);
    assert_eq!(c.cautions, vec![("web · Build JavaScript (dist)".to_string(), "Potrebbe contenere sorgenti: git non la ignora".to_string())]);
    assert!(!confirm_ready(&c));
    c.ack = true;
    assert!(confirm_ready(&c));
    assert_eq!(caution_title(1), "1 cartella al livello Attenzione:");
    assert_eq!(caution_title(2), "2 cartelle al livello Attenzione:");
}

#[test]
fn a_small_recycle_bin_offers_permanent_deletion() {
    let mut st = levels_state();
    st.settings.remove_mode = RemoveMode::Recycle;
    st.open_confirm_with(xiox_app::now_ms(), |p| vec![('D', p.expected_bytes, 0)]);
    let mut c = st.confirm.take().unwrap();
    assert_eq!(c.shortfall, vec![('D', 100, 0)]);
    assert_eq!(
        shortfall_text(&c.shortfall),
        "Il Cestino di D: ha 0 byte liberi, la pulizia ne sposta 100 byte. Eliminare definitivamente questa volta?"
    );
    // Il pulsante è «Definitiva»: il testo non parla del Cestino.
    assert_eq!(c.mode, RemoveMode::Recycle);
    assert_eq!(shown_mode(&c), RemoveMode::Permanent);
    assert_eq!(confirm_text(c.section, shown_mode(&c)), "Gli elementi selezionati verranno eliminati definitivamente.");
    make_permanent(&mut c).unwrap();
    assert_eq!(c.mode, RemoveMode::Permanent);
    assert_eq!(c.plan.as_ref().unwrap().mode, RemoveMode::Permanent);
    assert_eq!(c.plan.as_ref().unwrap().items.len(), 1, "lo stesso piano");
    assert!(c.shortfall.is_empty());
    let two = [('C', 10 << 30, 2 << 30), ('E', 3 << 20, 0)];
    assert_eq!(
        shortfall_text(&two),
        "Il Cestino di C: ha 2,00 GB liberi, la pulizia ne sposta 10,00 GB. Il Cestino di E: ha 0 byte liberi, la pulizia ne sposta 3,0 MB. Eliminare definitivamente questa volta?"
    );
}

#[test]
fn the_report_adds_up_folders_and_sessions() {
    let mut out = DevCleaned::new();
    assert!(out.is_empty());
    assert_eq!(cleaned_title(Some(&out), false), "Pulizia completata");
    merge_report(
        &mut out.report,
        CleanReport {
            freed_bytes: 100,
            files: 4,
            dirs: 2,
            elapsed_ms: 1500,
            by_category: vec![("Build Rust (target)".into(), 100)],
            skipped: vec![(PathBuf::from(r"D:\p\web\dist"), "Annullato".into())],
            cancelled: true,
            ..Default::default()
        },
    );
    out.labels.insert(path_key(Path::new(r"D:\p\web\dist")), "web · Build JavaScript (dist)".into());
    add_session_outcome(&mut out.report, "Codex · x", &ItemOutcome::Removed { bytes_freed: 300 }, RemoveMode::Recycle);
    let f = ItemOutcome::Failed { error: "bloccato".into(), locked_by: vec!["code.exe".into()] };
    add_session_outcome(&mut out.report, "Codex · y", &f, RemoveMode::Recycle);
    let r = &out.report;
    assert_eq!((r.freed_bytes, r.recycled_bytes, r.files, r.dirs), (100, 300, 5, 2));
    assert_eq!(r.by_category, vec![(SESSIONS_CATEGORY.to_string(), 300), ("Build Rust (target)".to_string(), 100)]);
    assert_eq!(r.failed, vec![(PathBuf::from("Codex · y"), "bloccato (in uso da: code.exe)".to_string())]);
    assert_eq!(out.label(Path::new(r"D:\p\web\dist")), "web · Build JavaScript (dist)");
    assert_eq!(cleaned_title(Some(&out), false), "Pulizia annullata");
    assert_eq!(cleaned_title(Some(&out), true), "Pulizia in corso…");
    assert_eq!(recycled_text(300), "300 byte nel Cestino: si liberano svuotandolo");
    let text = r.to_text();
    assert!(text.contains("Spazio liberato") && text.contains("Saltati (1)") && text.contains("Non riusciti (1)"), "{text}");

    let mut refused = DevCleaned::new();
    refused.errors.push("Piano rifiutato".into());
    assert_eq!(cleaned_title(Some(&refused), false), "Pulizia non eseguita");
}

#[test]
fn progress_text_and_bar() {
    let t = Tick { done_bytes: 512, total_bytes: 2048, done_items: 1, total_items: 4 };
    assert_eq!(tick_text(&t), "512 byte di 2 KB · 1 di 4 voci");
    assert_eq!(tick_fraction(&t), 0.25);
    let unknown = Tick { done_bytes: 0, total_bytes: 0, done_items: 1, total_items: 2 };
    assert_eq!(tick_fraction(&unknown), 0.5);
    assert_eq!(tick_text(&Tick { total_items: 1, ..unknown }), "0 byte di 0 byte · 1 di 1 voce");
    assert_eq!(fmt_duration(850), "850 ms");
    assert_eq!(fmt_duration(12_300), "12 s");
    assert_eq!(fmt_duration(125_000), "2 min 05 s");
}

#[test]
fn cancel_stops_the_running_cleanup() {
    let mut st = state();
    assert!(!st.cancelling());
    st.cancel = Some(Cancel::default());
    let shared = st.cancel.clone().unwrap();
    st.cancel_clean();
    assert!(st.cancelling() && shared.is_cancelled(), "lo stesso Cancel dei worker");
}

/// Un report con la radice indicata.
fn report_at(id: &str, root: &str) -> ProjectReport {
    let mut r = report(id, ProjectStatus::Inactive, true, 1);
    r.root = PathBuf::from(root);
    r
}

/// Un'analisi con roba dentro `D:\t\albero` e fuori (anche un fratello `albero-x`) e due sessioni.
fn mixed_scan() -> DevScan {
    let mut wt_in = report_at("wi", r"D:\t\albero\wt");
    wt_in.is_agent_worktree = true;
    let mut wt_out = report_at("wo", r"D:\Projects\vero-wt");
    wt_out.is_agent_worktree = true;
    DevScan {
        projects: vec![report_at("pi", r"D:\t\albero\p1"), report_at("po", r"D:\Projects\vero"), report_at("px", r"D:\t\albero-x\p2")],
        worktrees: vec![wt_in, wt_out],
        other_worktrees: vec![report_at("oi", r"D:\t\albero\o1"), report_at("oo", r"D:\altrove\o2")],
        archived: vec![session("1", None, true, 4), session("2", None, true, 5)],
        scanned_at_ms: xiox_app::now_ms(),
        ..Default::default()
    }
}

fn report_ids(rs: &[ProjectReport]) -> Vec<&str> {
    rs.iter().map(|r| r.id.as_str()).collect()
}

#[test]
fn keep_only_under_drops_everything_outside_the_root() {
    let mut scan = mixed_scan();
    keep_only_under(&mut scan, Path::new(r"D:\t\albero"));
    assert_eq!(report_ids(&scan.projects), vec!["pi"], "il fratello albero-x non sta sotto albero");
    assert_eq!(report_ids(&scan.worktrees), vec!["wi"]);
    assert_eq!(report_ids(&scan.other_worktrees), vec!["oi"]);
    assert!(scan.archived.is_empty(), "le sessioni non hanno una cartella nell'albero finto");

    // La radice stessa resta; maiuscole e barre non cambiano il giudizio.
    let mut scan = DevScan { projects: vec![report_at("r", r"D:\t\albero"), report_at("c", "d:/T/ALBERO/sub")], ..Default::default() };
    keep_only_under(&mut scan, Path::new(r"D:\t\albero"));
    assert_eq!(report_ids(&scan.projects), vec!["r", "c"]);
}

#[test]
fn only_root_never_keeps_real_sessions_or_worktrees() {
    let mut fake = DevState::only_root(PathBuf::from(r"D:\t\albero"), Section::Sessions, false);
    fake.set_scan(mixed_scan());
    let scan = fake.scan.clone().unwrap();
    assert!(scan.archived.is_empty() && fake.sel_sessions.is_empty());
    assert_eq!(report_ids(&scan.worktrees), vec!["wi"], "solo il worktree dentro l'albero finto");
    assert_eq!(fake.sel_worktrees.len(), 1);
    assert_eq!(report_ids(&scan.projects), vec!["pi"]);
    assert_eq!(report_ids(&scan.other_worktrees), vec!["oi"]);

    // Uno stato normale tiene tutto.
    let mut normal = state();
    normal.set_scan(mixed_scan());
    let scan = normal.scan.clone().unwrap();
    assert_eq!(scan.archived.len(), 2);
    assert_eq!(scan.worktrees.len(), 2);
    assert_eq!(scan.projects.len(), 3);
    assert_eq!(scan.other_worktrees.len(), 2);
}

#[test]
fn only_root_keeps_everything_in_memory() {
    let dir = PathBuf::from(r"Z:\albero\finto");
    let s = only_root_settings(xiox_app::default_settings(), dir.clone());
    assert!(!s.session_projects && s.scan_roots && !s.all_drives);
    assert_eq!(s.roots, vec![dir.clone()]);
    let mut st = DevState::only_root(dir, Section::Projects, false);
    assert!(!st.persist);
    st.settings.inactive_days = 3;
    st.save();
    assert!(st.settings_err.is_none());
}

/// `--dev-demo` cancella per davvero: solo con `--only-root` su un albero finto.
#[cfg(debug_assertions)]
#[test]
fn demo_runs_only_on_a_fake_tree() {
    // Fuori dalla cartella temporanea: serve il file segnale.
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let real = tempfile::Builder::new().prefix("demo-vero-").tempdir_in(&base).unwrap();
    let mut st = DevState::only_root(real.path().to_path_buf(), Section::Projects, false);
    let e = st.set_demo(Demo::Clean).unwrap_err();
    assert!(e.contains(DEMO_MARKER), "{e}");
    assert!(st.demo.is_none());
    std::fs::write(real.path().join(DEMO_MARKER), "").unwrap();
    st.set_demo(Demo::Clean).unwrap();
    assert_eq!(st.demo, Some(Demo::Clean));

    // Dentro la cartella temporanea basta --only-root, ma non la cartella temporanea stessa.
    let tmp = tempfile::tempdir().unwrap();
    let mut st = DevState::only_root(tmp.path().to_path_buf(), Section::Projects, false);
    st.set_demo(Demo::Confirm).unwrap();
    let mut st = DevState::only_root(std::env::temp_dir(), Section::Projects, false);
    assert!(st.set_demo(Demo::Confirm).is_err());
    let mut st = DevState::only_root(real.path().join("manca"), Section::Projects, false);
    assert!(st.set_demo(Demo::Confirm).is_err(), "cartella inesistente");

    // Senza --only-root (le radici vere dell'utente) mai, nemmeno con il segnale.
    let mut normal = state();
    normal.settings.roots = vec![real.path().to_path_buf()];
    assert!(normal.set_demo(Demo::Clean).is_err());
    assert!(normal.demo.is_none());
    // Le radici cambiate dopo l'avvio: il passo della dimostrazione si ferma.
    assert!(demo_allowed(false, &[real.path().to_path_buf(), tmp.path().to_path_buf()]).is_err());
}
