#![windows_subsystem = "windows"]

mod cleaner;
mod registry;
mod services;
mod sys;

use eframe::egui::{self, Align, Color32, CornerRadius, FontId, Layout, Margin, RichText, Stroke, TextStyle, Ui, Vec2};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

const SIDEBAR: Color32 = Color32::from_rgb(0x23, 0x26, 0x2c);
const SIDEBAR_SEL: Color32 = Color32::from_rgb(0x36, 0x3a, 0x43);
const ACCENT: Color32 = Color32::from_rgb(0x2f, 0x80, 0xed);
const GREEN: Color32 = Color32::from_rgb(0x2e, 0xa8, 0x62);
const BG: Color32 = Color32::from_rgb(0xf3, 0xf4, 0xf6);
const CARD: Color32 = Color32::WHITE;
const TEXT: Color32 = Color32::from_rgb(0x2b, 0x2e, 0x33);
const MUTED: Color32 = Color32::from_rgb(0x7a, 0x7f, 0x87);
const WARN_BG: Color32 = Color32::from_rgb(0xff, 0xf4, 0xd6);
const LINE: Color32 = Color32::from_rgb(0xe1, 0xe3, 0xe7);

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Cleaner,
    Registry,
    Services,
}

enum CleanMsg {
    Progress(String, f32),
    Analyzed(usize, cleaner::Found),
    Cleaned(usize, cleaner::Found, cleaner::Cleaned),
    Done,
}

enum RegMsg {
    Progress(String, f32),
    Found(Vec<registry::Issue>),
    Done,
    Fixed { ok: usize, failed: usize, backup: Option<String> },
}

enum SvcMsg {
    Loaded(Vec<services::Service>),
    Message(String),
}

#[derive(PartialEq)]
enum CleanView {
    Empty,
    Analyzed,
    Cleaned,
}

struct App {
    tab: Tab,
    admin: bool,
    frame: u64,
    screenshot: Option<PathBuf>,
    shot_sent: bool,
    mem: sys::Memory,
    mem_at: Instant,

    clean_sel: Vec<bool>,
    only_old: bool,
    clean_job: Option<Receiver<CleanMsg>>,
    clean_text: String,
    clean_progress: f32,
    found: Vec<Option<cleaner::Found>>,
    cleaned: Vec<Option<cleaner::Cleaned>>,
    clean_view: CleanView,
    confirm_clean: bool,

    reg_sel: Vec<bool>,
    issues: Vec<registry::Issue>,
    reg_job: Option<Receiver<RegMsg>>,
    reg_text: String,
    reg_progress: f32,
    reg_scanned: bool,
    reg_confirm: bool,
    reg_msg: String,

    services: Vec<services::Service>,
    svc_job: Option<Receiver<SvcMsg>>,
    svc_loaded: bool,
    svc_all: bool,
    svc_filter: String,
    svc_msg: String,
    svc_confirm: Option<u32>,
}

impl App {
    fn new(screenshot: Option<PathBuf>, tab: Tab) -> Self {
        let n = cleaner::CATEGORIES.len();
        let mut app = App {
            tab,
            admin: sys::is_admin(),
            frame: 0,
            screenshot,
            shot_sent: false,
            mem: sys::memory(),
            mem_at: Instant::now(),
            clean_sel: cleaner::CATEGORIES.iter().map(|c| c.default_on).collect(),
            only_old: true,
            clean_job: None,
            clean_text: String::new(),
            clean_progress: 0.0,
            found: vec![None; n],
            cleaned: vec![None; n],
            clean_view: CleanView::Empty,
            confirm_clean: false,
            reg_sel: registry::CATEGORIES.iter().map(|c| c.default_on).collect(),
            issues: vec![],
            reg_job: None,
            reg_text: String::new(),
            reg_progress: 0.0,
            reg_scanned: false,
            reg_confirm: false,
            reg_msg: String::new(),
            services: vec![],
            svc_job: None,
            svc_loaded: false,
            svc_all: false,
            svc_filter: String::new(),
            svc_msg: String::new(),
            svc_confirm: None,
        };
        if tab == Tab::Services {
            app.load_services();
        }
        app
    }

