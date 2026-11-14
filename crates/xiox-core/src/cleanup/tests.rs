use super::*;

/// Il resoconto somma categorie, voci native e opzioni dinamiche, le ordina per spazio e
/// riporta i file rimasti, le voci con fallimenti contati e le categorie annullate.
#[test]
fn clean_report_merges_categories_natives_and_rules() {
    let mut cleaned = vec![None; cleaner::CATEGORIES.len()];
    let locked = PathBuf::from(r"C:\t\a.tmp");
    cleaned[1] = Some(cleaner::Cleaned {
        freed: 100,
        removed: 4,
        failed: 1,
        dirs: 2,
        failures: vec![(locked.clone(), "in uso".into())],
        cancelled: false,
    });
    cleaned[3] = Some(cleaner::Cleaned { cancelled: true, ..Default::default() });
    let natives = vec![(
        "native:x".to_string(),
        "Appunti".to_string(),
        native::NativeCleaned { freed: 0, removed: 1, ..Default::default() },
    )];
    let rules = vec![(
        "c/o".to_string(),
        "Chrome – Cache".to_string(),
        rules::RuleCleaned {
            freed: 500,
            removed: 12,
            files: 10,
            failed: 2,
            notes: vec!["2 file in uso".into()],
            ..Default::default()
        },
    )];
    let r = clean_report(&cleaned, &natives, &rules, &Chosen::default(), 1200, CleanEnd::Cancelled);
    assert_eq!(r.freed_bytes, 600);
    // 4 file della categoria e i 10 file della regola: non gli Appunti, non le voci di registro.
    assert_eq!(r.files, 14);
    assert_eq!(r.dirs, 2);
    assert_eq!(r.elapsed_ms, 1200);
    assert!(r.cancelled);
    assert_eq!(
        r.by_category,
        vec![("Chrome – Cache".to_string(), 500), (cleaner::CATEGORIES[1].name.to_string(), 100)]
    );
    assert_eq!(r.failed.len(), 2);
    assert_eq!(r.failed[0], (locked, "in uso".to_string()));
    assert_eq!(r.failed[1].0, PathBuf::from("Chrome – Cache"));
    assert!(r.failed[1].1.contains("2 elementi lasciati: 2 file in uso"));
    assert_eq!(r.skipped, vec![(PathBuf::from(cleaner::CATEGORIES[3].name), "Annullato".to_string())]);
    assert!(r.to_text().contains("Spazio liberato"));
}

/// Voci native e dinamiche saltate o interrotte senza fallimenti, e voci scelte che non
/// hanno mai risposto (annullate prima di partire): tutte tra i saltati, anche nel testo.
#[test]
fn clean_report_lists_skipped_interrupted_and_never_started_items() {
    let mut cleaned = vec![None; cleaner::CATEGORIES.len()];
    cleaned[1] = Some(cleaner::Cleaned { freed: 10, removed: 1, ..Default::default() });
    let mut chrome = native::NativeCleaned::default();
    chrome.note("Saltata: Chrome in esecuzione".into());
    let natives = vec![("native:chrome".to_string(), "Chrome – Cronologia".to_string(), chrome)];
    let rules = vec![(
        "c/o".to_string(),
        "Firefox – Cache".to_string(),
        rules::RuleCleaned { freed: 2048, removed: 3, files: 3, notes: vec!["Pulizia interrotta".into()], ..Default::default() },
    )];
    let chosen = Chosen {
        cats: vec![1, 2],
        others: vec![
            ("native:chrome".into(), "Chrome – Cronologia".into()),
            ("c/o".into(), "Firefox – Cache".into()),
            ("native:dns".into(), "Cache DNS".into()),
            ("x/y".into(), "Edge – Cache".into()),
        ],
    };
    let r = clean_report(&cleaned, &natives, &rules, &chosen, 10, CleanEnd::Cancelled);
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    let skipped: Vec<(String, String)> =
        r.skipped.iter().map(|(p, w)| (p.display().to_string(), w.clone())).collect();
    assert_eq!(
        skipped,
        vec![
            (cleaner::CATEGORIES[2].name.to_string(), "Annullato".to_string()),
            ("Chrome – Cronologia".to_string(), "Saltata: Chrome in esecuzione".to_string()),
            ("Firefox – Cache".to_string(), "Pulizia interrotta: liberati 2 KB".to_string()),
            ("Cache DNS".to_string(), "Annullato".to_string()),
            ("Edge – Cache".to_string(), "Annullato".to_string()),
        ]
    );
    let text = r.to_text();
    assert!(text.contains("Saltati (5):"), "{text}");
    assert!(text.contains("Chrome – Cronologia: Saltata: Chrome in esecuzione"), "{text}");
    assert!(text.contains("Edge – Cache: Annullato"), "{text}");
}

