//! Raccolta di file e chiavi di registro da una regola.

use std::collections::HashSet;
use std::fs::Metadata;
use std::os::windows::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::expand::expand;
use crate::guard::Guard;
pub use crate::model::ExcludeRule;
use crate::model::{Action, Cleaner, Env, Opt, RegRoot, Search};
use crate::regguard::reg_allowed;
use crate::wildcard::wildcard_match;

/// Ciò che una regola toglierebbe.
///
/// Contratto di pulizia: la pulizia **deve** cancellare ogni voce con
/// [`crate::safedelete::delete_checked`], che rifà i controlli sullo stesso
/// handle con cui cancella (nessun collegamento seguito, percorso finale
/// uguale a quello raccolto, percorsi protetti): fra la raccolta e la pulizia
/// il disco può cambiare. Prima i `files` (un collegamento si toglie come
/// collegamento, mai il suo contenuto), poi le `dirs_to_remove` nell'ordine
/// dato (figli prima del padre), solo se vuote. Le chiavi di `reg` si
/// ricontrollano con [`crate::regguard::reg_allowed`] prima di cancellarle.
///
/// `skipped` conta ciò che si è scartato (protetti, collegamenti nel percorso,
/// cartelle illeggibili, chiavi di registro protette); `skipped_reasons` ne
/// spiega al più [`MAX_REASONS`], in italiano. `cancelled` è vero se la
/// raccolta è stata annullata: il risultato è incompleto e non va pulito.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Collected {
    pub files: Vec<(PathBuf, u64)>,
    pub dirs_to_remove: Vec<PathBuf>,
    pub reg: Vec<RegItem>,
    pub bytes: u64,
    pub skipped: u64,
    pub skipped_reasons: Vec<String>,
    pub cancelled: bool,
}

/// Quanti motivi di scarto si tengono in `Collected::skipped_reasons`.
pub const MAX_REASONS: usize = 20;

/// Chiave o valore di registro da cancellare.
///
/// `path` è relativo alla radice, **senza** `HKCU\`/`HKLM\` (`Software\…`):
/// chi scrive il backup `.reg` aggiunge `HKEY_CURRENT_USER\` o
/// `HKEY_LOCAL_MACHINE\` secondo `root`. Il percorso si usa così com'è, nella
/// vista a 64 bit del registro: la vista a 32 bit di HKLM si indica con
/// `Software\Wow6432Node\…` nel percorso stesso.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegItem {
    Key { root: RegRoot, path: String },
    Value { root: RegRoot, path: String, value: String },
}

/// Annullamento e avanzamento di una raccolta.
///
/// `cancel` si controlla prima di ogni azione e di ogni voce di cartella;
/// `tick` riceve il numero di voci (file e cartelle) esaminate finora, ogni
/// [`TICK_EVERY`] voci e una volta alla fine.
pub struct CollectCtl<'a> {
    pub cancel: &'a AtomicBool,
    pub tick: &'a (dyn Fn(u64) + Sync),
}

/// Ogni quante voci esaminate si chiama `CollectCtl::tick`.
pub const TICK_EVERY: u64 = 256;

pub fn collect(opt: &Opt, env: &Env, reg_exists: &dyn Fn(&RegItem) -> bool) -> Collected {
    collect_with_excludes(opt, env, &[], reg_exists)
}

/// Come `collect`, con in più i file che la regola (Winapp2 `ExcludeKey`) deve
/// lasciare: `pattern: None` esclude il percorso (file o cartella) e tutto ciò
/// che contiene; con un modello esclude, in quella cartella e sotto, i file
/// i cui nomi lo soddisfano (`;` separa più modelli). I componenti del
/// percorso di un'esclusione possono avere `*` e `?` (`ASUS\*\`).
pub fn collect_with_excludes(
    opt: &Opt,
    env: &Env,
    excludes: &[ExcludeRule],
    reg_exists: &dyn Fn(&RegItem) -> bool,
) -> Collected {
    run(opt, env, excludes, reg_exists, None)
}

/// Come `collect_with_excludes`, con annullamento e avanzamento.
pub fn collect_ctl(
    opt: &Opt,
    env: &Env,
    excludes: &[ExcludeRule],
    reg_exists: &dyn Fn(&RegItem) -> bool,
    ctl: &CollectCtl,
) -> Collected {
    run(opt, env, excludes, reg_exists, Some(ctl))
}

fn run(
    opt: &Opt,
    env: &Env,
    excludes: &[ExcludeRule],
    reg_exists: &dyn Fn(&RegItem) -> bool,
    ctl: Option<&CollectCtl>,
) -> Collected {
    let mut ctx = Ctx {
        env,
        guard: Guard::new(env),
        excludes: prepare_excludes(opt.excludes.iter().chain(excludes), env),
        out: Collected::default(),
        seen: HashSet::new(),
        ctl,
        examined: 0,
    };
    for action in &opt.actions {
        if ctx.cancelled() {
            break;
        }
        match action {
            Action::Files {
                path,
                patterns,
                recurse,
                remove_self,
                search,
            } => ctx.files_action(path, patterns, *recurse, *remove_self, *search),
            Action::RegKey { root, path } => ctx.reg_item(
                RegItem::Key {
                    root: *root,
                    path: path.clone(),
                },
                reg_exists,
            ),
            Action::RegValue { root, path, value } => ctx.reg_item(
                RegItem::Value {
                    root: *root,
                    path: path.clone(),
                    value: value.clone(),
                },
                reg_exists,
            ),
        }
    }
    if let Some(ctl) = ctl {
        (ctl.tick)(ctx.examined);
    }
    if ctx.cancelled() {
        ctx.out.cancelled = true;
        // Una cartella elencata a metà non è garantita vuota.
        ctx.out.dirs_to_remove.clear();
    }
    ctx.out
}

/// Nomi (come li scrive la regola) dei processi di `running` presenti in
/// `processes`. Il confronto ignora maiuscole e il suffisso `.exe`.
pub fn running(cleaner: &Cleaner, processes: &[String]) -> Vec<String> {
    let norm = |s: &str| {
        let s = s.trim().to_lowercase();
        s.strip_suffix(".exe").map(str::to_string).unwrap_or(s)
    };
    let active: Vec<String> = processes.iter().map(|p| norm(p)).collect();
    cleaner
        .running
        .iter()
        .filter(|r| {
            let n = norm(r);
            !n.is_empty() && active.contains(&n)
        })
        .cloned()
        .collect()
}

/// `true` se il cleaner è «installato»: esiste la cartella di base di almeno
/// un'azione su file, o almeno una chiave o un valore di registro (secondo
/// `reg_exists`, di solito [`reg_exists_real`]). Serve per i cleaner
/// inglobati, che non hanno `Detect`.
///
/// La cartella di base è la parte del percorso espanso prima del primo
/// componente con jolly (il percorso intero se non ne ha). Una base protetta
/// (`%UserProfile%`, la radice del disco, un percorso di rete…) non conta:
/// esisterebbe sempre o non si deve toccare.
pub fn present(cleaner: &Cleaner, env: &Env, reg_exists: &dyn Fn(&RegItem) -> bool) -> bool {
    let guard = Guard::new(env);
    cleaner.options.iter().flat_map(|o| &o.actions).any(|a| match a {
        Action::Files { path, .. } => expand(path, env).iter().any(|p| {
            let base: PathBuf = p
                .components()
                .take_while(|c| !has_wildcard(&c.as_os_str().to_string_lossy()))
                .collect();
            !base.as_os_str().is_empty()
                && guard.protected(&base).is_none()
                && std::fs::symlink_metadata(&base).is_ok()
        }),
        Action::RegKey { root, path } => reg_exists(&RegItem::Key {
            root: *root,
            path: path.clone(),
        }),
        Action::RegValue { root, path, value } => reg_exists(&RegItem::Value {
            root: *root,
            path: path.clone(),
            value: value.clone(),
        }),
    })
}

