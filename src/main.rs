#![windows_subsystem = "windows"]

mod dev;
mod theme;

use xiox_core::{browserdb, cleaner, native, registry, rules, services, sys, wipe};
use xiox_core::cleanup::{analysis_title, clean_report, Chosen, CleanEnd, CleanView, CleanupState};
use xiox_core::notify::{NoNotifier, Notifier, SectionId};
use xiox_core::regstate::RegistryState;
use xiox_core::svcstate::ServicesState;

use eframe::egui::{self, Align, Color32, FontId, Layout, Margin, RichText, Stroke, TextStyle, Ui, Vec2};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use theme::*;

/// The egui window as a `Notifier`: a worker's wake-up is a repaint request.
struct Repaint(egui::Context);

impl Notifier for Repaint {
    fn notify(&self, _: SectionId) {
        self.0.request_repaint()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Cleaner,
    Registry,
    Services,
    Dev,
}

struct App {
    tab: Tab,
    admin: bool,
    /// Account del processo e della sessione, se diversi (calcolato una volta all'avvio).
    mismatch: Option<(String, String)>,
    frame: u64,
    started: Instant,
    screenshot: Option<PathBuf>,
    shot_sent: bool,
    mem: sys::Memory,
    mem_at: Instant,

    cleanup: CleanupState,
    /// Finestra Informazioni aperta.
    about_open: bool,

    registry: RegistryState,
    services: ServicesState,

    dev: dev::DevState,
}

impl App {
    fn new(screenshot: Option<PathBuf>, tab: Tab, dev: dev::DevState, about_open: bool) -> Self {
        let mut app = App {
            tab,
            admin: sys::is_admin(),
            mismatch: sys::profile_mismatch(),
            frame: 0,
            started: Instant::now(),
            screenshot,
            shot_sent: false,
            mem: sys::memory(),
            mem_at: Instant::now(),
            cleanup: CleanupState::new(),
            about_open,
            registry: RegistryState::new(),
            services: ServicesState::new(),
            dev,
        };
        if tab == Tab::Services {
            // Senza finestra a cui chiedere un ridisegno: come prima, il ciclo di `update` rilegge.
            app.services.load(Arc::new(NoNotifier));
        }
        app
    }

    // ---------- background jobs ----------

    /// Avvia il caricamento delle regole e, tranne con --screenshot, l'aggiornamento di Winapp2.
    fn start_rules(&mut self, ctx: &egui::Context) {
        let fake_profile = self.screenshot.is_some();
        self.cleanup.start_rules(
            Arc::new(Repaint(ctx.clone())),
            rules::rules_env_from_process(fake_profile),
            rules::should_update(fake_profile),
        );
    }

    /// Solo build di debug, `--clean-demo` con `--screenshot`: un resoconto inventato per
    /// vedere la vista. Non legge né tocca alcun file.
    #[cfg(debug_assertions)]
    fn clean_demo(&mut self) {
        let user = PathBuf::from(r"C:\Users\demo\AppData\Local\Temp");
        self.cleanup.clean_sel = vec![false; self.cleanup.clean_sel.len()];
        for (i, freed, removed) in [(1usize, 734_003_200u64, 1_204u64), (3, 52_428_800, 2), (6, 18_874_368, 9)] {
            self.cleanup.clean_sel[i] = true;
            self.cleanup.found[i] = Some(cleaner::Found { bytes: freed, count: removed, ..Default::default() });
            let mut c = cleaner::Cleaned { freed, removed, dirs: if i == 1 { 37 } else { 0 }, ..Default::default() };
            if i == 1 {
                c.failed = 2;
                c.failures = vec![
                    (user.join("setup.log"), "Il file è in uso (in uso da: setup.exe)".into()),
                    (user.join("~DF1A2B.tmp"), "Accesso negato. (os error 5)".into()),
                ];
            }
            self.cleanup.cleaned[i] = Some(c);
        }
        self.cleanup.clean_sel[8] = true;
        self.cleanup.cleaned[8] = Some(cleaner::Cleaned { cancelled: true, ..Default::default() });
        let cats: Vec<usize> = (0..self.cleanup.clean_sel.len()).filter(|&i| self.cleanup.clean_sel[i]).collect();
        let native = native::NativeCleaned { freed: 0, removed: 1, done: vec!["Appunti svuotati".into()], ..Default::default() };
        self.cleanup.native_cleaned.push(("native:clipboard".into(), "Appunti".into(), native));
        let rule = rules::RuleCleaned {
            freed: 157_286_400,
            removed: 3_310,
            files: 3_310,
            failed: 1,
            notes: vec!["1 file in uso".into()],
            ..Default::default()
        };
        self.cleanup.rule_cleaned.push(("demo/cache".into(), "Google Chrome – Cache".into(), rule));
        let mut skipped = rules::RuleCleaned::default();
        skipped.notes.push("Saltata: Firefox in esecuzione".into());
        self.cleanup.rule_cleaned.push(("demo/ff".into(), "Firefox – Cache".into(), skipped));
        self.cleanup.clean_chosen = Chosen {
            cats,
            others: vec![
                ("native:clipboard".into(), "Appunti".into()),
                ("demo/cache".into(), "Google Chrome – Cache".into()),
                ("demo/ff".into(), "Firefox – Cache".into()),
                ("demo/edge".into(), "Microsoft Edge – Cache".into()),
            ],
        };
        self.cleanup.clean_view = CleanView::Cleaned;
        self.cleanup.clean_end = Some(CleanEnd::Cancelled);
        self.cleanup.clean_report = Some(clean_report(
            &self.cleanup.cleaned,
            &self.cleanup.native_cleaned,
            &self.cleanup.rule_cleaned,
            &self.cleanup.clean_chosen,
            8_400,
            CleanEnd::Cancelled,
        ));
    }

    fn poll(&mut self, ctx: &egui::Context) {
        self.cleanup.poll();
        self.registry.poll();
        self.services.poll(Arc::new(Repaint(ctx.clone())));
        self.dev.poll();
    }

