//! Voce «Sviluppatori»: il disegno egui. Lo stato (analisi, selezione, conferma, pulizia) sta in
//! `xiox_core::dev` e qui si disegna soltanto; il disegno sparisce con l'interfaccia egui.

use crate::theme::*;
use crate::{sys, Repaint};
use eframe::egui::collapsing_header::{paint_default_icon, CollapsingState};
use eframe::egui::{self, Align, Color32, Layout, Margin, RichText, Stroke, Ui, Vec2};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use xiox_app::ANALYSIS_FAILED;
use xiox_core::dev::*;
use xiox_core::notify::SharedNotifier;
use xiox_model::{path_key, Artifact, CleanReport, Level, ProjectReport, RemoveMode};
use xiox_mukade::ArchivedSession;

pub use xiox_core::dev::{DevState, Section};
#[cfg(debug_assertions)]
pub use xiox_core::dev::Demo;

const ROW_H: f32 = 22.0;
const CHECK_W: f32 = 22.0;
/// Spazio lasciato alla barra di scorrimento a destra delle tabelle.
const SCROLLBAR: f32 = 12.0;

/// I thread di lavoro svegliano la UI egui con una richiesta di ridisegno.
fn repaint(ctx: &egui::Context) -> SharedNotifier {
    Arc::new(Repaint(ctx.clone()))
}

/// Il colore del livello di una cartella.
fn level_color(level: Level) -> Color32 {
    match level {
        Level::Recommended => GREEN,
        Level::Optional => ACCENT,
        Level::Caution => RED,
    }
}


fn options(state: &mut DevState, ui: &mut Ui, busy: bool) {
    group(ui, "Sezione", 6.0);
    let counts = state.scan.as_ref().map(|s| [s.projects.len(), s.worktrees.len(), s.archived.len()]);
    let all = [Section::Projects, Section::Worktrees, Section::Sessions];
    // Le spunte di ogni sezione, anche di quelle non visibili.
    let picked = all.map(|s| match &state.scan {
        Some(scan) => section_stats(scan, s, &state.sel_artifacts, &state.sel_worktrees, &state.sel_sessions).0,
        None => 0,
    });
    for (k, s) in all.into_iter().enumerate() {
        let text = section_label(s, counts.map(|c| c[k]));
        let on = state.section == s;
        let btn = egui::Button::new(RichText::new(text).color(if on { ACCENT } else { TEXT }).strong())
            .fill(if on { SEL_BG } else { CARD })
            .stroke(Stroke::NONE)
            .min_size(Vec2::new(ui.available_width(), 28.0));
        if ui.add(btn).clicked() {
            state.set_section(s);
        }
        if picked[k] > 0 {
            ui.label(RichText::new(selected_text(s, picked[k])).size(12.0).color(MUTED));
        }
    }

    let mut changed = false;
    group(ui, "Opzioni", 12.0);
    ui.add_enabled_ui(!busy, |ui| {
        ui.horizontal(|ui| {
            ui.label("Giorni di inattività");
            let r = ui.add(egui::DragValue::new(&mut state.settings.inactive_days).range(1..=365));
            if (r.changed() && !r.dragged()) || r.drag_stopped() {
                changed = true;
            }
        });
    });

    group(ui, "Eliminazione", 12.0);
    ui.add_enabled_ui(!busy, |ui| {
        changed |= ui.radio_value(&mut state.settings.remove_mode, RemoveMode::Recycle, "Nel Cestino (recuperabile)").changed();
        changed |= ui.radio_value(&mut state.settings.remove_mode, RemoveMode::Permanent, "Definitiva").changed();
    });
    ui.add(
        egui::Label::new(
            RichText::new("Il Cestino non libera spazio finché non lo svuoti. «Cestino» in Pulizia file lo svuota.")
                .size(12.0)
                .color(MUTED),
        )
        .wrap(),
    );

    group(ui, "Cartelle aggiuntive", 12.0);
    ui.add_enabled_ui(!busy, |ui| {
        changed |= ui.checkbox(&mut state.settings.scan_roots, "Analizza anche queste cartelle").changed();
        ui.add_space(2.0);
        for (i, root) in state.settings.roots.iter().enumerate() {
            let text = root.display().to_string();
            let on = state.root_sel == Some(i);
            let r = ui
                .add(egui::Button::selectable(on, RichText::new(&text).color(if on { Color32::WHITE } else { TEXT })).truncate())
                .on_hover_text(&text);
            if r.clicked() {
                state.root_sel = if on { None } else { Some(i) };
            }
        }
        if state.settings.roots.is_empty() {
            ui.label(RichText::new("Nessuna cartella.").color(MUTED));
        }
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            if ui.button("Aggiungi cartella…").clicked() {
                if let Some(dir) = rfd::FileDialog::new().set_title("Cartella da analizzare").pick_folder() {
                    if !state.settings.roots.iter().any(|r| path_key(r) == path_key(&dir)) {
                        state.settings.roots.push(dir);
                        changed = true;
                    }
                }
            }
            let can = state.root_sel.is_some_and(|i| i < state.settings.roots.len());
            if ui.add_enabled(can, egui::Button::new("Togli")).clicked() {
                if let Some(i) = state.root_sel.take() {
                    state.settings.roots.remove(i);
                    changed = true;
                }
            }
        });
    });
    if changed {
        state.save();
    }
    if let Some(e) = &state.settings_err {
        ui.add(egui::Label::new(RichText::new(format!("Impostazioni non salvate: {e}")).size(12.0).color(RED)).wrap());
    }
}