/// Esistenza vera di una chiave o di un valore di registro (sola lettura).
/// HKLM si guarda nella vista a 64 bit e, per `Software\…`, anche in
/// `Software\Wow6432Node\…` (la vista a 32 bit).
#[cfg(windows)]
pub fn reg_exists_real(item: &RegItem) -> bool {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};
    let (root, path, value) = match item {
        RegItem::Key { root, path } => (*root, path.as_str(), None),
        RegItem::Value { root, path, value } => (*root, path.as_str(), Some(value.as_str())),
    };
    let check = |hive, path: &str, flags| {
        RegKey::predef(hive)
            .open_subkey_with_flags(path, flags)
            .is_ok_and(|k| value.is_none_or(|v| k.get_raw_value(v).is_ok()))
    };
    match root {
        RegRoot::Hkcu => check(HKEY_CURRENT_USER, path, KEY_READ),
        RegRoot::Hklm => {
            check(HKEY_LOCAL_MACHINE, path, KEY_READ | KEY_WOW64_64KEY)
                || wow6432_path(path)
                    .is_some_and(|alt| check(HKEY_LOCAL_MACHINE, &alt, KEY_READ | KEY_WOW64_64KEY))
        }
    }
}

#[cfg(not(windows))]
pub fn reg_exists_real(_item: &RegItem) -> bool {
    false
}

/// `Software\X` → `Software\Wow6432Node\X`; `None` se non è sotto `Software`
/// o è già nella vista a 32 bit.
fn wow6432_path(path: &str) -> Option<String> {
    let (first, rest) = path.split_once('\\')?;
    let second = rest.split('\\').next().unwrap_or("");
    (first.eq_ignore_ascii_case("software") && !second.eq_ignore_ascii_case("wow6432node"))
        .then(|| format!(r"{first}\Wow6432Node\{rest}"))
}

/// `true` se due percorsi di regola (con `%Variabili%`, `*` e `?`) possono
/// indicare la stessa cosa o l'uno stare dentro l'altro: dopo l'espansione,
/// i componenti in comune (fino al più corto) si corrispondono uno a uno, con
/// i jolly di ciascuno dei due. Maiuscole, `/` e `\` finali ignorati. Un
/// percorso con una variabile non definita non si sovrappone a nulla.
pub fn overlaps(a: &str, b: &str, env: &Env) -> bool {
    let a: Vec<Vec<String>> = expand(a, env).iter().map(|p| key_comps(p)).collect();
    let b: Vec<Vec<String>> = expand(b, env).iter().map(|p| key_comps(p)).collect();
    a.iter().any(|x| {
        b.iter().any(|y| {
            !x.is_empty()
                && !y.is_empty()
                && x.iter()
                    .zip(y)
                    .all(|(p, q)| wildcard_match(p, q) || wildcard_match(q, p))
        })
    })
}

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

struct Ctx<'a> {
    env: &'a Env,
    /// Cartelle protette calcolate una volta per raccolta.
    guard: Guard,
    excludes: Vec<Exclude>,
    out: Collected,
    /// Chiavi dei file già elencati (azioni che si sovrappongono).
    seen: HashSet<String>,
    ctl: Option<&'a CollectCtl<'a>>,
    /// Voci di cartella esaminate (per `CollectCtl::tick`).
    examined: u64,
}

struct Exclude {
    comps: Vec<String>,
    patterns: Option<Vec<String>>,
}

/// Come scorrere una cartella.
struct Spec<'a> {
    patterns: &'a [String],
    recurse: bool,
    /// Elenca le sottocartelle svuotate.
    want_dirs: bool,
    /// Le sottocartelle del primo livello che corrispondono si prendono intere
    /// (`walk.top`).
    top_dirs_whole: bool,
}