/// Thread della pulizia morto (panic) prima di `Done`: il job si chiude, il resoconto
/// si costruisce con quello che è arrivato e dice dell'errore; le voci mai arrivate
/// sono tra i saltati.
#[test]
fn dead_cleaning_thread_still_builds_the_report() {
    let mut app = CleanupState::new();
    let (tx, rx) = channel();
    app.clean_job = Some(rx);
    app.clean_view = CleanView::Cleaned;
    app.clean_cancel = Some((xiox_cleaner::Cancel::default(), Arc::new(AtomicBool::new(false))));
    app.clean_chosen = Chosen { cats: vec![1, 3], others: vec![("native:dns".into(), "Cache DNS".into())] };
    let c = cleaner::Cleaned { freed: 100, removed: 2, ..Default::default() };
    tx.send(CleanMsg::Cleaned(1, cleaner::Found::default(), c)).unwrap();
    app.poll();
    assert!(app.clean_job.is_some(), "il thread è vivo: si aspetta");
    drop(tx);
    app.poll();
    assert!(app.clean_job.is_none());
    assert!(app.clean_cancel.is_none());
    assert_eq!(app.clean_end, Some(CleanEnd::Died));
    let r = app.clean_report.as_ref().expect("resoconto");
    assert!(r.cancelled);
    assert_eq!((r.freed_bytes, r.files), (100, 2));
    assert_eq!(r.failed, vec![(PathBuf::from("Pulizia"), CLEAN_DIED.to_string())]);
    let never = "Non eseguito: pulizia interrotta da un errore interno".to_string();
    assert_eq!(
        r.skipped,
        vec![(PathBuf::from(cleaner::CATEGORIES[3].name), never.clone()), (PathBuf::from("Cache DNS"), never)]
    );
    assert!(r.to_text().contains("Pulizia interrotta da un errore interno"));
}

/// Analisi: thread morto o «Annulla» chiudono il job e il titolo dice che non è completa.
#[test]
fn dead_or_cancelled_analysis_is_marked_interrupted() {
    let mut app = CleanupState::new();
    let (tx, rx) = channel::<CleanMsg>();
    app.clean_job = Some(rx);
    app.clean_view = CleanView::Analyzed;
    drop(tx);
    app.poll();
    assert!(app.clean_job.is_none());
    assert!(app.clean_report.is_none());
    assert_eq!(app.clean_end, Some(CleanEnd::Died));
    assert_eq!(analysis_title(false, app.clean_end).0, "Analisi interrotta");

    let (tx, rx) = channel();
    app.clean_job = Some(rx);
    let cancel = xiox_cleaner::Cancel::default();
    cancel.cancel();
    app.clean_cancel = Some((cancel, Arc::new(AtomicBool::new(false))));
    tx.send(CleanMsg::Done).unwrap();
    app.poll();
    assert!(app.clean_job.is_none());
    assert_eq!(app.clean_end, Some(CleanEnd::Cancelled));
    let (title, note) = analysis_title(false, app.clean_end);
    assert_eq!(title, "Analisi annullata");
    assert!(note.unwrap().contains("le voci mancanti non sono state analizzate"));
    assert_eq!(analysis_title(false, Some(CleanEnd::Done)), ("Analisi completata", None));
    assert_eq!(analysis_title(true, None).0, "Analisi in corso…");
}