    // ---------- drawing ----------

    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("nav")
            .exact_width(212.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(SIDEBAR).inner_margin(Margin::symmetric(0, 14)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    let (r, p) = ui.allocate_painter(Vec2::splat(34.0), egui::Sense::hover());
                    let c = r.rect.center();
                    p.circle_filled(c, 16.0, ACCENT);
                    p.circle_filled(c, 9.0, SIDEBAR);
                    p.circle_filled(c + Vec2::new(6.0, -6.0), 4.0, GREEN);
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Xiox").color(Color32::WHITE).size(18.0).strong());
                        ui.label(RichText::new("Pulizia leggera").color(Color32::from_gray(150)).size(12.0));
                    });
                });
                ui.add_space(22.0);
                for (tab, icon, label) in [
                    (Tab::Cleaner, "🗑", "Pulizia file"),
                    (Tab::Registry, "🔧", "Registro"),
                    (Tab::Services, "⚙", "Servizi e RAM"),
                    (Tab::Dev, "💻", "Sviluppatori"),
                ] {
                    let sel = self.tab == tab;
                    let fill = if sel { SIDEBAR_SEL } else { SIDEBAR };
                    let text = RichText::new(format!("  {icon}   {label}")).size(15.0).color(if sel { Color32::WHITE } else { Color32::from_gray(190) });
                    let resp = ui.add(egui::Button::new(text).fill(fill).stroke(Stroke::NONE).corner_radius(0).min_size(Vec2::new(212.0, 46.0)));
                    if sel {
                        ui.painter().rect_filled(egui::Rect::from_min_size(resp.rect.min, Vec2::new(4.0, resp.rect.height())), 0.0, ACCENT);
                    }
                    if resp.clicked() {
                        if self.tab == Tab::Dev && tab != Tab::Dev {
                            // Uscendo dalla voce si liberano i dati dell'analisi (se nessun lavoro è in corso).
                            self.dev.release();
                        }
                        self.tab = tab;
                        if tab == Tab::Services && !self.services.loaded {
                            self.services.load(Arc::new(Repaint(ctx.clone())));
                        }
                    }
                }
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        let ram = format!("RAM: {} GB", (self.mem.total as f64 / 1073741824.0).round());
                        ui.label(RichText::new(ram).color(Color32::from_gray(140)).size(12.0));
                    });
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        let (t, c) = if self.admin { ("Amministratore", GREEN) } else { ("Utente standard", Color32::from_rgb(0xe0, 0xa0, 0x30)) };
                        let (r, p) = ui.allocate_painter(Vec2::new(10.0, 14.0), egui::Sense::hover());
                        p.circle_filled(r.rect.center(), 4.0, c);
                        ui.label(RichText::new(t).color(c).size(12.0));
                    });
                    ui.add_space(8.0);
                    // Stessa forma delle voci in alto; layout dall'alto per avere il testo a sinistra come loro.
                    ui.allocate_ui_with_layout(Vec2::new(212.0, 46.0), Layout::top_down(Align::Min), |ui| {
                        let open = self.about_open;
                        let text = RichText::new("  ℹ   Informazioni").size(15.0).color(if open { Color32::WHITE } else { Color32::from_gray(190) });
                        let fill = if open { SIDEBAR_SEL } else { SIDEBAR };
                        if ui.add(egui::Button::new(text).fill(fill).stroke(Stroke::NONE).corner_radius(0).min_size(Vec2::new(212.0, 46.0))).clicked() {
                            self.about_open = true;
                        }
                    });
                });
            });
    }

    /// Finestra Informazioni: nome, versione, Winapp2 caricato e attribuzione.
    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let winapp2 = match self.cleanup.rules.as_ref() {
            None => "Winapp2: caricamento in corso".to_string(),
            Some(rs) => match &rs.winapp2_version {
                Some(v) => format!("Winapp2: versione {v}"),
                None => "Winapp2: non ancora scaricato".to_string(),
            },
        };
        let mut close = false;
        let resp = egui::Modal::new(egui::Id::new("about")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new("Xiox").size(18.0).strong());
            ui.add_space(6.0);
            ui.label(format!("Versione {}", env!("CARGO_PKG_VERSION")));
            ui.label(winapp2);
            ui.add_space(8.0);
            ui.add(egui::Label::new(RichText::new(rules::ABOUT_RULES).color(MUTED)).wrap());
            ui.add_space(12.0);
            if secondary(ui, "Chiudi", true).clicked() {
                close = true;
            }
        });
        if close || resp.should_close() {
            self.about_open = false;
        }
    }

    /// Avviso quando l'app gira con un account diverso da quello della sessione (profilo sbagliato).
    fn profile_strip(&self, ui: &mut Ui) {
        let Some((p, s)) = &self.mismatch else { return };
        egui::Frame::new().fill(WARN_BG).corner_radius(6).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
            let text = format!("⚠  Stai pulendo il profilo di «{p}», non quello di «{s}» che ha aperto la sessione.");
            ui.add(egui::Label::new(RichText::new(text).color(TEXT)).wrap());
        });
        ui.add_space(8.0);
    }

    fn cleaner_tab(&mut self, ui: &mut Ui) {
        let busy = self.cleanup.clean_job.is_some();
        egui::SidePanel::left("clean_opts")
            .exact_width(270.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(CARD).inner_margin(Margin::same(14)).stroke(Stroke::new(1.0_f32, LINE)))
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    let mut group = "";
                    for (i, c) in cleaner::CATEGORIES.iter().enumerate() {
                        if c.group != group {
                            group = c.group;
                            ui.add_space(6.0);
                            ui.label(RichText::new(group).strong().color(ACCENT));
                            ui.separator();
                        }
                        ui.add_enabled_ui(!busy, |ui| {
                            let label = if c.needs_admin && !self.admin { format!("{}  🛡", c.name) } else { c.name.to_string() };
                            ui.checkbox(&mut self.cleanup.clean_sel[i], label).on_hover_text(c.help);
                        });
                    }
                    let natives = Natives { items: &self.cleanup.natives, admin: self.admin };
                    match &self.cleanup.rules {
                        Some(rs) => {
                            let status = rules::status_line(rs, self.cleanup.winapp2_update.as_ref());
                            rules_panel(ui, rs, natives, status, &mut self.cleanup.rule_sel, &mut self.cleanup.rule_search, &self.cleanup.processes, busy);
                        }
                        // Regole non ancora caricate: le voci native ci sono già.
                        None => group_panel(ui, None, natives, &rules::panel_groups(&[], natives.items), &mut self.cleanup.rule_sel, &self.cleanup.processes, busy),
                    }
                    ui.add_space(12.0);
                    ui.label(RichText::new("Opzioni").strong().color(ACCENT));
                    ui.separator();
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.checkbox(&mut self.cleanup.only_old, format!("Elimina dalle cartelle Temp solo i file più vecchi di {} ore", cleaner::TEMP_MIN_AGE_HOURS));
                    });
                });
            });
        egui::TopBottomPanel::bottom("clean_buttons")
            .frame(egui::Frame::new().inner_margin(Margin::symmetric(16, 12)))
            .show_inside(ui, |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let any = self.cleanup.any_selected();
                    if primary(ui, "Avvia pulizia", !busy && any).clicked() {
                        self.cleanup.confirm_warnings = self.cleanup.rule_warnings();
                        self.cleanup.confirm_clean = true;
                    }
                    if secondary(ui, "Analizza", !busy && any).clicked() {
                        let ctx = ui.ctx().clone();
                        self.cleanup.start(false, Arc::new(Repaint(ctx)));
                    }
                });
            });
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin::same(16))).show_inside(ui, |ui| {
            self.profile_strip(ui);
            card(ui, |ui| {
                if busy {
                    ui.label(RichText::new(&self.cleanup.clean_text).size(16.0).color(TEXT));
                    ui.add_space(6.0);
                    ui.add(egui::ProgressBar::new(self.cleanup.clean_progress).fill(ACCENT).show_percentage());
                    ui.add_space(6.0);
                    let cancelling = self.cleanup.cancelling();
                    if secondary(ui, if cancelling { "Annullamento…" } else { "Annulla" }, !cancelling).clicked() {
                        self.cleanup.cancel();
                    }
                    ui.add_space(10.0);
                }
                match self.cleanup.clean_view {
                    CleanView::Empty => {
                        ui.add_space(30.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("Pulizia dei file temporanei").size(22.0).strong().color(TEXT));
                            ui.add_space(6.0);
                            ui.label(RichText::new("Scegli a sinistra cosa pulire, poi premi «Analizza» per vedere quanto spazio puoi liberare.").color(MUTED));
                        });
                    }
                    CleanView::Analyzed => self.analysis_table(ui),
                    CleanView::Cleaned => self.cleaned_view(ui),
                }
            });
        });
        if self.cleanup.confirm_clean {
            let mut close = false;
            let warnings = self.cleanup.confirm_warnings.clone();
            egui::Modal::new(egui::Id::new("confirm_clean")).show(ui.ctx(), |ui| {
                ui.set_width(380.0);
                ui.label(RichText::new("Avviare la pulizia?").size(18.0).strong());
                ui.add_space(6.0);
                ui.label("I file selezionati verranno eliminati definitivamente. I file in uso vengono lasciati dove sono.");
                if !warnings.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new("Avvisi delle opzioni scelte").strong());
                    let list = |ui: &mut Ui| {
                        for w in &warnings {
                            ui.label(RichText::new(w).color(RED));
                        }
                    };
                    if warnings.len() > 4 {
                        egui::ScrollArea::vertical().max_height(160.0).show(ui, list);
                    } else {
                        list(ui);
                    }
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if primary(ui, "Continua", true).clicked() {
                        let ctx = ui.ctx().clone();
                        self.cleanup.start(true, Arc::new(Repaint(ctx)));
                        close = true;
                    }
                    if secondary(ui, "Annulla", true).clicked() {
                        close = true;
                    }
                });
            });
            if close {
                self.cleanup.confirm_clean = false;
            }
        }
    }

    fn analysis_table(&self, ui: &mut Ui) {
        let total: u64 = self.cleanup.found.iter().flatten().map(|f| f.bytes).sum::<u64>()
            + self.cleanup.native_found.iter().map(|r| r.2.bytes).sum::<u64>()
            + self.cleanup.rule_found.iter().map(|r| r.2.bytes).sum::<u64>();
        let files: u64 = self.cleanup.found.iter().flatten().map(|f| f.count).sum::<u64>()
            + self.cleanup.native_found.iter().map(|r| r.2.files).sum::<u64>()
            + self.cleanup.rule_found.iter().map(|r| r.2.count).sum::<u64>();
        let (title, note) = analysis_title(self.cleanup.clean_job.is_some(), self.cleanup.clean_end);
        ui.label(RichText::new(title).size(20.0).strong().color(TEXT));
        ui.label(RichText::new(format!("{} da eliminare ({} file)", sys::fmt_size(total), files)).size(15.0).color(ACCENT));
        if let Some(note) = note {
            ui.label(RichText::new(note).color(RED));
        }
        ui.add_space(10.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("an").striped(true).min_col_width(80.0).spacing([24.0, 8.0]).show(ui, |ui| {
                ui.label(RichText::new("Elemento").strong());
                ui.label(RichText::new("Dimensione").strong());
                ui.label(RichText::new("File").strong());
                ui.label("");
                ui.end_row();
                for (i, f) in self.cleanup.found.iter().enumerate() {
                    if let Some(f) = f {
                        ui.label(cleaner::CATEGORIES[i].name);
                        ui.label(sys::fmt_size(f.bytes));
                        ui.label(f.count.to_string());
                        if f.skipped_dirs > 0 && !self.admin {
                            ui.label(RichText::new("🛡 alcune cartelle richiedono l'amministratore").color(MUTED).size(12.0));
                        } else {
                            ui.label("");
                        }
                        ui.end_row();
                    }
                }
                for (_, label, f) in &self.cleanup.native_found {
                    ui.label(label);
                    ui.label(sys::fmt_size(f.bytes));
                    // Canali, DNS e Appunti non sono file: il numero sta nel dettaglio.
                    ui.label(if f.files > 0 { f.files.to_string() } else { "—".to_string() });
                    note_cell(ui, &f.detail);
                    ui.end_row();
                }
                for (_, label, f) in &self.cleanup.rule_found {
                    ui.label(label);
                    ui.label(sys::fmt_size(f.bytes));
                    ui.label(f.count.to_string());
                    if f.skipped > 0 {
                        ui.label(RichText::new(format!("{} elementi lasciati (protetti o illeggibili)", f.skipped)).color(MUTED).size(12.0))
                            .on_hover_text(f.skipped_reasons.join("\n"));
                    } else {
                        ui.label("");
                    }
                    ui.end_row();
                }
            });
            for (key, label, f) in &self.cleanup.native_found {
                native_list(ui, key, label, &f.detail);
            }
        });
    }

    fn cleaned_view(&mut self, ui: &mut Ui) {
        let busy = self.cleanup.clean_job.is_some();
        let report = if busy { None } else { self.cleanup.clean_report.as_ref() };
        let Some(r) = report else {
            // Durante la pulizia: il totale e le voci finite, come sempre.
            let freed: u64 = self.cleanup.cleaned.iter().flatten().map(|c| c.freed).sum::<u64>()
                + self.cleanup.native_cleaned.iter().map(|r| r.2.freed).sum::<u64>()
                + self.cleanup.rule_cleaned.iter().map(|r| r.2.freed).sum::<u64>();
            let title = if busy { "Pulizia in corso…" } else { "Pulizia completata" };
            ui.label(RichText::new(title).size(20.0).strong().color(TEXT));
            ui.label(RichText::new(format!("Liberati {}", sys::fmt_size(freed))).size(15.0).color(GREEN));
            ui.add_space(10.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.cleaned_detail(ui));
            return;
        };
        let title = if r.cancelled { "Pulizia annullata" } else { "Pulizia completata" };
        let mut save = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).size(20.0).strong().color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                save = secondary(ui, "Salva come testo…", true).clicked();
            });
        });
        // Con --screenshot nessun dialogo di sistema.
        if save && self.screenshot.is_none() {
            if let Some(out) = dev::save_report_text(r) {
                self.cleanup.clean_saved = Some(out);
            }
        }
        if let Some((text, ok)) = &self.cleanup.clean_saved {
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(if *ok { GREEN } else { RED })).wrap());
        }
        let Some(r) = &self.cleanup.clean_report else { return };
        dev::report_view(ui, r, &|p| p.display().to_string(), "clean_report", |ui| {
            ui.add_space(12.0);
            ui.label(RichText::new("Dettaglio per voce").strong().color(TEXT));
            self.cleaned_detail(ui);
        });
    }

    /// Una riga per categoria, voce nativa e opzione dinamica: liberati, eliminati, lasciati, note.
    fn cleaned_detail(&self, ui: &mut Ui) {
        egui::Grid::new("cl").striped(true).min_col_width(80.0).spacing([24.0, 8.0]).show(ui, |ui| {
            ui.label(RichText::new("Elemento").strong());
            ui.label(RichText::new("Liberati").strong());
            ui.label(RichText::new("Eliminati").strong());
            ui.label(RichText::new("Lasciati (in uso)").strong());
            ui.label("");
            ui.end_row();
            for (i, c) in self.cleanup.cleaned.iter().enumerate() {
                if let Some(c) = c {
                    ui.label(cleaner::CATEGORIES[i].name);
                    ui.label(sys::fmt_size(c.freed));
                    ui.label(c.removed.to_string());
                    ui.label(c.failed.to_string());
                    ui.label("");
                    ui.end_row();
                }
            }
            for (_, label, c) in &self.cleanup.native_cleaned {
                ui.label(label);
                ui.label(sys::fmt_size(c.freed));
                ui.label(c.removed.to_string());
                ui.label(c.failed.to_string());
                note_cell(ui, &c.notes);
                ui.end_row();
            }
            for (_, label, c) in &self.cleanup.rule_cleaned {
                ui.label(label);
                ui.label(sys::fmt_size(c.freed));
                ui.label(c.removed.to_string());
                ui.label(c.failed.to_string());
                let mut notes = c.notes.clone();
                if let Some(b) = &c.backup {
                    notes.push(format!("Backup del registro: {}", b.display()));
                }
                match notes.first() {
                    Some(first) => {
                        ui.label(RichText::new(first).color(MUTED).size(12.0)).on_hover_text(notes.join("\n"));
                    }
                    None => {
                        ui.label("");
                    }
                }
                ui.end_row();
            }
        });
        for (key, label, c) in &self.cleanup.native_cleaned {
            let lines: Vec<String> = c.done.iter().chain(&c.notes).cloned().collect();
            native_list(ui, key, label, &lines);
        }
    }

    fn registry_tab(&mut self, ui: &mut Ui) {
        let busy = self.registry.busy();
        egui::SidePanel::left("reg_opts")
            .exact_width(270.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(CARD).inner_margin(Margin::same(14)).stroke(Stroke::new(1.0_f32, LINE)))
            .show_inside(ui, |ui| {
                ui.label(RichText::new("Registro").strong().color(ACCENT));
                ui.separator();
                ui.add_enabled_ui(!busy, |ui| {
                    for (i, c) in registry::CATEGORIES.iter().enumerate() {
                        ui.checkbox(&mut self.registry.sel[i], c.name);
                    }
                });
                ui.add_space(14.0);
                ui.label(RichText::new("Prima di riparare puoi salvare un backup: con un doppio clic sul file .reg si torna indietro.").size(12.0).color(MUTED));
            });
        egui::TopBottomPanel::bottom("reg_buttons")
            .frame(egui::Frame::new().inner_margin(Margin::symmetric(16, 12)))
            .show_inside(ui, |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let sel = self.registry.issues.iter().filter(|i| i.selected).count();
                    if primary(ui, &format!("Ripara selezionati ({sel})…"), !busy && sel > 0).clicked() {
                        self.registry.confirm = true;
                    }
                    if secondary(ui, "Cerca problemi", !busy && self.registry.sel.iter().any(|&b| b)).clicked() {
                        self.registry.start_scan(Arc::new(Repaint(ui.ctx().clone())));
                    }
                });
            });
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin::same(16))).show_inside(ui, |ui| {
            self.profile_strip(ui);
            card(ui, |ui| {
                if busy {
                    ui.label(RichText::new(&self.registry.text).size(16.0).color(TEXT));
                    ui.add_space(6.0);
                    ui.add(egui::ProgressBar::new(self.registry.progress).fill(ACCENT).show_percentage());
                    ui.add_space(10.0);
                }
                if !self.registry.msg.is_empty() {
                    ui.label(RichText::new(&self.registry.msg).color(GREEN));
                    ui.add_space(8.0);
                }
                if !self.registry.scanned {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Pulizia del registro").size(22.0).strong().color(TEXT));
                        ui.add_space(6.0);
                        ui.label(RichText::new("Premi «Cerca problemi» per trovare voci che puntano a file o programmi che non esistono più.").color(MUTED));
                    });
                    return;
                }
                if self.registry.issues.is_empty() && !busy {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new("✔  Nessun problema trovato").size(18.0).color(GREEN)));
                    return;
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} problemi trovati", self.registry.issues.len())).size(18.0).strong().color(TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Nessuno").clicked() {
                            self.registry.issues.iter_mut().for_each(|i| i.selected = false);
                        }
                        if ui.small_button("Tutti").clicked() {
                            self.registry.issues.iter_mut().for_each(|i| i.selected = true);
                        }
                    });
                });
                ui.add_space(6.0);
                let w = ui.available_width();
                let (wp, wd) = (210.0, ((w - 270.0) * 0.45).max(120.0));
                let wk = (w - 30.0 - wp - wd - 20.0).max(120.0);
                ui.horizontal(|ui| {
                    ui.add_space(26.0);
                    cell(ui, wp, RichText::new("Problema").strong());
                    cell(ui, wd, RichText::new("Dati").strong());
                    cell(ui, wk, RichText::new("Chiave di registro").strong());
                });
                ui.separator();
                let n = self.registry.issues.len();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 22.0, n, |ui, range| {
                    for i in range {
                        let it = &mut self.registry.issues[i];
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut it.selected, "");
                            cell(ui, wp, &it.problem);
                            cell(ui, wd, &it.data);
                            cell(ui, wk, it.fix.key_text());
                        });
                    }
                });
            });
        });
        if self.registry.confirm {
            let mut close = false;
            egui::Modal::new(egui::Id::new("reg_confirm")).show(ui.ctx(), |ui| {
                ui.set_width(400.0);
                ui.label(RichText::new("Salvare un backup del registro?").size(18.0).strong());
                ui.add_space(6.0);
                ui.label("Consigliato: prima delle modifiche viene salvata una copia (.reg) nella cartella Documenti\\Xiox.");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if primary(ui, "Sì, salva e ripara", true).clicked() {
                        self.registry.start_fix(true, Arc::new(Repaint(ui.ctx().clone())));
                        close = true;
                    }
                    if secondary(ui, "Ripara senza backup", true).clicked() {
                        self.registry.start_fix(false, Arc::new(Repaint(ui.ctx().clone())));
                        close = true;
                    }
                    if ui.button("Annulla").clicked() {
                        close = true;
                    }
                });
            });
            if close {
                self.registry.confirm = false;
            }
        }
    }

    fn services_tab(&mut self, ui: &mut Ui) {
        if self.mem_at.elapsed() > Duration::from_secs(2) {
            self.mem = sys::memory();
            self.mem_at = Instant::now();
        }
        ui.ctx().request_repaint_after(Duration::from_secs(2));
        let busy = self.services.busy();
        egui::TopBottomPanel::bottom("svc_buttons")
            .frame(egui::Frame::new().inner_margin(Margin::symmetric(16, 12)))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    let sel: Vec<&services::Service> = self.services.services.iter().filter(|s| s.selected && !s.protected).collect();
                    let kb: u64 = sel.iter().map(|s| s.freeable_kb()).sum();
                    ui.label(RichText::new(format!("Selezionati: {}  ·  RAM liberabile (stima): {}", sel.len(), sys::fmt_size(kb * 1024))).color(TEXT));
                    let n = sel.len();
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary(ui, "Disattiva selezionati", !busy && n > 0).clicked() {
                            self.services.confirm = Some(services::START_DISABLED);
                        }
                        if secondary(ui, "Imposta su Manuale", !busy && n > 0).clicked() {
                            self.services.confirm = Some(services::START_MANUAL);
                        }
                        let saved = services::read_backup().len();
                        if secondary(ui, &format!("Ripristina originali ({saved})"), !busy && saved > 0).clicked() {
                            self.services.confirm = Some(0);
                        }
                    });
                });
            });
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin::same(16))).show_inside(ui, |ui| {
            self.profile_strip(ui);
            // RAM card
            card(ui, |ui| {
                let used = self.mem.total.saturating_sub(self.mem.available);
                let frac = if self.mem.total > 0 { used as f32 / self.mem.total as f32 } else { 0.0 };
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Memoria RAM in uso").size(16.0).strong().color(TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(format!("{} di {} ({:.0}%)", sys::fmt_size(used), sys::fmt_size(self.mem.total), frac * 100.0)).color(MUTED));
                    });
                });
                ui.add_space(4.0);
                let color = if frac > 0.85 { Color32::from_rgb(0xd9, 0x48, 0x3b) } else if frac > 0.65 { Color32::from_rgb(0xe0, 0xa0, 0x30) } else { GREEN };
                let bar_w = ui.max_rect().width();
                ui.add(egui::ProgressBar::new(frac).fill(color).desired_height(14.0).desired_width(bar_w));
            });
            ui.add_space(10.0);
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.services.all, false, "Consigliati da spegnere");
                    ui.selectable_value(&mut self.services.all, true, "Tutti i servizi");
                    ui.add_space(12.0);
                    ui.add(egui::TextEdit::singleline(&mut self.services.filter).hint_text("Cerca…").desired_width(180.0));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add_enabled(!busy, egui::Button::new("⟳ Aggiorna")).clicked() {
                            self.services.load(Arc::new(Repaint(ui.ctx().clone())));
                        }
                    });
                });
                if !self.services.msg.is_empty() {
                    ui.label(RichText::new(&self.services.msg).color(GREEN));
                }
                ui.add_space(6.0);
                if busy && self.services.services.is_empty() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Lettura dei servizi…");
                    });
                    return;
                }
                let rows: Vec<usize> = self.services.visible();
                let w = ui.available_width();
                let (wn, ws, wa, wm) = (((w - 60.0) * 0.30).max(160.0), 110.0, 150.0, 120.0);
                let wd = (w - 40.0 - wn - ws - wa - wm - 30.0).max(120.0);
                ui.horizontal(|ui| {
                    ui.add_space(26.0);
                    cell(ui, wn, RichText::new("Servizio").strong());
                    cell(ui, ws, RichText::new("Stato").strong());
                    cell(ui, wa, RichText::new("Avvio").strong());
                    cell(ui, wm, RichText::new("Memoria").strong());
                    cell(ui, wd, RichText::new("Perché spegnerlo").strong());
                });
                ui.separator();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 22.0, rows.len(), |ui, range| {
                    for &i in &rows[range] {
                        let s = &mut self.services.services[i];
                        ui.horizontal(|ui| {
                            if s.protected {
                                ui.add_enabled(false, egui::Checkbox::new(&mut false, "")).on_disabled_hover_text("Servizio necessario a Windows: protetto");
                            } else {
                                ui.checkbox(&mut s.selected, "");
                            }
                            cell(ui, wn, RichText::new(&s.display).color(TEXT)).on_hover_text(&s.name);
                            let (st, sc) = if s.running { ("In esecuzione", GREEN) } else { ("Fermo", MUTED) };
                            cell(ui, ws, RichText::new(st).color(sc));
                            let ac = if s.start == services::START_DISABLED { Color32::from_rgb(0xd9, 0x48, 0x3b) } else { TEXT };
                            cell(ui, wa, RichText::new(s.start_text()).color(ac));
                            let mem = if !s.running {
                                "–".to_string()
                            } else if s.shared > 1 {
                                format!("{} (condivisa)", sys::fmt_size(s.mem_kb * 1024))
                            } else {
                                sys::fmt_size(s.mem_kb * 1024)
                            };
                            cell(ui, wm, mem);
                            let note = if s.protected { "🔒 Necessario a Windows" } else { s.suggestion.unwrap_or("") };
                            cell(ui, wd, RichText::new(note).color(MUTED));
                        });
                    }
                });
            });
        });
        if let Some(action) = self.services.confirm {
            let mut close = false;
            egui::Modal::new(egui::Id::new("svc_confirm")).show(ui.ctx(), |ui| {
                ui.set_width(400.0);
                let (title, body) = match action {
                    0 => ("Ripristinare i servizi?", "Tutti i servizi cambiati da Xiox tornano come erano prima."),
                    services::START_MANUAL => ("Impostare su Manuale?", "I servizi si fermano subito e Windows li avvierà solo quando servono."),
                    _ => ("Disattivare i servizi?", "I servizi si fermano subito e non partiranno più. Potrai sempre usare «Ripristina originali»."),
                };
                ui.label(RichText::new(title).size(18.0).strong());
                ui.add_space(6.0);
                ui.label(body);
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if primary(ui, "Continua", true).clicked() {
                        let notifier = Arc::new(Repaint(ui.ctx().clone()));
                        if action == 0 {
                            self.services.apply(None, notifier);
                        } else {
                            self.services.apply(Some(action), notifier);
                        }
                        close = true;
                    }
                    if secondary(ui, "Annulla", true).clicked() {
                        close = true;
                    }
                });
            });
            if close {
                self.services.confirm = None;
            }
        }
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else { return };
        ctx.request_repaint();
        let busy = self.cleanup.clean_job.is_some()
            || self.registry.busy()
            || self.services.busy()
            || self.dev.busy()
            || self.dev.demo_pending()
            || self.cleanup.rules_rx.is_some();
        // L'analisi Sviluppatori dura circa un minuto senza amministratore: la si aspetta fino a 5 minuti.
        let dev_wait = self.dev.busy() && self.started.elapsed() < Duration::from_secs(300);
        if self.frame >= 120 && (!busy || self.frame >= 900) && !dev_wait && !self.shot_sent {
            self.shot_sent = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let shots: Vec<std::sync::Arc<egui::ColorImage>> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None })
                .collect()
        });
        if let Some(img) = shots.first() {
            let [w, h] = img.size;
            let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, bytes) {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = buf.save(&path);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// Gruppi dinamici del pannello sinistro di Pulizia file: gli inglobati per
/// gruppo ("Windows", "Browser", "Sviluppo e AI", `rules::GROUP_ORDER`; un
/// `CollapsingHeader` per cleaner) e "Applicazioni" (Winapp2, uno per sezione,
/// con «Cerca»). Il corpo di un `CollapsingHeader` si costruisce solo se aperto.
/// `status`: riga di stato di Winapp2 e motivo al passaggio del mouse (`rules::status_line`).
#[allow(clippy::too_many_arguments)]
fn rules_panel(ui: &mut Ui, rs: &rules::RuleSet, natives: Natives, status: (String, Option<String>), sel: &mut HashSet<String>, search: &mut String, processes: &[String], busy: bool) {
    group_panel(ui, Some(rs), natives, &rules::panel_groups(&rs.groups, natives.items), sel, processes, busy);
    ui.add_space(6.0);
    ui.label(RichText::new(rules::WINAPP2_GROUP).strong().color(ACCENT));
    ui.separator();
    let (text, why) = status;
    let hover: Vec<&str> = why.as_deref().into_iter().chain(rs.warnings.iter().map(String::as_str)).collect();
    let status = ui.label(RichText::new(text).color(MUTED).size(12.0));
    if !hover.is_empty() {
        status.on_hover_text(hover.join("\n"));
    }
    if rs.sections.is_empty() {
        return;
    }
    ui.add(egui::TextEdit::singleline(search).hint_text("Cerca").desired_width(f32::INFINITY));
    let needle = search.trim().to_lowercase();
    let hit = |text: &str| text.to_lowercase().contains(&needle);
    for (section, idx) in &rs.sections {
        let whole = needle.is_empty() || hit(section);
        let items: Vec<&xiox_rules::Cleaner> = idx
            .iter()
            .map(|&i| &rs.cleaners[i])
            .filter(|c| whole || hit(&c.label) || c.options.iter().any(|o| hit(&o.label)))
            .collect();
        if items.is_empty() {
            continue;
        }
        egui::CollapsingHeader::new(format!("{section} ({})", items.len()))
            .id_salt(("winapp2", section))
            .default_open(false)
            .show(ui, |ui| {
                for c in items {
                    for o in &c.options {
                        rule_checkbox(ui, c, o, &c.label, sel, processes, busy);
                    }
                }
            });
    }
}

/// Le voci native e se l'app è amministratore (per il 🛡).
#[derive(Clone, Copy)]
struct Natives<'a> {
    items: &'a [native::NativeItem],
    admin: bool,
}