fn body(state: &mut DevState, ui: &mut Ui, busy: bool) {
    if busy {
        match state.tick {
            Some(t) => {
                let text = if state.cancelling() { "Annullamento in corso…" } else { "Pulizia in corso…" };
                ui.label(RichText::new(text).size(16.0).color(TEXT));
                ui.add_space(6.0);
                ui.add(egui::ProgressBar::new(tick_fraction(&t)).fill(ACCENT).text(tick_text(&t)));
                ui.add_space(8.0);
                if secondary(ui, "Annulla", !state.cancelling()).clicked() {
                    state.cancel_clean();
                }
                ui.add_space(10.0);
            }
            None => {
                ui.label(RichText::new(&state.text).size(16.0).color(TEXT));
                ui.add_space(6.0);
                ui.add(egui::ProgressBar::new(state.progress).fill(ACCENT).show_percentage());
                ui.add_space(10.0);
            }
        }
    }
    match state.view {
        View::Empty if busy => {
            ui.label(RichText::new("L'analisi può durare un minuto: intanto puoi usare le altre voci.").color(MUTED));
        }
        View::Empty => {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Pulizia per sviluppatori").size(22.0).strong().color(TEXT));
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Premi «Analizza»: i progetti vengono dalle sessioni di Claude Code, Codex, OpenCode e Pi.")
                        .color(MUTED),
                );
                if let Some(e) = &state.error {
                    ui.add_space(8.0);
                    ui.label(RichText::new(e).color(RED));
                }
            });
        }
        View::Scanned => scanned_view(state, ui),
        View::Cleaned if busy && state.tick.is_some() => {}
        View::Cleaned => cleaned_view(state, ui, busy),
    }
}

fn scanned_view(state: &mut DevState, ui: &mut Ui) {
    let Some(scan) = state.scan.clone() else { return };
    let (selected, total, bytes) = state.selection();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Analisi completata").size(20.0).strong().color(TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let none = ui.small_button("Nessuno").clicked();
            let all = ui.small_button("Tutti").clicked();
            if none || all {
                match state.section {
                    Section::Projects => select_all_artifacts(&scan.projects, &mut state.sel_artifacts, all),
                    Section::Worktrees => select_reports(&scan.worktrees, &mut state.sel_worktrees, all),
                    Section::Sessions => select_sessions(&scan.archived, &mut state.sel_sessions, all),
                }
            }
        });
    });
    ui.label(
        RichText::new(format!("{} da liberare · {selected} di {total} selezionati", sys::fmt_size(bytes)))
            .size(15.0)
            .color(ACCENT),
    );
    let how = if scan.backend == "mft" { "dimensioni dalla MFT del disco" } else { "cartelle misurate una per una" };
    ui.label(RichText::new(format!("{} · {how}", fmt_elapsed(scan.elapsed_ms))).size(12.0).color(MUTED));
    match state.section {
        Section::Projects => {
            ui.label(
                RichText::new("Ogni cartella ha la sua casella: «Consigliato» è proposto nei progetti inattivi, le altre le scegli tu.")
                    .size(12.0)
                    .color(MUTED),
            );
        }
        Section::Worktrees => {
            ui.label(
                RichText::new("I worktree si rimuovono con git e sempre in modo definitivo; quelli con modifiche non salvate restano.")
                    .size(12.0)
                    .color(MUTED),
            );
        }
        Section::Sessions => {}
    }
    ui.add_space(8.0);
    match state.section {
        Section::Projects => {
            projects_table(ui, &scan.projects, &mut state.sel_artifacts, analysis_failed(&scan), state.screenshot)
        }
        Section::Worktrees => {
            worktrees_table(ui, &scan.worktrees, &scan.other_worktrees, &mut state.sel_worktrees, analysis_failed(&scan))
        }
        Section::Sessions => sessions_table(ui, &scan.archived, &mut state.sel_sessions),
    }
}