impl Ctx<'_> {
    /// Percorsi protetti: file, cartelle di base e percorso della regola.
    fn prot(&self, path: &Path) -> Option<&'static str> {
        self.guard.protected(path)
    }

    /// Cartelle che si toglierebbero esse stesse (`dirs_to_remove`).
    fn prot_dir(&self, path: &Path) -> Option<&'static str> {
        self.guard.protected_dir_removal(path)
    }

    /// Mette `dir` fra le cartelle da togliere, se si può togliere essa stessa
    /// (una cartella temporanea si svuota ma resta). `true` se messa.
    fn push_dir(&mut self, dir: PathBuf) -> bool {
        if self.prot_dir(&dir).is_some() {
            return false;
        }
        self.out.dirs_to_remove.push(dir);
        true
    }

    fn cancelled(&self) -> bool {
        self.ctl.is_some_and(|c| c.cancel.load(Ordering::Relaxed))
    }

    /// Una voce di cartella in più esaminata.
    fn step(&mut self) {
        self.examined += 1;
        if self.examined.is_multiple_of(TICK_EVERY)
            && let Some(ctl) = self.ctl
        {
            (ctl.tick)(self.examined);
        }
    }

    /// Conta uno scarto e ne tiene il motivo (al più `MAX_REASONS`).
    fn skip(&mut self, reason: impl FnOnce() -> String) {
        self.out.skipped += 1;
        if self.out.skipped_reasons.len() < MAX_REASONS {
            self.out.skipped_reasons.push(reason());
        }
    }

    fn skip_protected(&mut self, path: &Path, why: &str) {
        self.skip(|| format!("percorso protetto: {} ({why})", path.display()));
    }

    fn files_action(
        &mut self,
        path: &str,
        patterns: &[String],
        recurse: bool,
        remove_self: bool,
        search: Search,
    ) {
        let patterns = split_patterns(patterns);
        for expanded in expand(path, self.env) {
            // Il percorso della regola, jolly compresi, prima di risolverlo:
            // `%UserProfile%\*` si rifiuta qui, non dopo essere diventato
            // `source`, `Documents`… (i controlli sui percorsi concreti restano).
            if let Some(why) = self.prot(&expanded) {
                self.skip_protected(&expanded, why);
                continue;
            }
            let bases = match resolve_glob(&expanded, self.env) {
                Ok(b) => b,
                Err(()) => {
                    self.skip(|| format!("collegamento non seguito: {}", expanded.display()));
                    continue;
                }
            };
            for base in bases {
                if self.cancelled() {
                    return;
                }
                self.one_base(&base, &patterns, recurse, remove_self, search);
            }
        }
    }

    fn one_base(
        &mut self,
        base: &Path,
        patterns: &[String],
        recurse: bool,
        remove_self: bool,
        search: Search,
    ) {
        if let Some(why) = self.prot(base) {
            self.skip_protected(base, why);
            return;
        }
        let Ok(md) = std::fs::symlink_metadata(base) else {
            return; // non esiste: niente da fare
        };
        if chain_has_link(base, self.env) {
            // La cartella o un suo antenato è un collegamento: non si segue.
            self.skip(|| format!("collegamento non seguito: {}", base.display()));
            return;
        }
        let name = base
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !md.is_dir() {
            // Un file: per `file` e `glob` basta la corrispondenza del percorso;
            // per gli altri il nome deve soddisfare i modelli.
            let by_path = matches!(search, Search::File | Search::Glob);
            if by_path || name_matches(patterns, &name) {
                self.add_file(base, md.len());
            }
            return;
        }
        if matches!(search, Search::File | Search::Glob) {
            // Una cartella indicata dal percorso: si prende intera.
            let spec = Spec {
                patterns: &[],
                recurse: true,
                want_dirs: true,
                top_dirs_whole: false,
            };
            if self.walk(base, &spec) {
                self.push_dir(base.to_path_buf());
            }
            return;
        }
        if self.dir_excluded(base) {
            return;
        }
        let spec = match search {
            Search::WalkAll => Spec {
                patterns,
                recurse: true,
                want_dirs: true,
                top_dirs_whole: false,
            },
            Search::WalkFiles => Spec {
                patterns,
                recurse: true,
                want_dirs: false,
                top_dirs_whole: false,
            },
            Search::WalkTop => Spec {
                patterns,
                recurse: false,
                want_dirs: false,
                top_dirs_whole: true,
            },
            _ => Spec {
                patterns,
                recurse,
                want_dirs: remove_self && recurse,
                top_dirs_whole: false,
            },
        };
        let all = self.walk(base, &spec);
        if remove_self && all {
            self.push_dir(base.to_path_buf());
        }
    }

    /// Scorre `dir` senza mai seguire collegamenti. `true` se tutto ciò che
    /// contiene è stato elencato per la rimozione (la cartella resterebbe vuota).
    fn walk(&mut self, dir: &Path, spec: &Spec) -> bool {
        let Ok(rd) = std::fs::read_dir(dir) else {
            self.skip(|| format!("cartella illeggibile: {}", dir.display()));
            return false;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        let mut all = true;
        for e in entries {
            if self.cancelled() {
                return false;
            }
            self.step();
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            // Su Windows i metadati della voce vengono dall'elenco della cartella
            // e non seguono i collegamenti (come `symlink_metadata`).
            let Ok(md) = e.metadata() else {
                all = false;
                continue;
            };
            if is_link(&md) {
                // Un collegamento vale come un file di 0 byte e si toglie come
                // collegamento: il suo contenuto non si tocca.
                if !(name_matches(spec.patterns, &name) && self.add_file(&path, 0)) {
                    all = false;
                }
            } else if md.is_dir() {
                if let Some(why) = self.prot(&path) {
                    self.skip_protected(&path, why);
                    all = false;
                } else if self.dir_excluded(&path) {
                    all = false;
                } else if spec.top_dirs_whole {
                    let whole = Spec {
                        patterns: &[],
                        recurse: true,
                        want_dirs: true,
                        top_dirs_whole: false,
                    };
                    if !(name_matches(spec.patterns, &name)
                        && self.walk(&path, &whole)
                        && self.push_dir(path))
                    {
                        all = false;
                    }
                } else if spec.recurse {
                    let sub_all = self.walk(&path, spec);
                    if !(sub_all && spec.want_dirs && self.push_dir(path)) {
                        all = false;
                    }
                } else {
                    all = false;
                }
            } else if !(name_matches(spec.patterns, &name) && self.add_file(&path, md.len())) {
                all = false;
            }
        }
        all
    }

    /// Elenca un file (o un collegamento) se il percorso finale non è protetto
    /// né escluso. `true` se è nella lista (anche se già c'era).
    fn add_file(&mut self, path: &Path, size: u64) -> bool {
        if let Some(why) = self.prot(path) {
            self.skip_protected(path, why);
            return false;
        }
        if self.excluded(path) {
            return false;
        }
        let key = key_comps(path).join("\\");
        if !self.seen.insert(key) {
            return true;
        }
        self.out.files.push((path.to_path_buf(), size));
        self.out.bytes += size;
        true
    }

    fn excluded(&self, path: &Path) -> bool {
        let c = key_comps(path);
        let name = c.last().cloned().unwrap_or_default();
        self.excludes.iter().any(|ex| match &ex.patterns {
            None => under(&c, &ex.comps),
            Some(p) => {
                c.len() > ex.comps.len() && under(&c, &ex.comps) && p.iter().any(|p| name_match(p, &name))
            }
        })
    }

    fn dir_excluded(&self, dir: &Path) -> bool {
        let c = key_comps(dir);
        self.excludes
            .iter()
            .any(|ex| ex.patterns.is_none() && under(&c, &ex.comps))
    }

    fn reg_item(&mut self, item: RegItem, reg_exists: &dyn Fn(&RegItem) -> bool) {
        let (root, path, value) = match &item {
            RegItem::Key { root, path } => (*root, path.as_str(), None),
            RegItem::Value { root, path, value } => (*root, path.as_str(), Some(value.as_str())),
        };
        if let Err(why) = reg_allowed(root, path, value) {
            self.skip(|| format!("chiave di registro protetta: {} ({why})", reg_display(&item)));
            return;
        }
        if !self.out.reg.contains(&item) && reg_exists(&item) {
            self.out.reg.push(item);
        }
    }
}

/// `path` coincide con `dir` o sta sotto; i componenti di `dir` possono avere
/// `*` e `?` (un componente per componente).
fn under(path: &[String], dir: &[String]) -> bool {
    path.len() >= dir.len() && dir.iter().zip(path).all(|(d, p)| wildcard_match(d, p))
}

/// `HKCU\Software\X|valore`, per i messaggi.
fn reg_display(item: &RegItem) -> String {
    let root = |r: &RegRoot| match r {
        RegRoot::Hkcu => "HKCU",
        RegRoot::Hklm => "HKLM",
    };
    match item {
        RegItem::Key { root: r, path } => format!("{}\\{path}", root(r)),
        RegItem::Value { root: r, path, value } => format!("{}\\{path}|{value}", root(r)),
    }
}

fn prepare_excludes<'a>(
    excludes: impl Iterator<Item = &'a ExcludeRule>,
    env: &Env,
) -> Vec<Exclude> {
    let mut out = Vec::new();
    for rule in excludes {
        let patterns = rule.pattern.as_ref().map(|p| {
            p.split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        });
        for path in expand(&rule.path, env) {
            let comps = key_comps(&path);
            if !comps.is_empty() {
                out.push(Exclude {
                    comps,
                    patterns: patterns.clone(),
                });
            }
        }
    }
    out
}

fn split_patterns(patterns: &[String]) -> Vec<String> {
    patterns
        .iter()
        .flat_map(|p| p.split(';'))
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// Nessun modello = tutto. `*.*` vale `*` (come in Windows).
fn name_matches(patterns: &[String], name: &str) -> bool {
    patterns.is_empty() || patterns.iter().any(|p| name_match(p, name))
}

fn name_match(pattern: &str, name: &str) -> bool {
    pattern == "*.*" || wildcard_match(pattern, name)
}

fn has_wildcard(s: &str) -> bool {
    s.contains(['*', '?'])
}

fn is_link(md: &Metadata) -> bool {
    md.file_type().is_symlink() || md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.is_dir() && !is_link(&m))
        .unwrap_or(false)
}