/// I gruppi dei cleaner inglobati (`rules::panel_groups`): titolo, poi le voci
/// native del gruppo come caselle, poi un `CollapsingHeader` per cleaner XML.
fn group_panel(ui: &mut Ui, rs: Option<&rules::RuleSet>, natives: Natives, groups: &[(String, Vec<usize>, Vec<usize>)], sel: &mut HashSet<String>, processes: &[String], busy: bool) {
    for (group, nat, idx) in groups {
        ui.add_space(6.0);
        ui.label(RichText::new(group).strong().color(ACCENT));
        ui.separator();
        for item in nat.iter().map(|&n| &natives.items[n]) {
            native_checkbox(ui, item, natives.admin, sel, processes, busy);
        }
        let Some(rs) = rs else { continue };
        for c in idx.iter().map(|&i| &rs.cleaners[i]) {
            let header = egui::CollapsingHeader::new(&c.label).id_salt(("rule", &c.id)).default_open(false).show(ui, |ui| {
                for o in &c.options {
                    rule_checkbox(ui, c, o, &o.label, sel, processes, busy);
                }
            });
            if !c.description.is_empty() {
                header.header_response.on_hover_text(&c.description);
            }
        }
    }
}

/// Casella di una voce nativa: come le categorie statiche (🛡 se serve
/// l'amministratore), disattivata se un suo processo è attivo, avviso al
/// passaggio del mouse.
fn native_checkbox(ui: &mut Ui, item: &native::NativeItem, admin: bool, sel: &mut HashSet<String>, processes: &[String], busy: bool) {
    let key = rules::native_key(item);
    let label = if item.needs_admin && !admin { format!("{}  🛡", item.label) } else { item.label.to_string() };
    if let Some(why) = rules::native_blocked(item, processes) {
        let mut off = false;
        ui.add_enabled(false, egui::Checkbox::new(&mut off, label)).on_disabled_hover_text(&why);
        ui.label(RichText::new(why).color(MUTED).size(12.0));
        return;
    }
    let mut help = item.description.to_string();
    if let Some(w) = item.warning {
        help.push_str("\n\nAttenzione: ");
        help.push_str(w);
    }
    let mut on = sel.contains(&key);
    let r = ui.add_enabled(!busy, egui::Checkbox::new(&mut on, label)).on_hover_text(help);
    if r.changed() {
        if on {
            sel.insert(key);
        } else {
            sel.remove(&key);
        }
    }
    if item.id == browserdb::COOKIES_ID {
        ui.add_space(2.0);
        let hint = "Apre Documenti\\Xiox\\cookie_da_conservare.txt nel Blocco note (lo crea se manca). \
                    Un dominio per riga: i suoi cookie restano.";
        if secondary(ui, "Modifica lista cookie", true).on_hover_text(hint).clicked() {
            open_keep_list();
        }
        ui.add_space(4.0);
    }
}