fn cleaned_view(state: &mut DevState, ui: &mut Ui, busy: bool) {
    let title = cleaned_title(state.cleaned.as_ref(), busy);
    let can_save = !busy && state.cleaned.as_ref().is_some_and(|c| !c.is_empty());
    let mut save = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(20.0).strong().color(TEXT));
        if can_save {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                save = secondary(ui, "Salva come testo…", true).clicked();
            });
        }
    });
    if save {
        save_report(state);
    }
    if let Some((text, ok)) = &state.saved {
        ui.add(egui::Label::new(RichText::new(text).size(12.0).color(if *ok { GREEN } else { RED })).wrap());
    }
    let Some(c) = &state.cleaned else { return };
    if c.interrupted {
        let text = "Un errore interno ha fermato la pulizia: altri elementi potrebbero essere stati toccati senza comparire qui. Premi «Analizza» per vedere cosa resta.";
        ui.add(egui::Label::new(RichText::new(text).color(RED)).wrap());
    }
    if c.is_empty() {
        if c.errors.is_empty() && !c.interrupted {
            ui.add_space(10.0);
            ui.label(RichText::new("Nessun elemento da pulire.").color(MUTED));
        }
        return;
    }
    report_view(ui, &c.report, &|p| c.label(p), "dev_cleaned", |_| {});
}

/// «Salva come testo…»: il resoconto in un file scelto dall'utente.
fn save_report(state: &mut DevState) {
    let Some(c) = &state.cleaned else { return };
    if state.screenshot {
        return;
    }
    if let Some(out) = save_report_text(&c.report) {
        state.saved = Some(out);
    }
}

/// Il resoconto di una pulizia (Sviluppatori e Pulizia file): spazio liberato, conteggi,
/// durata, tabella per categoria, saltati e non riusciti. `label` dice come mostrare un
/// percorso dei saltati e dei non riusciti; `more` aggiunge righe in fondo, nello stesso
/// riquadro scorrevole.
pub fn report_view(ui: &mut Ui, r: &CleanReport, label: &dyn Fn(&Path) -> String, id_salt: &str, more: impl FnOnce(&mut Ui)) {
    ui.label(RichText::new(format!("Spazio liberato: {}", sys::fmt_size(r.freed_bytes))).size(15.0).color(GREEN));
    if r.recycled_bytes > 0 {
        ui.label(RichText::new(recycled_text(r.recycled_bytes)).color(TEXT));
    }
    ui.label(
        RichText::new(format!(
            "File eliminati: {} · Cartelle eliminate: {} · Durata: {}",
            r.files,
            r.dirs,
            fmt_duration(r.elapsed_ms)
        ))
        .size(12.0)
        .color(MUTED),
    );
    if r.cancelled {
        ui.label(RichText::new("Annullata: le voci non ancora iniziate sono rimaste dov'erano.").color(MUTED));
    }
    ui.add_space(10.0);
    let gap = ui.spacing().item_spacing.x;
    egui::ScrollArea::vertical().id_salt(id_salt).auto_shrink([false, false]).show(ui, |ui| {
        let total = ui.available_width() - SCROLLBAR;
        if !r.by_category.is_empty() {
            let w = col_widths(total, &[0.0, 120.0], gap);
            header(ui, &w, &["Categoria", "Spazio"]);
            for (i, (label, bytes)) in r.by_category.iter().enumerate() {
                striped_row(ui, i, |ui| {
                    crate::cell(ui, w[0], RichText::new(label).color(TEXT));
                    crate::cell(ui, w[1], RichText::new(sys::fmt_size(*bytes)).color(TEXT));
                });
            }
        }
        for (title, list, color) in [("Saltati", &r.skipped, MUTED), ("Non riusciti", &r.failed, RED)] {
            if list.is_empty() {
                continue;
            }
            ui.add_space(12.0);
            ui.label(RichText::new(format!("{title} ({})", list.len())).strong().color(TEXT));
            let w = col_widths(total, &[0.0, 300.0], gap);
            header(ui, &w, &["Elemento", "Motivo"]);
            for (i, (path, why)) in list.iter().enumerate() {
                striped_row(ui, i, |ui| {
                    crate::cell(ui, w[0], RichText::new(label(path)).color(TEXT)).on_hover_text(path.display().to_string());
                    crate::cell(ui, w[1], RichText::new(why).color(color)).on_hover_text(why);
                });
            }
        }
        more(ui);
    });
}