/// Componenti in minuscolo (senza `\\?\`, vuoti e `.`).
fn key_comps(path: &Path) -> Vec<String> {
    let s = path.to_string_lossy().replace('/', "\\");
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    s.split('\\')
        .filter(|c| !c.is_empty() && *c != ".")
        .map(str::to_lowercase)
        .collect()
}

/// `true` se un componente del percorso (la cartella stessa compresa) è un
/// collegamento o una giunzione. Si controllano solo i componenti sotto la più
/// lunga cartella nota dell'ambiente (`%AppData%`, `%UserProfile%`…), oppure,
/// se nessuna lo contiene, quelli sotto la radice del disco.
fn chain_has_link(path: &Path, env: &Env) -> bool {
    let target = key_comps(path);
    let trusted = ROOT_VARS
        .iter()
        .filter_map(|v| env.get(v))
        .map(|v| key_comps(Path::new(v)))
        .filter(|c| !c.is_empty() && target.starts_with(c))
        .map(|c| c.len())
        .max()
        .unwrap_or(1);
    let mut cur = PathBuf::new();
    let mut depth = 0usize;
    for comp in path.components() {
        cur.push(comp);
        if matches!(comp, Component::Prefix(_) | Component::RootDir | Component::CurDir) {
            continue;
        }
        depth += 1;
        if depth <= trusted.saturating_sub(1) {
            continue;
        }
        if let Ok(md) = std::fs::symlink_metadata(&cur)
            && is_link(&md)
        {
            return true;
        }
    }
    false
}

const ROOT_VARS: &[&str] = &[
    "windir",
    "systemroot",
    "programfiles",
    "programfiles(x86)",
    "programdata",
    "userprofile",
    "appdata",
    "localappdata",
    "systemdrive",
    "temp",
    "tmp",
];