/// Apre la lista dei cookie da conservare in un thread a parte (crearla e
/// avviare l'editor non deve fermare la UI); un errore si mostra in una finestra.
fn open_keep_list() {
    std::thread::spawn(|| {
        if let Err(e) = browserdb::open_keep_list() {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("Lista dei cookie da conservare")
                .set_description(format!("Non è stato possibile aprire la lista: {e}"))
                .show();
        }
    });
}

/// Elenco completo di una voce nativa sotto la tabella (collegamenti trovati,
/// canali svuotati, note), aperto: si legge prima di «Avvia pulizia».
fn native_list(ui: &mut Ui, key: &str, label: &str, lines: &[String]) {
    if lines.len() < 2 {
        return;
    }
    ui.add_space(8.0);
    egui::CollapsingHeader::new(format!("{label}: elenco ({} righe)", lines.len()))
        .id_salt(("native_list", key))
        .default_open(true)
        .show(ui, |ui| {
            for l in lines {
                ui.label(RichText::new(l).color(MUTED).size(12.0));
            }
        });
}

/// Ultima colonna delle tabelle: la prima riga, le altre al passaggio del mouse.
fn note_cell(ui: &mut Ui, lines: &[String]) {
    match lines.first() {
        Some(first) => {
            ui.label(RichText::new(first).color(MUTED).size(12.0)).on_hover_text(lines.join("\n"));
        }
        None => {
            ui.label("");
        }
    }
}