/// «Salva come testo…»: il resoconto in un file scelto dall'utente. `None` se l'utente
/// chiude il dialogo; altrimenti (messaggio, riuscito).
pub fn save_report_text(r: &CleanReport) -> Option<(String, bool)> {
    let path = rfd::FileDialog::new()
        .set_title("Salva il resoconto")
        .set_file_name("Resoconto Xiox.txt")
        .add_filter("Testo", &["txt"])
        .save_file()?;
    Some(match std::fs::write(&path, r.to_text()) {
        Ok(()) => (format!("Resoconto salvato in {}", path.display()), true),
        Err(e) => (format!("Resoconto non salvato: {e}"), false),
    })
}

/// La voce «Sviluppatori»: pannello opzioni, pulsanti in basso, card centrale, conferma.
pub fn dev_tab(state: &mut DevState, ui: &mut Ui, admin: bool) {
    if state.auto_scan && !state.busy() {
        state.auto_scan = false;
        state.start_scan(repaint(ui.ctx()), admin);
    }
    #[cfg(debug_assertions)]
    {
        state.run_demo(repaint(ui.ctx()));
    }
    let busy = state.busy();
    egui::SidePanel::left("dev_opts")
        .exact_width(270.0)
        .resizable(false)
        .frame(egui::Frame::new().fill(CARD).inner_margin(Margin::same(14)).stroke(Stroke::new(1.0_f32, LINE)))
        .show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| options(state, ui, busy));
        });
    egui::TopBottomPanel::bottom("dev_buttons")
        .frame(egui::Frame::new().inner_margin(Margin::symmetric(16, 12)))
        .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                let (selected, _, bytes) = state.selection();
                if state.scan.is_some() {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(format!("Selezionati in {}: {}", state.section.name(), sys::fmt_size(bytes))).color(TEXT));
                        ui.label(RichText::new(selection_text(state.section, selected)).size(12.0).color(MUTED));
                    });
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if primary(ui, &clean_button(state.section, selected), state.can_clean()).clicked() {
                        state.open_confirm();
                    }
                    if secondary(ui, "Analizza", !busy).clicked() {
                        state.start_scan(repaint(ui.ctx()), admin);
                    }
                });
            });
        });
    egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin::same(16))).show_inside(ui, |ui| {
        match (&state.view, &state.scan, &state.cleaned) {
            (View::Scanned, Some(scan), _) => {
                if let Some(n) = &state.notice {
                    notice_strip(ui, n);
                }
                warn_strip(ui, &scan.warnings)
            }
            // Un piano o una scelta di sessioni rifiutati: niente è stato toccato e non si
            // riprova da soli, serve una nuova analisi.
            (View::Cleaned, _, Some(c)) if !c.errors.is_empty() => error_strip(ui, &c.errors),
            _ => {}
        }
        card(ui, |ui| body(state, ui, busy));
    });
    confirm_dialog(state, ui.ctx());
}