    // ---------- background jobs ----------

    fn start_clean(&mut self, also_clean: bool, ctx: &egui::Context) {
        let cats: Vec<usize> = (0..self.clean_sel.len()).filter(|&i| self.clean_sel[i]).collect();
        let (tx, rx) = channel();
        self.clean_job = Some(rx);
        self.found = vec![None; self.clean_sel.len()];
        self.cleaned = vec![None; self.clean_sel.len()];
        self.clean_view = if also_clean { CleanView::Cleaned } else { CleanView::Analyzed };
        let only_old = self.only_old;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let total = cats.len().max(1) as f32;
            for (k, &i) in cats.iter().enumerate() {
                let name = cleaner::CATEGORIES[i].name;
                let _ = tx.send(CleanMsg::Progress(format!("{} – {}", if also_clean { "Pulizia" } else { "Analisi" }, name), k as f32 / total));
                ctx.request_repaint();
                let f = cleaner::analyze(i, only_old);
                if also_clean {
                    let c = cleaner::clean(i, &f);
                    let _ = tx.send(CleanMsg::Cleaned(i, f, c));
                } else {
                    let _ = tx.send(CleanMsg::Analyzed(i, f));
                }
            }
            let _ = tx.send(CleanMsg::Done);
            ctx.request_repaint();
        });
    }

    fn start_reg_scan(&mut self, ctx: &egui::Context) {
        let cats: Vec<usize> = (0..self.reg_sel.len()).filter(|&i| self.reg_sel[i]).collect();
        let (tx, rx) = channel();
        self.reg_job = Some(rx);
        self.issues.clear();
        self.reg_msg.clear();
        self.reg_scanned = true;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let total = cats.len().max(1) as f32;
            for (k, &i) in cats.iter().enumerate() {
                let name = registry::CATEGORIES[i].name;
                let _ = tx.send(RegMsg::Progress(format!("Controllo: {name}"), k as f32 / total));
                ctx.request_repaint();
                let found = registry::scan(i, &|_| {});
                let _ = tx.send(RegMsg::Found(found));
            }
            let _ = tx.send(RegMsg::Done);
            ctx.request_repaint();
        });
    }

    fn start_reg_fix(&mut self, with_backup: bool, ctx: &egui::Context) {
        let chosen: Vec<registry::Issue> = self.issues.iter().filter(|i| i.selected).cloned().collect();
        let (tx, rx) = channel();
        self.reg_job = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(RegMsg::Progress("Salvataggio del backup…".into(), 0.0));
            let backup = if with_backup {
                let refs: Vec<&registry::Issue> = chosen.iter().collect();
                registry::backup(&refs).ok().map(|p| p.to_string_lossy().to_string())
            } else {
                None
            };
            let (mut ok, mut failed) = (0, 0);
            let total = chosen.len().max(1) as f32;
            for (k, issue) in chosen.iter().enumerate() {
                if k % 20 == 0 {
                    let _ = tx.send(RegMsg::Progress("Riparazione in corso…".into(), k as f32 / total));
                    ctx.request_repaint();
                }
                if registry::apply(&issue.fix) {
                    ok += 1;
                } else {
                    failed += 1;
                }
            }
            let _ = tx.send(RegMsg::Fixed { ok, failed, backup });
            ctx.request_repaint();
        });
    }

    fn load_services(&mut self) {
        let (tx, rx) = channel();
        self.svc_job = Some(rx);
        self.svc_loaded = true;
        std::thread::spawn(move || {
            let _ = tx.send(SvcMsg::Loaded(services::load()));
        });
    }

    fn apply_services(&mut self, start: Option<u32>) {
        let chosen: Vec<services::Service> = self.services.iter().filter(|s| s.selected && !s.protected).cloned().collect();
        let (tx, rx) = channel();
        self.svc_job = Some(rx);
        std::thread::spawn(move || send_svc(tx, chosen, start));
    }

    fn poll(&mut self) {
        if let Some(rx) = &self.clean_job {
            let mut done = false;
            while let Ok(m) = rx.try_recv() {
                match m {
                    CleanMsg::Progress(t, p) => {
                        self.clean_text = t;
                        self.clean_progress = p;
                    }
                    CleanMsg::Analyzed(i, f) => self.found[i] = Some(f),
                    CleanMsg::Cleaned(i, f, c) => {
                        self.found[i] = Some(f);
                        self.cleaned[i] = Some(c);
                    }
                    CleanMsg::Done => done = true,
                }
            }
            if done {
                self.clean_job = None;
            }
        }
        if let Some(rx) = &self.reg_job {
            let mut done = false;
            while let Ok(m) = rx.try_recv() {
                match m {
                    RegMsg::Progress(t, p) => {
                        self.reg_text = t;
                        self.reg_progress = p;
                    }
                    RegMsg::Found(v) => self.issues.extend(v),
                    RegMsg::Done => done = true,
                    RegMsg::Fixed { ok, failed, backup } => {
                        self.reg_msg = format!("Riparati {ok} problemi.");
                        if failed > 0 {
                            self.reg_msg.push_str(&format!(" {failed} non riparati (servono i diritti di amministratore)."));
                        }
                        if let Some(b) = backup {
                            self.reg_msg.push_str(&format!("\nBackup salvato in: {b}"));
                        }
                        if failed == 0 {
                            self.issues.retain(|i| !i.selected);
                        }
                        done = true;
                    }
                }
            }
            if done {
                self.reg_job = None;
            }
        }
        if let Some(rx) = &self.svc_job {
            let mut reload = false;
            let mut done = false;
            while let Ok(m) = rx.try_recv() {
                match m {
                    SvcMsg::Loaded(v) => {
                        self.services = v;
                        done = true;
                    }
                    SvcMsg::Message(t) => {
                        self.svc_msg = t;
                        reload = true;
                    }
                }
            }
            if done {
                self.svc_job = None;
            }
            if reload {
                self.svc_job = None;
                self.load_services();
            }
        }
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
                        ui.label(RichText::new("PulisciPC").color(Color32::WHITE).size(18.0).strong());
                        ui.label(RichText::new("Pulizia leggera").color(Color32::from_gray(150)).size(12.0));
                    });
                });
                ui.add_space(22.0);
                for (tab, icon, label) in [
                    (Tab::Cleaner, "🗑", "Pulizia file"),
                    (Tab::Registry, "🔧", "Registro"),
                    (Tab::Services, "⚙", "Servizi e RAM"),
                ] {
                    let sel = self.tab == tab;
                    let fill = if sel { SIDEBAR_SEL } else { SIDEBAR };
                    let text = RichText::new(format!("  {icon}   {label}")).size(15.0).color(if sel { Color32::WHITE } else { Color32::from_gray(190) });
                    let resp = ui.add(egui::Button::new(text).fill(fill).stroke(Stroke::NONE).corner_radius(0).min_size(Vec2::new(212.0, 46.0)));
                    if sel {
                        ui.painter().rect_filled(egui::Rect::from_min_size(resp.rect.min, Vec2::new(4.0, resp.rect.height())), 0.0, ACCENT);
                    }
                    if resp.clicked() {
                        self.tab = tab;
                        if tab == Tab::Services && !self.svc_loaded {
                            self.load_services();
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
                });
            });
    }

    fn admin_strip(&mut self, ui: &mut Ui, what: &str) {
        if self.admin {
            return;
        }
        egui::Frame::new().fill(WARN_BG).corner_radius(6).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 230.0).max(150.0);
                ui.vertical(|ui| {
                    ui.set_max_width(w);
                    ui.add(egui::Label::new(RichText::new(format!("⚠  {what}")).color(TEXT)).wrap());
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Riavvia come amministratore").clicked() && sys::relaunch_as_admin() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        });
        ui.add_space(8.0);
    }

    fn cleaner_tab(&mut self, ui: &mut Ui) {
        let busy = self.clean_job.is_some();
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
                            ui.checkbox(&mut self.clean_sel[i], label).on_hover_text(c.help);
                        });
                    }
                    ui.add_space(12.0);
                    ui.label(RichText::new("Opzioni").strong().color(ACCENT));
                    ui.separator();
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.checkbox(&mut self.only_old, format!("Elimina dalle cartelle Temp solo i file più vecchi di {} ore", cleaner::TEMP_MIN_AGE_HOURS));
                    });
                });
            });
        egui::TopBottomPanel::bottom("clean_buttons")
            .frame(egui::Frame::new().inner_margin(Margin::symmetric(16, 12)))
            .show_inside(ui, |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let any = self.clean_sel.iter().any(|&b| b);
                    if primary(ui, "Avvia pulizia", !busy && any).clicked() {
                        self.confirm_clean = true;
                    }
                    if secondary(ui, "Analizza", !busy && any).clicked() {
                        let ctx = ui.ctx().clone();
                        self.start_clean(false, &ctx);
                    }
                });
            });
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin::same(16))).show_inside(ui, |ui| {
            self.admin_strip(ui, "Le voci con lo scudo richiedono i diritti di amministratore.");
            card(ui, |ui| {
                if busy {
                    ui.label(RichText::new(&self.clean_text).size(16.0).color(TEXT));
                    ui.add_space(6.0);
                    ui.add(egui::ProgressBar::new(self.clean_progress).fill(ACCENT).show_percentage());
                    ui.add_space(10.0);
                }
                match self.clean_view {
                    CleanView::Empty => {
                        ui.add_space(30.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("Pulizia dei file temporanei").size(22.0).strong().color(TEXT));
                            ui.add_space(6.0);
                            ui.label(RichText::new("Scegli a sinistra cosa pulire, poi premi «Analizza» per vedere quanto spazio puoi liberare.").color(MUTED));
                        });
                    }
                    CleanView::Analyzed => self.analysis_table(ui),
                    CleanView::Cleaned => self.cleaned_table(ui),
                }
            });
        });
        if self.confirm_clean {
            let mut close = false;
            egui::Modal::new(egui::Id::new("confirm_clean")).show(ui.ctx(), |ui| {
                ui.set_width(380.0);
                ui.label(RichText::new("Avviare la pulizia?").size(18.0).strong());
                ui.add_space(6.0);
                ui.label("I file selezionati verranno eliminati definitivamente. I file in uso vengono lasciati dove sono.");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if primary(ui, "Continua", true).clicked() {
                        let ctx = ui.ctx().clone();
                        self.start_clean(true, &ctx);
                        close = true;
                    }
                    if secondary(ui, "Annulla", true).clicked() {
                        close = true;
                    }
                });
            });
            if close {
                self.confirm_clean = false;
            }
        }
    }

    fn analysis_table(&self, ui: &mut Ui) {
        let total: u64 = self.found.iter().flatten().map(|f| f.bytes).sum();
        let files: u64 = self.found.iter().flatten().map(|f| f.count).sum();
        let title = if self.clean_job.is_some() { "Analisi in corso…".to_string() } else { "Analisi completata".to_string() };
        ui.label(RichText::new(title).size(20.0).strong().color(TEXT));
        ui.label(RichText::new(format!("{} da eliminare ({} file)", sys::fmt_size(total), files)).size(15.0).color(ACCENT));
        ui.add_space(10.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("an").striped(true).min_col_width(80.0).spacing([24.0, 8.0]).show(ui, |ui| {
                ui.label(RichText::new("Elemento").strong());
                ui.label(RichText::new("Dimensione").strong());
                ui.label(RichText::new("File").strong());
                ui.label("");
                ui.end_row();
                for (i, f) in self.found.iter().enumerate() {
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
            });
        });
    }

    fn cleaned_table(&self, ui: &mut Ui) {
        let freed: u64 = self.cleaned.iter().flatten().map(|c| c.freed).sum();
        let title = if self.clean_job.is_some() { "Pulizia in corso…" } else { "Pulizia completata" };
        ui.label(RichText::new(title).size(20.0).strong().color(TEXT));
        ui.label(RichText::new(format!("Liberati {}", sys::fmt_size(freed))).size(15.0).color(GREEN));
        ui.add_space(10.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("cl").striped(true).min_col_width(80.0).spacing([24.0, 8.0]).show(ui, |ui| {
                ui.label(RichText::new("Elemento").strong());
                ui.label(RichText::new("Liberati").strong());
                ui.label(RichText::new("Eliminati").strong());
                ui.label(RichText::new("Lasciati (in uso)").strong());
                ui.end_row();
                for (i, c) in self.cleaned.iter().enumerate() {
                    if let Some(c) = c {
                        ui.label(cleaner::CATEGORIES[i].name);
                        ui.label(sys::fmt_size(c.freed));
                        ui.label(c.removed.to_string());
                        ui.label(c.failed.to_string());
                        ui.end_row();
                    }
                }
            });
        });
    }

    fn registry_tab(&mut self, ui: &mut Ui) {
        let busy = self.reg_job.is_some();
        egui::SidePanel::left("reg_opts")
            .exact_width(270.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(CARD).inner_margin(Margin::same(14)).stroke(Stroke::new(1.0_f32, LINE)))
            .show_inside(ui, |ui| {
                ui.label(RichText::new("Registro").strong().color(ACCENT));
                ui.separator();
                ui.add_enabled_ui(!busy, |ui| {
                    for (i, c) in registry::CATEGORIES.iter().enumerate() {
                        ui.checkbox(&mut self.reg_sel[i], c.name);
                    }
                });
                ui.add_space(14.0);
                ui.label(RichText::new("Prima di riparare puoi salvare un backup: con un doppio clic sul file .reg si torna indietro.").size(12.0).color(MUTED));
            });
        egui::TopBottomPanel::bottom("reg_buttons")
            .frame(egui::Frame::new().inner_margin(Margin::symmetric(16, 12)))
            .show_inside(ui, |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let sel = self.issues.iter().filter(|i| i.selected).count();
                    if primary(ui, &format!("Ripara selezionati ({sel})…"), !busy && sel > 0).clicked() {
                        self.reg_confirm = true;
                    }
                    if secondary(ui, "Cerca problemi", !busy && self.reg_sel.iter().any(|&b| b)).clicked() {
                        let ctx = ui.ctx().clone();
                        self.start_reg_scan(&ctx);
                    }
                });
            });
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin::same(16))).show_inside(ui, |ui| {
            self.admin_strip(ui, "Senza diritti di amministratore si riparano solo le voci del tuo utente.");
            card(ui, |ui| {
                if busy {
                    ui.label(RichText::new(&self.reg_text).size(16.0).color(TEXT));
                    ui.add_space(6.0);
                    ui.add(egui::ProgressBar::new(self.reg_progress).fill(ACCENT).show_percentage());
                    ui.add_space(10.0);
                }
                if !self.reg_msg.is_empty() {
                    ui.label(RichText::new(&self.reg_msg).color(GREEN));
                    ui.add_space(8.0);
                }
                if !self.reg_scanned {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Pulizia del registro").size(22.0).strong().color(TEXT));
                        ui.add_space(6.0);
                        ui.label(RichText::new("Premi «Cerca problemi» per trovare voci che puntano a file o programmi che non esistono più.").color(MUTED));
                    });
                    return;
                }
                if self.issues.is_empty() && !busy {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new("✔  Nessun problema trovato").size(18.0).color(GREEN)));
                    return;
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} problemi trovati", self.issues.len())).size(18.0).strong().color(TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Nessuno").clicked() {
                            self.issues.iter_mut().for_each(|i| i.selected = false);
                        }
                        if ui.small_button("Tutti").clicked() {
                            self.issues.iter_mut().for_each(|i| i.selected = true);
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
                let n = self.issues.len();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 22.0, n, |ui, range| {
                    for i in range {
                        let it = &mut self.issues[i];
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
        if self.reg_confirm {
            let mut close = false;
            egui::Modal::new(egui::Id::new("reg_confirm")).show(ui.ctx(), |ui| {
                ui.set_width(400.0);
                ui.label(RichText::new("Salvare un backup del registro?").size(18.0).strong());
                ui.add_space(6.0);
                ui.label("Consigliato: prima delle modifiche viene salvata una copia (.reg) nella cartella Documenti\\PulisciPC.");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if primary(ui, "Sì, salva e ripara", true).clicked() {
                        let ctx = ui.ctx().clone();
                        self.start_reg_fix(true, &ctx);
                        close = true;
                    }
                    if secondary(ui, "Ripara senza backup", true).clicked() {
                        let ctx = ui.ctx().clone();
                        self.start_reg_fix(false, &ctx);
                        close = true;
                    }
                    if ui.button("Annulla").clicked() {
                        close = true;
                    }
                });
            });
            if close {
                self.reg_confirm = false;
            }
        }
    }

    fn services_tab(&mut self, ui: &mut Ui) {
        if self.mem_at.elapsed() > Duration::from_secs(2) {
            self.mem = sys::memory();
            self.mem_at = Instant::now();
        }
        ui.ctx().request_repaint_after(Duration::from_secs(2));
        let busy = self.svc_job.is_some();
        egui::TopBottomPanel::bottom("svc_buttons")
            .frame(egui::Frame::new().inner_margin(Margin::symmetric(16, 12)))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    let sel: Vec<&services::Service> = self.services.iter().filter(|s| s.selected && !s.protected).collect();
                    let kb: u64 = sel.iter().map(|s| s.freeable_kb()).sum();
                    ui.label(RichText::new(format!("Selezionati: {}  ·  RAM liberabile (stima): {}", sel.len(), sys::fmt_size(kb * 1024))).color(TEXT));
                    let n = sel.len();
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary(ui, "Disattiva selezionati", !busy && n > 0).clicked() {
                            self.svc_confirm = Some(services::START_DISABLED);
                        }
                        if secondary(ui, "Imposta su Manuale", !busy && n > 0).clicked() {
                            self.svc_confirm = Some(services::START_MANUAL);
                        }
                        let saved = services::read_backup().len();
                        if secondary(ui, &format!("Ripristina originali ({saved})"), !busy && saved > 0).clicked() {
                            self.svc_confirm = Some(0);
                        }
                    });
                });
            });
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin::same(16))).show_inside(ui, |ui| {
            self.admin_strip(ui, "Per cambiare i servizi servono i diritti di amministratore.");
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
                    ui.selectable_value(&mut self.svc_all, false, "Consigliati da spegnere");
                    ui.selectable_value(&mut self.svc_all, true, "Tutti i servizi");
                    ui.add_space(12.0);
                    ui.add(egui::TextEdit::singleline(&mut self.svc_filter).hint_text("Cerca…").desired_width(180.0));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add_enabled(!busy, egui::Button::new("⟳ Aggiorna")).clicked() {
                            self.load_services();
                        }
                    });
                });
                if !self.svc_msg.is_empty() {
                    ui.label(RichText::new(&self.svc_msg).color(GREEN));
                }
                ui.add_space(6.0);
                if busy && self.services.is_empty() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Lettura dei servizi…");
                    });
                    return;
                }
                let filter = self.svc_filter.to_lowercase();
                let rows: Vec<usize> = (0..self.services.len())
                    .filter(|&i| {
                        let s = &self.services[i];
                        (self.svc_all || s.suggestion.is_some())
                            && (filter.is_empty() || s.display.to_lowercase().contains(&filter) || s.name.to_lowercase().contains(&filter))
                    })
                    .collect();
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
                        let s = &mut self.services[i];
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
        if let Some(action) = self.svc_confirm {
            let mut close = false;
            egui::Modal::new(egui::Id::new("svc_confirm")).show(ui.ctx(), |ui| {
                ui.set_width(400.0);
                let (title, body) = match action {
                    0 => ("Ripristinare i servizi?", "Tutti i servizi cambiati da PulisciPC tornano come erano prima."),
                    services::START_MANUAL => ("Impostare su Manuale?", "I servizi si fermano subito e Windows li avvierà solo quando servono."),
                    _ => ("Disattivare i servizi?", "I servizi si fermano subito e non partiranno più. Potrai sempre usare «Ripristina originali»."),
                };
                ui.label(RichText::new(title).size(18.0).strong());
                ui.add_space(6.0);
                ui.label(body);
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if primary(ui, "Continua", true).clicked() {
                        if action == 0 {
                            self.apply_services(None);
                        } else {
                            self.apply_services(Some(action));
                        }
                        close = true;
                    }
                    if secondary(ui, "Annulla", true).clicked() {
                        close = true;
                    }
                });
            });
            if close {
                self.svc_confirm = None;
            }
        }
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else { return };
        ctx.request_repaint();
        let busy = self.clean_job.is_some() || self.reg_job.is_some() || self.svc_job.is_some();
        if self.frame >= 120 && (!busy || self.frame >= 900) && !self.shot_sent {
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

fn send_svc(tx: Sender<SvcMsg>, chosen: Vec<services::Service>, start: Option<u32>) {
    let msg = match start {
        None => {
            let (ok, bad) = services::restore_all();
            if bad > 0 {
                format!("Ripristinati {ok} servizi, {bad} non riusciti.")
            } else {
                format!("Ripristinati {ok} servizi.")
            }
        }
        Some(s) => {
            let mut ok = 0;
            let mut bad = 0;
            for svc in &chosen {
                if services::set_start(svc, s) {
                    ok += 1;
                } else {
                    bad += 1;
                }
            }
            let what = if s == services::START_DISABLED { "disattivati" } else { "impostati su Manuale" };
            if bad > 0 {
                format!("{ok} servizi {what}, {bad} non riusciti (servono i diritti di amministratore).")
            } else {
                format!("{ok} servizi {what}.")
            }
        }
    };
    let _ = tx.send(SvcMsg::Message(msg));
}

fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(8)
        .stroke(Stroke::new(1.0_f32, LINE))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

fn cell(ui: &mut Ui, w: f32, text: impl Into<egui::WidgetText>) -> egui::Response {
    ui.allocate_ui_with_layout(Vec2::new(w, 20.0), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_width(w);
        ui.set_max_width(w);
        ui.add(egui::Label::new(text).truncate())
    })
    .inner
}