/// Casella di un'opzione dinamica: disattivata se non supportata (spiegazione al
/// passaggio del mouse) o se un suo processo è attivo ("Chiudi … per pulire").
fn rule_checkbox(ui: &mut Ui, c: &xiox_rules::Cleaner, o: &xiox_rules::Opt, label: &str, sel: &mut HashSet<String>, processes: &[String], busy: bool) {
    let key = rules::key(c, o);
    if let Some(why) = rules::blocked(c, o, processes) {
        let mut off = false;
        ui.add_enabled(false, egui::Checkbox::new(&mut off, label)).on_disabled_hover_text(&why);
        if o.unsupported.is_none() {
            ui.label(RichText::new(why).color(MUTED).size(12.0));
        }
        return;
    }
    let mut on = sel.contains(&key);
    let mut help = o.description.clone();
    if let Some(w) = &o.warning {
        if !help.is_empty() {
            help.push_str("\n\n");
        }
        help.push_str("Attenzione: ");
        help.push_str(w);
    }
    let mut r = ui.add_enabled(!busy, egui::Checkbox::new(&mut on, label));
    if !help.is_empty() {
        r = r.on_hover_text(help);
    }
    if r.changed() {
        if on {
            sel.insert(key);
        } else {
            sel.remove(&key);
        }
    }
}