fn confirm_dialog(state: &mut DevState, ctx: &egui::Context) {
    let Some(c) = &mut state.confirm else { return };
    let mut go = false;
    let mut permanent = false;
    let mut close = false;
    egui::Modal::new(egui::Id::new("dev_confirm")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.label(RichText::new(confirm_title(c.section)).size(18.0).strong());
        ui.add_space(6.0);
        ui.add(egui::Label::new(confirm_text(c.section, shown_mode(c))).wrap());
        if let Some(o) = &c.others {
            ui.add_space(4.0);
            ui.add(egui::Label::new(o.as_str()).wrap());
        }
        ui.add_space(8.0);
        if let Some(e) = &c.error {
            ui.add(egui::Label::new(RichText::new(e).color(RED)).wrap());
        }
        for s in &c.summary {
            ui.label(RichText::new(s).color(ACCENT));
        }
        if !c.rows.is_empty() {
            ui.add_space(4.0);
            let w = col_widths(ui.available_width() - SCROLLBAR, &[0.0, 90.0], ui.spacing().item_spacing.x);
            egui::ScrollArea::vertical()
                .id_salt("dev_confirm_list")
                .max_height(180.0)
                .auto_shrink([false, true])
                .show_rows(ui, ROW_H, c.rows.len(), |ui, range| {
                    for i in range {
                        let (label, size) = &c.rows[i];
                        striped_row(ui, i, |ui| {
                            crate::cell(ui, w[0], label.as_str());
                            crate::cell(ui, w[1], RichText::new(size).color(MUTED));
                        });
                    }
                });
        }
        if !c.cautions.is_empty() {
            ui.add_space(10.0);
            ui.label(RichText::new(caution_title(c.cautions.len())).strong().color(RED));
            egui::ScrollArea::vertical().id_salt("dev_confirm_cautions").max_height(150.0).auto_shrink([false, true]).show(
                ui,
                |ui| {
                    for (label, note) in &c.cautions {
                        ui.label(RichText::new(label).color(TEXT));
                        ui.add(egui::Label::new(RichText::new(note).size(12.0).color(MUTED)).wrap());
                        ui.add_space(2.0);
                    }
                },
            );
            ui.checkbox(&mut c.ack, "Ho letto gli avvisi");
        }
        if !c.shortfall.is_empty() {
            ui.add_space(10.0);
            ui.add(egui::Label::new(RichText::new(shortfall_text(&c.shortfall)).strong().color(TEXT)).wrap());
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let ready = confirm_ready(c);
            if c.shortfall.is_empty() {
                go = primary(ui, &confirm_button(c), ready).clicked();
            } else {
                permanent = primary(ui, "Definitiva", ready).clicked();
            }
            close = secondary(ui, "Annulla", true).clicked();
        });
    });
    if permanent {
        if let Some(c) = state.confirm.as_mut() {
            if let Err(e) = make_permanent(c) {
                c.error = Some(e);
                return;
            }
        }
        go = true;
    }
    if go {
        if let Some(c) = state.confirm.take().and_then(|c| state.accept_confirm(c, xiox_app::now_ms())) {
            state.start_clean(repaint(ctx), c);
        }
    } else if close {
        state.confirm = None;
    }
}

// ---------- tables ----------

/// Colonne dei progetti: freccia, casella, nome e stato (cartella e avviso), ultimo uso
/// (livello), dimensione.
const PROJECT_COLS: [f32; 5] = [CHECK_W, CHECK_W, 0.0, 120.0, 80.0];