fn primary(ui: &mut Ui, text: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(Color32::WHITE).strong().size(15.0))
            .fill(if enabled { ACCENT } else { Color32::from_rgb(0xa8, 0xc4, 0xee) })
            .corner_radius(CornerRadius::same(5))
            .min_size(Vec2::new(150.0, 36.0)),
    )
}

fn secondary(ui: &mut Ui, text: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(ACCENT).strong().size(15.0))
            .fill(CARD)
            .stroke(Stroke::new(1.0_f32, ACCENT))
            .corner_radius(CornerRadius::same(5))
            .min_size(Vec2::new(130.0, 36.0)),
    )
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame += 1;
        self.poll();
        if self.clean_job.is_some() || self.reg_job.is_some() || self.svc_job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        self.sidebar(ctx);
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG)).show(ctx, |ui| match self.tab {
            Tab::Cleaner => self.cleaner_tab(ui),
            Tab::Registry => self.registry_tab(ui),
            Tab::Services => self.services_tab(ui),
        });
        self.handle_screenshot(ctx);
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
                    _ => Tab::Cleaner,
                };
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("PulisciPC")
        .with_inner_size([1100.0, 700.0])
        .with_min_inner_size([900.0, 560.0]);
    if screenshot.is_some() {
        viewport = viewport.with_active(false).with_window_level(egui::WindowLevel::AlwaysOnBottom);
    }
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Glow, ..Default::default() };
    eframe::run_native(
        "PulisciPC",
        options,
        Box::new(move |cc| {
            style(&cc.egui_ctx);
            Ok(Box::new(App::new(screenshot, tab)))
        }),
    )
}