fn cell(ui: &mut Ui, w: f32, text: impl Into<egui::WidgetText>) -> egui::Response {
    ui.allocate_ui_with_layout(Vec2::new(w, 20.0), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_width(w);
        ui.set_max_width(w);
        ui.add(egui::Label::new(text).truncate())
    })
    .inner
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame += 1;
        self.poll(ctx);
        if self.cleanup.clean_job.is_some() || self.registry.busy() || self.services.busy() || self.dev.busy() {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        self.sidebar(ctx);
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG)).show(ctx, |ui| match self.tab {
            Tab::Cleaner => self.cleaner_tab(ui),
            Tab::Registry => self.registry_tab(ui),
            Tab::Services => self.services_tab(ui),
            Tab::Dev => {
                if self.mismatch.is_some() {
                    egui::TopBottomPanel::top("dev_profile")
                        .frame(egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 12, bottom: 0 }))
                        .show_inside(ui, |ui| self.profile_strip(ui));
                }
                dev::dev_tab(&mut self.dev, ui, self.admin)
            }
        });
        self.about_window(ctx);
        self.handle_screenshot(ctx);
    }

    /// Con --screenshot il puntatore non c'è: niente evidenziazioni né suggerimenti dove
    /// capita che il cursore stia (sotto Xvfb sta al centro della finestra).
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if self.screenshot.is_some() {
            raw_input.events.retain(|e| !matches!(e, egui::Event::PointerMoved(_) | egui::Event::MouseMoved(_)));
            raw_input.events.push(egui::Event::PointerGone);
        }
    }
}