/// Ogni progetto è una riga con la casella a tre stati e si apre sulle sue cartelle, ognuna con
/// la sua casella, il livello, la dimensione e l'avviso. Nulla è nascosto: le cartelle dei
/// progetti in uso o bloccati restano visibili, con la casella spenta.
fn projects_table(ui: &mut Ui, reports: &[ProjectReport], sel: &mut HashSet<String>, failed: bool, open_all: bool) {
    if reports.is_empty() {
        empty_note(ui, empty_text(Section::Projects, failed));
        return;
    }
    let w = col_widths(ui.available_width() - SCROLLBAR, &PROJECT_COLS, ui.spacing().item_spacing.x);
    header(ui, &w, &["", "", "Progetto · cartella", "Ultimo uso · livello", "Dimensione"]);
    egui::ScrollArea::vertical().id_salt("dev_projects").auto_shrink([false, false]).show(ui, |ui| {
        let mut row = 0;
        for r in reports {
            let free = status_allows(r);
            let locked = (!free).then(|| locked_why(r));
            let locked = locked.as_deref();
            let color = if free { TEXT } else { MUTED };
            let mut open = CollapsingState::load_with_default_open(ui.ctx(), ui.make_persistent_id(("dev_project", &r.id)), open_all);
            if open_all {
                open.set_open(true);
            }
            striped_row(ui, row, |ui| {
                ui.allocate_ui_with_layout(Vec2::new(w[0], ROW_H), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_min_width(w[0]);
                    ui.set_max_width(w[0]);
                    if !r.artifacts.is_empty() {
                        open.show_toggle_button(ui, paint_default_icon);
                    }
                });
                let tri = tri_state(r, sel);
                let mut all = tri == Tri::All;
                let check = ui
                    .allocate_ui_with_layout(Vec2::new(w[1], ROW_H), Layout::left_to_right(Align::Center), |ui| {
                        ui.set_min_width(w[1]);
                        ui.set_max_width(w[1]);
                        ui.add_enabled(
                            free && !r.artifacts.is_empty(),
                            egui::Checkbox::without_text(&mut all).indeterminate(tri == Tri::Some),
                        )
                    })
                    .inner;
                if check.clicked() {
                    toggle_project(r, sel);
                }
                if let Some(why) = locked {
                    check.on_disabled_hover_text(why);
                }
                ui.allocate_ui_with_layout(Vec2::new(w[2], ROW_H), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_min_width(w[2]);
                    ui.set_max_width(w[2]);
                    ui.add(egui::Label::new(RichText::new(&r.name).strong().color(color)))
                        .on_hover_text(r.root.display().to_string());
                    // Il motivo intero, a capo se serve: la riga si alza.
                    ui.add(egui::Label::new(RichText::new(&r.reason).size(12.0).color(MUTED)).wrap());
                });
                crate::cell(ui, w[3], RichText::new(r.last_activity_ms.map_or_else(|| "—".to_string(), fmt_date)).color(color));
                crate::cell(ui, w[4], RichText::new(sys::fmt_size(r.reclaimable_bytes)).color(color));
            });
            row += 1;
            open.show_body_unindented(ui, |ui| {
                for a in &r.artifacts {
                    striped_row(ui, row, |ui| artifact_cells(ui, &w, a, sel, locked));
                    row += 1;
                }
            });
        }
    });
}

/// Una cartella di un progetto: casella, etichetta con l'avviso sotto, livello, dimensione.
/// `locked`: perché il progetto non si pulisce (casella spenta, spiegata al passaggio).
fn artifact_cells(ui: &mut Ui, w: &[f32], a: &Artifact, sel: &mut HashSet<String>, locked: Option<&str>) {
    let free = locked.is_none();
    crate::cell(ui, w[0], "");
    let mut on = sel.contains(&a.id);
    let check = check_cell(ui, w[1], &mut on, free);
    if check.changed() {
        set_artifact(sel, a, on);
    }
    if let Some(why) = locked {
        check.on_disabled_hover_text(why);
    }
    let color = if free { TEXT } else { MUTED };
    ui.allocate_ui_with_layout(Vec2::new(w[2], ROW_H), Layout::top_down(Align::Min), |ui| {
        ui.set_min_width(w[2]);
        ui.set_max_width(w[2]);
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.add(egui::Label::new(RichText::new(&a.label).color(color)).truncate()).on_hover_text(a.path.display().to_string());
        if !a.note.is_empty() {
            // L'avviso intero («Si riscarica con…», «Potrebbe contenere sorgenti…»), a capo se serve.
            ui.add(egui::Label::new(RichText::new(&a.note).size(12.0).color(MUTED)).wrap());
        }
    });
    crate::cell(ui, w[3], RichText::new(level_word(a.level)).color(if free { level_color(a.level) } else { MUTED }));
    crate::cell(ui, w[4], RichText::new(sys::fmt_size(a.size.effective_bytes())).color(color));
}