/// Espande i caratteri jolly di un percorso (componente per componente, senza
/// mai attraversare un collegamento). `Err` se la parte letterale iniziale
/// passa da un collegamento.
fn resolve_glob(path: &Path, env: &Env) -> Result<Vec<PathBuf>, ()> {
    let comps: Vec<Component> = path.components().collect();
    let Some(first) = comps
        .iter()
        .position(|c| matches!(c, Component::Normal(s) if has_wildcard(&s.to_string_lossy())))
    else {
        return Ok(vec![path.to_path_buf()]);
    };
    let prefix: PathBuf = comps[..first].iter().collect();
    if prefix.as_os_str().is_empty() {
        return Ok(Vec::new());
    }
    if chain_has_link(&prefix, env) {
        return Err(());
    }
    let mut current = vec![prefix];
    for (i, comp) in comps.iter().enumerate().skip(first) {
        let last = i + 1 == comps.len();
        let name = comp.as_os_str().to_string_lossy().into_owned();
        let mut next = Vec::new();
        for dir in &current {
            if has_wildcard(&name) {
                if !is_real_dir(dir) {
                    continue;
                }
                let Ok(rd) = std::fs::read_dir(dir) else {
                    continue;
                };
                for e in rd.flatten() {
                    if wildcard_match(&name, &e.file_name().to_string_lossy()) {
                        let p = e.path();
                        if last || is_real_dir(&p) {
                            next.push(p);
                        }
                    }
                }
            } else {
                let p = dir.join(comp);
                let ok = if last {
                    std::fs::symlink_metadata(&p).is_ok()
                } else {
                    is_real_dir(&p)
                };
                if ok {
                    next.push(p);
                }
            }
        }
        current = next;
    }
    current.sort();
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Action, Origin, Search};
    use std::collections::HashMap;
    use std::fs;
    use std::path::Path;

    fn env(pairs: &[(&str, &str)]) -> Env {
        Env(pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<HashMap<_, _>>())
    }

    fn opt(actions: Vec<Action>) -> Opt {
        Opt {
            id: "o".into(),
            label: "o".into(),
            description: String::new(),
            warning: None,
            default_on: false,
            actions,
            excludes: vec![],
            unsupported: None,
        }
    }

    fn files_action(path: &Path, patterns: &[&str], recurse: bool, remove_self: bool) -> Action {
        Action::Files {
            path: path.to_string_lossy().into_owned(),
            patterns: patterns.iter().map(|s| s.to_string()).collect(),
            recurse,
            remove_self,
            search: Search::Winapp2,
        }
    }

    fn search_action(path: &Path, search: Search, recurse: bool) -> Action {
        Action::Files {
            path: path.to_string_lossy().into_owned(),
            patterns: vec![],
            recurse,
            remove_self: false,
            search,
        }
    }

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn never(_: &RegItem) -> bool {
        false
    }

    fn names(c: &Collected, root: &Path) -> Vec<String> {
        let mut v: Vec<String> = c
            .files
            .iter()
            .map(|(p, _)| {
                p.strip_prefix(root)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .replace('/', "\\")
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn collects_matching_files_recursively_with_sizes() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("cache");
        write(&base.join("a.tmp"), "12345");
        write(&base.join("sub").join("b.tmp"), "123");
        write(&base.join("sub").join("deep").join("c.tmp"), "1");
        let o = opt(vec![files_action(&base, &["*.*"], true, false)]);
        let c = collect(&o, &Env::default(), &never);
        assert_eq!(
            names(&c, &base),
            vec!["a.tmp", r"sub\b.tmp", r"sub\deep\c.tmp"]
        );
        assert_eq!(c.bytes, 9);
        assert_eq!(c.files.iter().map(|f| f.1).sum::<u64>(), 9);
        assert_eq!(c.skipped, 0);
        assert!(c.dirs_to_remove.is_empty());
        // senza RECURSE solo il livello superiore
        let o = opt(vec![files_action(&base, &["*"], false, false)]);
        let c = collect(&o, &Env::default(), &never);
        assert_eq!(names(&c, &base), vec!["a.tmp"]);
    }

    #[test]
    fn patterns_are_case_insensitive_and_semicolon_separated() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("logs");
        for n in ["A.LOG", "b.Tmp", "c.txt", "d.dat"] {
            write(&base.join(n), "x");
        }
        let o = opt(vec![files_action(&base, &["*.log;*.TMP"], false, false)]);
        let c = collect(&o, &Env::default(), &never);
        assert_eq!(names(&c, &base), vec!["A.LOG", "b.Tmp"]);
        let o = opt(vec![files_action(&base, &["*.log", "?.dat"], false, false)]);
        let c = collect(&o, &Env::default(), &never);
        assert_eq!(names(&c, &base), vec!["A.LOG", "d.dat"]);
    }

    #[test]
    fn exclude_key_keeps_the_named_file() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("cache");
        write(&base.join("a.tmp"), "1");
        write(&base.join("keep.tmp"), "1");
        write(&base.join("sub").join("b.log"), "1");
        write(&base.join("sub").join("c.tmp"), "1");
        let o = opt(vec![files_action(&base, &["*"], true, true)]);
        let ex = [ExcludeRule {
            path: base.join("KEEP.tmp").to_string_lossy().into_owned(),
            pattern: None,
        }];
        let c = collect_with_excludes(&o, &Env::default(), &ex, &never);
        assert_eq!(names(&c, &base), vec!["a.tmp", r"sub\b.log", r"sub\c.tmp"]);
        // la cartella non si toglie: contiene ancora un file escluso
        assert!(c.dirs_to_remove.contains(&base.join("sub")));
        assert!(!c.dirs_to_remove.contains(&base));
        // esclusione con modello
        let ex = [ExcludeRule {
            path: base.join("sub").to_string_lossy().into_owned(),
            pattern: Some("*.LOG;x.y".into()),
        }];
        let c = collect_with_excludes(&o, &Env::default(), &ex, &never);
        assert_eq!(names(&c, &base), vec!["a.tmp", "keep.tmp", r"sub\c.tmp"]);
        assert!(!c.dirs_to_remove.contains(&base.join("sub")));
        // esclusione di un'intera cartella
        let ex = [ExcludeRule {
            path: base.join("sub").to_string_lossy().into_owned(),
            pattern: None,
        }];
        let c = collect_with_excludes(&o, &Env::default(), &ex, &never);
        assert_eq!(names(&c, &base), vec!["a.tmp", "keep.tmp"]);
        assert!(c.dirs_to_remove.is_empty());
    }

    #[test]
    fn remove_self_lists_the_folder_after_files() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("cache");
        write(&base.join("a.tmp"), "1");
        write(&base.join("sub").join("b.tmp"), "1");
        let o = opt(vec![files_action(&base, &["*"], true, true)]);
        let c = collect(&o, &Env::default(), &never);
        assert_eq!(names(&c, &base), vec!["a.tmp", r"sub\b.tmp"]);
        // sottocartella prima, la cartella stessa per ultima
        assert_eq!(c.dirs_to_remove, vec![base.join("sub"), base.clone()]);
        // se qualcosa resta (modello che non copre tutto) la cartella non si elenca
        write(&base.join("keep.dat"), "1");
        let o = opt(vec![files_action(&base, &["*.tmp"], true, true)]);
        let c = collect(&o, &Env::default(), &never);
        assert!(!c.dirs_to_remove.contains(&base), "{:?}", c.dirs_to_remove);
        // senza REMOVESELF non si elenca nulla
        let o = opt(vec![files_action(&base, &["*"], true, false)]);
        assert!(collect(&o, &Env::default(), &never).dirs_to_remove.is_empty());
    }

    fn make_dir_link(link: &Path, target: &Path) -> bool {
        if std::os::windows::fs::symlink_dir(target, link).is_ok() {
            return true;
        }
        std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn junction_or_symlink_inside_is_not_followed() {
        let t = tempfile::tempdir().unwrap();
        let outside = t.path().join("outside");
        write(&outside.join("precious.txt"), "do not touch");
        write(&outside.join("nested").join("p2.txt"), "do not touch");
        let base = t.path().join("cache");
        write(&base.join("a.tmp"), "1");
        let link = base.join("link");
        if !make_dir_link(&link, &outside) {
            println!(
                "SKIP junction_or_symlink_inside_is_not_followed: nessun privilegio per creare collegamenti"
            );
            return;
        }
        let o = opt(vec![files_action(&base, &["*"], true, true)]);
        let c = collect(&o, &Env::default(), &never);
        assert_eq!(names(&c, &base), vec!["a.tmp", "link"]);
        let entry = c.files.iter().find(|(p, _)| p.ends_with("link")).unwrap();
        assert_eq!(entry.1, 0, "un collegamento vale 0 byte");
        assert!(c.files.iter().all(|(p, _)| !p.starts_with(&outside)));
        assert!(!c.dirs_to_remove.contains(&link));
        // il collegamento come base della regola non si segue
        let o = opt(vec![files_action(&link, &["*"], true, true)]);
        let c = collect(&o, &Env::default(), &never);
        assert!(c.files.is_empty() && c.dirs_to_remove.is_empty());
        assert_eq!(c.skipped, 1);
        // un collegamento nel mezzo del percorso nemmeno
        let o = opt(vec![files_action(&link.join("nested"), &["*"], true, false)]);
        let c = collect(&o, &Env::default(), &never);
        assert!(c.files.is_empty());
        assert_eq!(c.skipped, 1);
        assert!(outside.join("precious.txt").exists());
    }

    #[test]
    fn protected_expansion_collects_nothing_and_counts() {
        let t = tempfile::tempdir().unwrap();
        write(&t.path().join("doc.txt"), "mine");
        let e = env(&[("userprofile", &t.path().to_string_lossy())]);
        let o = opt(vec![Action::Files {
            path: "%UserProfile%".into(),
            patterns: vec!["*.*".into()],
            recurse: true,
            remove_self: false,
            search: Search::Winapp2,
        }]);
        let c = collect(&o, &e, &never);
        assert!(c.files.is_empty() && c.dirs_to_remove.is_empty());
        assert_eq!(c.skipped, 1);
        // anche `%AppData%` intera con REMOVESELF
        let e = env(&[("appdata", &t.path().to_string_lossy())]);
        let o = opt(vec![Action::Files {
            path: "%AppData%".into(),
            patterns: vec!["*".into()],
            recurse: true,
            remove_self: true,
            search: Search::Winapp2,
        }]);
        let c = collect(&o, &e, &never);
        assert!(c.files.is_empty() && c.dirs_to_remove.is_empty());
        assert_eq!(c.skipped, 1);
        // variabile non definita: si scarta senza contare
        let o = opt(vec![Action::Files {
            path: "%NOPE%\\x".into(),
            patterns: vec!["*".into()],
            recurse: true,
            remove_self: false,
            search: Search::Winapp2,
        }]);
        let c = collect(&o, &Env::default(), &never);
        assert!(c.files.is_empty());
        assert_eq!(c.skipped, 0);
    }

    #[test]
    fn protected_final_paths_are_skipped_and_counted() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("agent");
        write(&base.join("a.tmp"), "1");
        write(&base.join("auth.json"), "secret");
        write(&base.join(".credentials.json"), "secret");
        write(&base.join(".git").join("HEAD"), "ref");
        let o = opt(vec![files_action(&base, &["*"], true, true)]);
        let c = collect(&o, &Env::default(), &never);
        assert_eq!(names(&c, &base), vec!["a.tmp"]);
        assert_eq!(c.skipped, 3);
        assert!(c.dirs_to_remove.is_empty(), "{:?}", c.dirs_to_remove);
    }

    #[test]
    fn registry_items_are_filtered_by_existence() {
        let present = RegItem::Key {
            root: RegRoot::Hkcu,
            path: r"Software\Foo".into(),
        };
        let present_value = RegItem::Value {
            root: RegRoot::Hklm,
            path: r"Software\Bar".into(),
            value: "Mru".into(),
        };
        let o = opt(vec![
            Action::RegKey {
                root: RegRoot::Hkcu,
                path: r"Software\Foo".into(),
            },
            Action::RegKey {
                root: RegRoot::Hkcu,
                path: r"Software\Gone".into(),
            },
            Action::RegValue {
                root: RegRoot::Hklm,
                path: r"Software\Bar".into(),
                value: "Mru".into(),
            },
            // fuori da Software, o Software intero: mai, nemmeno se esiste
            Action::RegKey {
                root: RegRoot::Hklm,
                path: r"System\CurrentControlSet".into(),
            },
            Action::RegKey {
                root: RegRoot::Hkcu,
                path: "Software".into(),
            },
        ]);
        let exists = |i: &RegItem| match i {
            RegItem::Key { path, .. } => path != r"Software\Gone",
            RegItem::Value { .. } => true,
        };
        let c = collect(&o, &Env::default(), &exists);
        assert_eq!(c.reg, vec![present, present_value]);
        assert_eq!(c.skipped, 2);
        assert!(c.files.is_empty());
    }

    /// Le chiavi e i valori provati dalla revisione non si raccolgono, anche se
    /// esistono e arrivano da un `Opt` costruito a mano (CleanerML, prove).
    #[test]
    fn registry_probes_are_refused_at_collection() {
        let refused = [
            (RegRoot::Hklm, r"Software\Microsoft", None),
            (RegRoot::Hklm, r"Software\Microsoft\Windows NT\CurrentVersion", None),
            (RegRoot::Hklm, r"Software\Policies", None),
            (RegRoot::Hklm, r"Software\Classes\exefile", None),
            (RegRoot::Hklm, r"Software\Classes\*", None),
            (RegRoot::Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Run", None),
            (
                RegRoot::Hklm,
                r"Software\Microsoft\Windows NT\CurrentVersion\Winlogon",
                Some("Userinit"),
            ),
        ];
        let allowed = [
            (
                RegRoot::Hkcu,
                r"Software\Microsoft\Windows\CurrentVersion\Explorer\RecentDocs",
                None,
            ),
            (RegRoot::Hklm, r"Software\Classes\.mpf", None),
            (RegRoot::Hklm, r"Software\Microsoft\Tracing", None),
            (RegRoot::Hkcu, r"Software\Microsoft\Internet Explorer\TypedURLs", None),
        ];
        let action = |(root, path, value): &(RegRoot, &str, Option<&str>)| match value {
            Some(v) => Action::RegValue {
                root: *root,
                path: path.to_string(),
                value: v.to_string(),
            },
            None => Action::RegKey {
                root: *root,
                path: path.to_string(),
            },
        };
        let o = opt(refused.iter().chain(&allowed).map(action).collect());
        let c = collect(&o, &Env::default(), &|_| true);
        let got: Vec<&str> = c
            .reg
            .iter()
            .map(|i| match i {
                RegItem::Key { path, .. } | RegItem::Value { path, .. } => path.as_str(),
            })
            .collect();
        assert_eq!(got, allowed.iter().map(|a| a.1).collect::<Vec<_>>());
        assert_eq!(c.skipped, refused.len() as u64);
    }

    #[test]
    fn running_matches_exe_names_case_insensitively() {
        let cl = Cleaner {
            id: "x".into(),
            label: "x".into(),
            group: "g".into(),
            origin: Origin::Winapp2,
            description: String::new(),
            running: vec!["Code.exe".into(), "firefox".into(), "absent.exe".into()],
            options: vec![],
        };
        let procs: Vec<String> = ["explorer.exe", "CODE.EXE", "Firefox.exe"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(running(&cl, &procs), vec!["Code.exe", "firefox"]);
        assert!(running(&cl, &[]).is_empty());
    }

    #[test]
    fn glob_and_walk_searches() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        write(&root.join("p1").join("cache").join("a.tmp"), "1");
        write(&root.join("p2").join("cache").join("b.tmp"), "1");
        write(&root.join("p2").join("other").join("c.tmp"), "1");
        // Winapp2 con carattere jolly nel percorso
        let o = opt(vec![files_action(
            &root.join("p*").join("cache"),
            &["*"],
            true,
            false,
        )]);
        let c = collect(&o, &Env::default(), &never);
        assert_eq!(names(&c, root), vec![r"p1\cache\a.tmp", r"p2\cache\b.tmp"]);
        // glob (CleanerML): i file corrispondenti
        let o = opt(vec![search_action(
            &root.join("p2").join("cache").join("*.tmp"),
            Search::Glob,
            false,
        )]);
        assert_eq!(
            names(&collect(&o, &Env::default(), &never), root),
            vec![r"p2\cache\b.tmp"]
        );
        // file singolo
        let o = opt(vec![search_action(
            &root.join("p1").join("cache").join("a.tmp"),
            Search::File,
            false,
        )]);
        assert_eq!(
            names(&collect(&o, &Env::default(), &never), root),
            vec![r"p1\cache\a.tmp"]
        );
        // walk.all: file e cartelle vuotate
        let o = opt(vec![search_action(&root.join("p2"), Search::WalkAll, true)]);
        let c = collect(&o, &Env::default(), &never);
        assert_eq!(names(&c, root), vec![r"p2\cache\b.tmp", r"p2\other\c.tmp"]);
        assert_eq!(
            c.dirs_to_remove,
            vec![root.join("p2").join("cache"), root.join("p2").join("other")]
        );
        // walk.top: file del livello superiore e cartelle intere
        write(&root.join("p2").join("top.tmp"), "1");
        let o = opt(vec![search_action(&root.join("p2"), Search::WalkTop, false)]);
        let c = collect(&o, &Env::default(), &never);
        assert_eq!(
            names(&c, root),
            vec![r"p2\cache\b.tmp", r"p2\other\c.tmp", r"p2\top.tmp"]
        );
        // walk.files: niente cartelle
        let o = opt(vec![search_action(&root.join("p2"), Search::WalkFiles, true)]);
        let c = collect(&o, &Env::default(), &never);
        assert_eq!(c.files.len(), 3);
        assert!(c.dirs_to_remove.is_empty());
    }

    #[test]
    fn intermediate_and_suffix_wildcards_resolve_to_candidate_bases() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let e = env(&[("localappdata", &root.to_string_lossy())]);
        // due cartelle di versione sorelle, solo una ha `caches`
        write(&root.join("JetBrains").join("IDEA2025").join("caches").join("a.idx"), "12");
        write(&root.join("JetBrains").join("IDEA2025").join("keep.xml"), "1");
        write(&root.join("JetBrains").join("PyCharm2024").join("other").join("b.idx"), "1");
        let walk_files = |path: &str, search: Search| Action::Files {
            path: path.into(),
            patterns: vec![],
            recurse: true,
            remove_self: false,
            search,
        };
        let o = opt(vec![walk_files(
            r"%LocalAppData%\JetBrains\*\caches",
            Search::WalkFiles,
        )]);
        let c = collect(&o, &e, &never);
        assert_eq!(names(&c, root), vec![r"JetBrains\IDEA2025\caches\a.idx"]);
        assert_eq!(c.bytes, 2);
        // carattere jolly come suffisso di un segmento
        write(&root.join("Google").join("AndroidStudio2024.1").join("caches").join("c.bin"), "1");
        write(&root.join("Google").join("AndroidStudio2025.1").join("caches").join("d.bin"), "1");
        write(&root.join("Google").join("Chrome").join("caches").join("e.bin"), "1");
        let o = opt(vec![walk_files(
            r"%LocalAppData%\Google\AndroidStudio*\caches",
            Search::WalkFiles,
        )]);
        let c = collect(&o, &e, &never);
        assert_eq!(
            names(&c, root),
            vec![
                r"Google\AndroidStudio2024.1\caches\c.bin",
                r"Google\AndroidStudio2025.1\caches\d.bin"
            ]
        );
        // ogni candidato passa da `protected`: un jolly che porta a `.git` non raccoglie
        write(&root.join("w1").join(".git").join("HEAD"), "x");
        let o = opt(vec![walk_files(r"%LocalAppData%\w*\.git", Search::WalkFiles)]);
        let c = collect(&o, &e, &never);
        assert!(c.files.is_empty());
        assert_eq!(c.skipped, 1);
        // modelli già tradotti da regex: `*.jsonl` e `*.jsonl.*`
        write(&root.join("agent").join("s1").join("x.jsonl"), "1");
        write(&root.join("agent").join("s1").join("x.jsonl.bak"), "1");
        write(&root.join("agent").join("s1").join("x.txt"), "1");
        let o = opt(vec![Action::Files {
            path: r"%LocalAppData%\agent\*".into(),
            patterns: vec!["*.jsonl".into(), "*.jsonl.*".into()],
            recurse: true,
            remove_self: false,
            search: Search::WalkFiles,
        }]);
        let c = collect(&o, &e, &never);
        assert_eq!(
            names(&c, root),
            vec![r"agent\s1\x.jsonl", r"agent\s1\x.jsonl.bak"]
        );
    }

    #[test]
    fn overlapping_actions_do_not_double_count() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("c");
        write(&base.join("a.tmp"), "1234");
        let a = files_action(&base, &["*"], true, false);
        let o = opt(vec![a.clone(), a]);
        let c = collect(&o, &Env::default(), &never);
        assert_eq!(c.files.len(), 1);
        assert_eq!(c.bytes, 4);
    }

    /// I1: `%UserProfile%\*` con RECURSE|REMOVESELF si rifiuta sul percorso
    /// della regola, prima di risolvere il jolly in `source`, `Documents`…
    #[test]
    fn wildcard_rule_under_a_protected_folder_is_refused_before_resolving() {
        let t = tempfile::tempdir().unwrap();
        let profile = t.path().join("u");
        write(&profile.join("source").join("repo").join("main.rs"), "fn main() {}");
        let e = env(&[("userprofile", &profile.to_string_lossy())]);
        let o = opt(vec![Action::Files {
            path: r"%UserProfile%\*".into(),
            patterns: vec!["*.*".into()],
            recurse: true,
            remove_self: true,
            search: Search::Winapp2,
        }]);
        let c = collect(&o, &e, &never);
        assert!(c.files.is_empty(), "{:?}", c.files);
        assert!(c.dirs_to_remove.is_empty(), "{:?}", c.dirs_to_remove);
        assert_eq!(c.skipped, 1);
        assert!(
            c.skipped_reasons.iter().any(|r| r.starts_with("percorso protetto: ")),
            "{:?}",
            c.skipped_reasons
        );
        // Lo stesso da un cleaner CleanerML (walk.all e glob).
        let xml = r#"<cleaner id="t" os="windows"><label>t</label>
            <option id="o"><label>o</label><description>d</description>
            <action command="delete" search="walk.all" path="%UserProfile%\*"/>
            <action command="delete" search="glob" path="%UserProfile%\*"/>
            </option></cleaner>"#;
        let cl = crate::cleanerml::parse_cleaner(xml, "t", &e).unwrap();
        let c = collect(&cl.options[0], &e, &never);
        assert!(c.files.is_empty() && c.dirs_to_remove.is_empty(), "{c:?}");
        assert_eq!(c.skipped, 2);
        assert!(profile.join("source").join("repo").join("main.rs").exists());
    }

    /// I2: esclusioni con jolly nel percorso (`ASUS\*\`, `DOMStore\*\`,
    /// `*Santa* Rampage`) si confrontano componente per componente.
    #[test]
    fn exclusions_with_wildcards_in_the_path_keep_their_files() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("ASUS");
        write(&base.join("app1").join("install.log"), "keep");
        write(&base.join("app1").join("other.log"), "x");
        write(&base.join("app2").join("sub").join("INSTALL.LOG"), "keep");
        write(&base.join("DOMStore").join("abc").join("d.log"), "keep");
        write(&base.join("The Santa Claus Rampage").join("s.log"), "keep");
        let o = opt(vec![files_action(&base, &["*.log"], true, false)]);
        let ex = [
            ExcludeRule {
                path: format!(r"{}\*\", base.display()),
                pattern: Some("install.log".into()),
            },
            ExcludeRule {
                path: format!(r"{}\DOMStore\*\", base.display()),
                pattern: None,
            },
            ExcludeRule {
                path: format!(r"{}\*Santa* Rampage", base.display()),
                pattern: None,
            },
        ];
        let c = collect_with_excludes(&o, &Env::default(), &ex, &never);
        assert_eq!(names(&c, &base), vec![r"app1\other.log"]);
    }

    /// I5: 20 000 file si raccolgono in tempo lineare.
    #[test]
    fn twenty_thousand_files_collect_quickly() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("many");
        for d in 0..200 {
            let dir = base.join(format!("d{d:03}"));
            fs::create_dir_all(&dir).unwrap();
            for f in 0..100 {
                fs::write(dir.join(format!("f{f:03}.tmp")), b"").unwrap();
            }
        }
        let o = opt(vec![files_action(&base, &["*"], true, true)]);
        let t0 = std::time::Instant::now();
        let c = collect(&o, &Env::default(), &never);
        let took = t0.elapsed();
        assert_eq!(c.files.len(), 20_000);
        eprintln!("raccolta di 20 000 file: {took:?}");
        let limit = if cfg!(debug_assertions) { 3.0 } else { 1.0 };
        assert!(took.as_secs_f64() < limit, "raccolta di 20 000 file in {took:?}");
    }

    #[test]
    fn collect_ctl_reports_progress_and_stops_when_cancelled() {
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicBool, Ordering};
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("c");
        for i in 0..600 {
            write(&base.join(format!("s{}", i % 3)).join(format!("{i}.tmp")), "1");
        }
        let o = opt(vec![files_action(&base, &["*"], true, true)]);
        let cancel = AtomicBool::new(false);
        let seen = Mutex::new(Vec::new());
        let tick = |n: u64| seen.lock().unwrap().push(n);
        let c = collect_ctl(&o, &Env::default(), &[], &never, &CollectCtl { cancel: &cancel, tick: &tick });
        assert_eq!(c.files.len(), 600);
        assert!(!c.cancelled);
        let ticks = seen.lock().unwrap().clone();
        assert!(ticks.len() >= 2, "{ticks:?}");
        assert!(ticks.windows(2).all(|w| w[0] <= w[1]), "{ticks:?}");
        assert_eq!(*ticks.last().unwrap(), 603, "voci esaminate: 600 file e 3 cartelle");
        // Annullata: si ferma, non elenca cartelle da togliere e lo dice.
        cancel.store(true, Ordering::SeqCst);
        let c = collect_ctl(&o, &Env::default(), &[], &never, &CollectCtl { cancel: &cancel, tick: &tick });
        assert!(c.cancelled);
        assert!(c.files.is_empty() && c.dirs_to_remove.is_empty(), "{c:?}");
    }

    #[test]
    fn skipped_reasons_are_italian_and_capped_at_twenty() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("agent");
        for i in 0..30 {
            write(&base.join(format!("p{i:02}")).join("auth.json"), "secret");
        }
        let o = opt(vec![files_action(&base, &["*"], true, false)]);
        let c = collect(&o, &Env::default(), &never);
        assert!(c.files.is_empty());
        assert_eq!(c.skipped, 30);
        assert_eq!(c.skipped_reasons.len(), 20);
        assert!(
            c.skipped_reasons[0].starts_with("percorso protetto: ")
                && c.skipped_reasons[0].contains("auth.json")
                && c.skipped_reasons[0].contains("file di credenziali"),
            "{:?}",
            c.skipped_reasons[0]
        );
    }

    #[test]
    fn link_in_the_path_is_explained_in_the_reasons() {
        let t = tempfile::tempdir().unwrap();
        let outside = t.path().join("outside");
        write(&outside.join("x.tmp"), "1");
        let link = t.path().join("link");
        if !make_dir_link(&link, &outside) {
            println!("SKIP link_in_the_path_is_explained_in_the_reasons: nessun collegamento");
            return;
        }
        let o = opt(vec![files_action(&link, &["*"], true, false)]);
        let c = collect(&o, &Env::default(), &never);
        assert!(c.files.is_empty());
        assert!(
            c.skipped_reasons.iter().any(|r| r.starts_with("collegamento non seguito: ")),
            "{:?}",
            c.skipped_reasons
        );
    }

    fn cleaner(actions: Vec<Action>) -> Cleaner {
        Cleaner {
            id: "c".into(),
            label: "c".into(),
            group: "g".into(),
            origin: Origin::CleanerMl("c"),
            description: String::new(),
            running: vec![],
            options: vec![opt(actions)],
        }
    }

    #[test]
    fn present_checks_base_folders_and_registry() {
        let t = tempfile::tempdir().unwrap();
        let profile = t.path().join("u");
        fs::create_dir_all(profile.join(".tool").join("cache")).unwrap();
        let e = env(&[("userprofile", &profile.to_string_lossy())]);
        let files = |p: &str, search| Action::Files {
            path: p.into(),
            patterns: vec![],
            recurse: true,
            remove_self: false,
            search,
        };
        // La cartella c'è (anche con un jolly dopo la base).
        assert!(present(&cleaner(vec![files(r"%UserProfile%\.tool\cache", Search::WalkAll)]), &e, &never));
        assert!(present(&cleaner(vec![files(r"%UserProfile%\.tool\*.log", Search::Glob)]), &e, &never));
        // Non c'è, o la base sarebbe il profilo intero (sempre presente).
        assert!(!present(&cleaner(vec![files(r"%UserProfile%\.other\cache", Search::WalkAll)]), &e, &never));
        assert!(!present(&cleaner(vec![files(r"%UserProfile%\*\cache", Search::WalkAll)]), &e, &never));
        assert!(!present(&cleaner(vec![files(r"%Nope%\x", Search::WalkAll)]), &e, &never));
        // Registro: decide `reg_exists`.
        let key = Action::RegKey {
            root: RegRoot::Hkcu,
            path: r"Software\Tool".into(),
        };
        assert!(present(&cleaner(vec![key.clone()]), &e, &|_| true));
        assert!(!present(&cleaner(vec![key]), &e, &never));
    }

    #[test]
    fn overlaps_is_wildcard_aware_and_two_way() {
        let e = env(&[("localappdata", r"C:\Users\u\AppData\Local")]);
        assert!(overlaps(r"%LocalAppData%\Google\Chrome", r"C:\USERS\u\appdata\local\google", &e));
        assert!(overlaps(r"C:\Users\u\AppData\Local\Google", r"%LocalAppData%\Google\Chrome\Cache", &e));
        assert!(overlaps(r"%LocalAppData%\Google\Chrome*\Cache", r"%LocalAppData%\Google\Chrome Beta", &e));
        assert!(overlaps(r"%LocalAppData%\*\Cache", r"%LocalAppData%\Google\Cache\x", &e));
        assert!(overlaps(r"%LocalAppData%\Google\", r"%LocalAppData%/google", &e));
        assert!(!overlaps(r"%LocalAppData%\Google\Chrome", r"%LocalAppData%\Google\Edge", &e));
        assert!(!overlaps(r"%LocalAppData%\Mozilla", r"%LocalAppData%\Google", &e));
        assert!(!overlaps(r"%Nope%\x", r"%LocalAppData%", &e));
    }

    /// Sola lettura del registro vero: chiavi che esistono su ogni Windows.
    #[cfg(windows)]
    #[test]
    fn reg_exists_real_reads_keys_and_values() {
        assert!(reg_exists_real(&RegItem::Key {
            root: RegRoot::Hklm,
            path: r"Software\Microsoft\Windows NT\CurrentVersion".into(),
        }));
        assert!(reg_exists_real(&RegItem::Value {
            root: RegRoot::Hklm,
            path: r"Software\Microsoft\Windows NT\CurrentVersion".into(),
            value: "ProductName".into(),
        }));
        assert!(!reg_exists_real(&RegItem::Value {
            root: RegRoot::Hklm,
            path: r"Software\Microsoft\Windows NT\CurrentVersion".into(),
            value: "pc-rules-no-such-value".into(),
        }));
        assert!(!reg_exists_real(&RegItem::Key {
            root: RegRoot::Hkcu,
            path: r"Software\pc-rules-no-such-key".into(),
        }));
        assert_eq!(wow6432_path(r"Software\A\B").as_deref(), Some(r"Software\Wow6432Node\A\B"));
        assert_eq!(wow6432_path(r"Software\Wow6432Node\A"), None);
    }

    /// N3: `%AppData%\Microsoft*` non tocca mai `%AppData%\Microsoft`; le
    /// cartelle concrete non protette di un jolly con prefisso sì.
    #[test]
    fn partial_wildcard_touches_only_unprotected_concrete_folders() {
        let t = tempfile::tempdir().unwrap();
        let appdata = t.path().join("Roaming");
        write(&appdata.join("Microsoft").join("a.log"), "keep");
        write(&appdata.join("Microsoftx").join("b.log"), "x");
        write(&appdata.join("Discord").join("c.log"), "x");
        write(&appdata.join("DiscordPTB").join("d.log"), "x");
        let e = env(&[("appdata", &appdata.to_string_lossy())]);
        let rule = |path: &str| Action::Files {
            path: path.into(),
            patterns: vec!["*".into()],
            recurse: true,
            remove_self: false,
            search: Search::Winapp2,
        };
        let c = collect(&opt(vec![rule(r"%AppData%\Microsoft*")]), &e, &never);
        assert!(c.files.iter().all(|(p, _)| !p.starts_with(appdata.join("Microsoft"))), "{c:?}");
        let c = collect(&opt(vec![rule(r"%AppData%\Discord*")]), &e, &never);
        assert_eq!(names(&c, &appdata), vec![r"DiscordPTB\d.log", r"Discord\c.log"]);
        assert!(appdata.join("Microsoft").join("a.log").exists());
    }
}