fn style(ctx: &egui::Context) {
    let mut v = egui::Visuals::light();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT;
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xe8, 0xf0, 0xfd);
    v.widgets.inactive.bg_fill = Color32::from_rgb(0xf7, 0xf8, 0xfa);
    v.extreme_bg_color = Color32::from_rgb(0xe9, 0xeb, 0xef);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Color32::from_gray(175));
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.text_styles.insert(TextStyle::Body, FontId::proportional(14.0));
        s.text_styles.insert(TextStyle::Button, FontId::proportional(14.0));
        s.text_styles.insert(TextStyle::Small, FontId::proportional(12.0));
        s.spacing.item_spacing = Vec2::new(8.0, 6.0);
        s.spacing.button_padding = Vec2::new(12.0, 5.0);
        s.spacing.interact_size.y = 22.0;
    });
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut screenshot = None;
    let mut tab = Tab::Cleaner;
    let mut analyze = false;
    let mut about = false;
    let mut section = dev::Section::Projects;
    let mut only_root: Option<PathBuf> = None;
    #[cfg(debug_assertions)]
    let mut demo = None;
    #[cfg(debug_assertions)]
    let mut clean_demo = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--screenshot" if i + 1 < args.len() => {
                screenshot = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
            "--tab" if i + 1 < args.len() => {
                tab = match args[i + 1].as_str() {
                    "registry" | "registro" => Tab::Registry,
                    "services" | "servizi" => Tab::Services,
                    "dev" | "sviluppatori" => Tab::Dev,
                    _ => Tab::Cleaner,
                };
                i += 1;
            }
            "--analyze" => analyze = true,
            // Come --tab: apre una vista all'avvio (per gli screenshot della finestra Informazioni).
            "--about" => about = true,
            "--dev-section" if i + 1 < args.len() => {
                section = dev::Section::parse(&args[i + 1]).unwrap_or(section);
                i += 1;
            }
            // Verifiche su un albero finto: si cerca solo lì, impostazioni in memoria.
            "--only-root" if i + 1 < args.len() => {
                only_root = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
            // Solo build di debug e solo con --only-root su un albero finto: conferma, pulizia o
            // annulla da soli. Nelle build di rilascio non esiste.
            #[cfg(debug_assertions)]
            "--dev-demo" if i + 1 < args.len() => {
                demo = dev::Demo::parse(&args[i + 1]);
                i += 1;
            }
            // Solo build di debug e solo con --screenshot: un resoconto di Pulizia file inventato
            // (nessun file letto o toccato). Nelle build di rilascio non esiste.
            #[cfg(debug_assertions)]
            "--clean-demo" => clean_demo = true,
            _ => {}
        }
        i += 1;
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Xiox")
        .with_inner_size([1100.0, 700.0])
        .with_min_inner_size([900.0, 560.0]);
    if screenshot.is_some() {
        viewport = viewport.with_active(false).with_window_level(egui::WindowLevel::AlwaysOnBottom);
    }
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Glow, ..Default::default() };
    // Cartelle di «Pulisci spazio libero» rimaste da uno spegnimento brusco:
    // via subito, in un thread a parte (mai negli screenshot né su un albero finto).
    if screenshot.is_none() && only_root.is_none() {
        wipe::remove_leftovers_in_background();
    }
    eframe::run_native(
        "Xiox",
        options,
        Box::new(move |cc| {
            style(&cc.egui_ctx);
            let mut dev = match only_root {
                Some(dir) => dev::DevState::only_root(dir, section, analyze),
                None => dev::DevState::new(section, analyze),
            };
            if screenshot.is_some() {
                dev.set_screenshot();
            }
            #[cfg(debug_assertions)]
            if let Some(d) = demo {
                if let Err(e) = dev.set_demo(d) {
                    eprintln!("{e}");
                    return Err(e.into());
                }
            }
            #[cfg(debug_assertions)]
            let clean_demo = clean_demo && screenshot.is_some();
            let mut app = App::new(screenshot, tab, dev, about);
            #[cfg(debug_assertions)]
            if clean_demo {
                app.clean_demo();
            }
            app.start_rules(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}