fn worktrees_table(ui: &mut Ui, reports: &[ProjectReport], others: &[ProjectReport], sel: &mut [bool], failed: bool) {
    let w = col_widths(ui.available_width() - SCROLLBAR, &[CHECK_W, 0.0, 110.0, 150.0, 86.0], ui.spacing().item_spacing.x);
    let titles = ["", "Worktree", "Repository", "Stato", "Dimensione"];
    if reports.is_empty() {
        empty_note(ui, empty_text(Section::Worktrees, failed));
    } else {
        header(ui, &w, &titles);
    }
    egui::ScrollArea::vertical().id_salt("dev_worktrees").auto_shrink([false, false]).show(ui, |ui| {
        for (i, r) in reports.iter().enumerate() {
            let on = selectable(r);
            let why = nothing_to_remove(r);
            striped_row(ui, i, |ui| {
                let check = check_cell(ui, w[0], &mut sel[i], on);
                if let Some(why) = why {
                    check.on_disabled_hover_text(why);
                }
                worktree_cells(ui, &w, r, if on { TEXT } else { MUTED }, why);
            });
        }
        if !others.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new("Altri worktree (creati a mano, non si rimuovono da qui)").color(MUTED));
            ui.add_space(2.0);
            if reports.is_empty() {
                header(ui, &w, &titles);
            }
            for (i, r) in others.iter().enumerate() {
                striped_row(ui, i, |ui| {
                    crate::cell(ui, w[0], "");
                    worktree_cells(ui, &w, r, MUTED, None);
                });
            }
        }
    });
}

/// `why`: perché la riga non ha nulla da togliere, al posto dello stato e in grigio come le
/// righe bloccate.
fn worktree_cells(ui: &mut Ui, w: &[f32], r: &ProjectReport, color: Color32, why: Option<&str>) {
    crate::cell(ui, w[1], RichText::new(&r.name).color(color)).on_hover_text(r.root.display().to_string());
    let repo = r.git.main_worktree.as_deref();
    let resp = crate::cell(ui, w[2], RichText::new(repo.map_or_else(|| "—".to_string(), folder_name)).color(color));
    if let Some(p) = repo {
        resp.on_hover_text(p.display().to_string());
    }
    match why {
        Some(why) => crate::cell(ui, w[3], RichText::new(why).size(12.0).color(MUTED)),
        None => crate::cell(ui, w[3], RichText::new(&r.reason).color(color)),
    };
    crate::cell(ui, w[4], RichText::new(sys::fmt_size(r.reclaimable_bytes)).color(color));
}

fn sessions_table(ui: &mut Ui, sessions: &[ArchivedSession], sel: &mut [bool]) {
    if sessions.is_empty() {
        empty_note(ui, empty_text(Section::Sessions, false));
        return;
    }
    let w = col_widths(ui.available_width() - SCROLLBAR, &[CHECK_W, 76.0, 0.0, 80.0, 74.0, 116.0], ui.spacing().item_spacing.x);
    header(ui, &w, &["", "Harness", "Titolo", "Progetto", "Data", "Dimensione"]);
    egui::ScrollArea::vertical().id_salt("dev_sessions").auto_shrink([false, false]).show_rows(ui, ROW_H, sessions.len(), |ui, range| {
        for i in range {
            let s = &sessions[i];
            let on = s.blocked.is_none();
            let color = if on { TEXT } else { MUTED };
            striped_row(ui, i, |ui| {
                let check = check_cell(ui, w[0], &mut sel[i], on);
                if let Some(why) = &s.blocked {
                    check.on_disabled_hover_text(why);
                }
                crate::cell(ui, w[1], RichText::new(agent_label(s.agent)).color(color));
                crate::cell(ui, w[2], RichText::new(session_title(s)).color(color));
                let project = crate::cell(ui, w[3], RichText::new(s.cwd.as_deref().map_or_else(|| "—".to_string(), folder_name)).color(color));
                if let Some(cwd) = &s.cwd {
                    project.on_hover_text(cwd.display().to_string());
                }
                crate::cell(ui, w[4], RichText::new(s.updated_ms.map_or_else(|| "—".to_string(), fmt_date)).color(color));
                match &s.blocked {
                    Some(why) => crate::cell(ui, w[5], RichText::new(why).size(12.0).color(MUTED)),
                    None if s.file.is_none() => crate::cell(ui, w[5], RichText::new(session_size(s)).size(12.0).color(MUTED)),
                    None => crate::cell(ui, w[5], RichText::new(session_size(s)).color(TEXT)),
                };
            });
        }
    });
}

fn group(ui: &mut Ui, title: &str, space: f32) {
    ui.add_space(space);
    ui.label(RichText::new(title).strong().color(ACCENT));
    ui.separator();
}

fn header(ui: &mut Ui, w: &[f32], titles: &[&str]) {
    ui.horizontal(|ui| {
        for (w, t) in w.iter().zip(titles) {
            crate::cell(ui, *w, RichText::new(*t).strong());
        }
    });
    ui.separator();
}

fn empty_note(ui: &mut Ui, text: &str) {
    ui.add_space(20.0);
    ui.vertical_centered(|ui| ui.label(RichText::new(text).size(16.0).color(MUTED)));
    ui.add_space(10.0);
}

/// Una riga di tabella alta [`ROW_H`]; le righe dispari hanno lo sfondo [`STRIPE`].
fn striped_row(ui: &mut Ui, i: usize, add: impl FnOnce(&mut Ui)) {
    let bg = ui.painter().add(egui::Shape::Noop);
    let r = ui
        .horizontal(|ui| {
            ui.set_min_height(ROW_H);
            add(ui)
        })
        .response
        .rect;
    if i % 2 == 1 {
        let half = ui.spacing().item_spacing.y / 2.0;
        let rect = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), (r.top() - half)..=(r.bottom() + half));
        ui.painter().set(bg, egui::Shape::rect_filled(rect, 3.0, STRIPE));
    }
}

fn check_cell(ui: &mut Ui, w: f32, value: &mut bool, enabled: bool) -> egui::Response {
    ui.allocate_ui_with_layout(Vec2::new(w, ROW_H), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_width(w);
        ui.set_max_width(w);
        ui.add_enabled(enabled, egui::Checkbox::without_text(value))
    })
    .inner
}

/// Gli avvisi dell'analisi, il fallimento dell'analisi dei progetti per primo e in evidenza.
fn warn_strip(ui: &mut Ui, warnings: &[String]) {
    if warnings.is_empty() {
        return;
    }
    let list = ordered_warnings(warnings);
    egui::Frame::new().fill(WARN_BG).corner_radius(6).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        for w in list.iter().take(2) {
            if w.as_str() == ANALYSIS_FAILED {
                let text = format!("⚠  {w} I progetti e i worktree non sono stati analizzati: premi «Analizza» per riprovare.");
                ui.add(egui::Label::new(RichText::new(text).strong().color(TEXT)).wrap());
            } else {
                ui.add(egui::Label::new(RichText::new(format!("⚠  {w}")).color(TEXT)).truncate());
            }
        }
        if list.len() > 2 {
            let rest: Vec<&str> = list[2..].iter().map(|w| w.as_str()).collect();
            ui.label(RichText::new(format!("e altri {} avvisi", list.len() - 2)).size(12.0).color(MUTED))
                .on_hover_text(rest.join("\n"));
        }
    });
    ui.add_space(8.0);
}

/// Un avviso sul pulsante appena premuto (per esempio l'analisi troppo vecchia), niente dialogo.
fn notice_strip(ui: &mut Ui, text: &str) {
    egui::Frame::new().fill(WARN_BG).corner_radius(6).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(RichText::new(format!("⚠  {text}")).strong().color(TEXT)).wrap());
    });
    ui.add_space(8.0);
}

/// Richieste rifiutate da `xiox_app` (per esempio «Piano rifiutato: …»): per intero, mai troncate.
fn error_strip(ui: &mut Ui, errors: &[String]) {
    egui::Frame::new().fill(WARN_BG).corner_radius(6).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        for e in errors {
            ui.add(egui::Label::new(RichText::new(format!("⚠  {e}")).color(TEXT)).wrap());
        }
        ui.label(RichText::new("Quella parte non è stata toccata. Premi «Analizza» per rifare l'analisi prima di riprovare.").strong().color(TEXT));
    });
    ui.add_space(8.0);
}

/// Larghezze delle colonne: la colonna `0.0` prende quello che resta (almeno 80 px).
fn col_widths(total: f32, cols: &[f32], gap: f32) -> Vec<f32> {
    let fixed: f32 = cols.iter().sum();
    let gaps = gap * cols.len().saturating_sub(1) as f32;
    let flex = (total - fixed - gaps).max(80.0);
    cols.iter().map(|&w| if w == 0.0 { flex } else { w }).collect()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_widths_give_the_rest_to_the_flexible_column() {
        assert_eq!(col_widths(500.0, &[20.0, 0.0, 100.0], 8.0), vec![20.0, 364.0, 100.0]);
        assert_eq!(col_widths(100.0, &[20.0, 0.0, 100.0], 8.0), vec![20.0, 80.0, 100.0]);
    }
}
