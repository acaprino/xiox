//! Raccolta di file e chiavi di registro da una regola.

use std::fs::Metadata;
use std::os::windows::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use crate::expand::expand;
use crate::guard::protected;
use crate::model::{Action, Cleaner, Env, Opt, RegRoot, Search};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Collected {
    pub files: Vec<(PathBuf, u64)>,
    pub dirs_to_remove: Vec<PathBuf>,
    pub reg: Vec<RegItem>,
    pub bytes: u64,
    pub skipped: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegItem {
    Key { root: RegRoot, path: String },
    Value { root: RegRoot, path: String, value: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcludeRule {
    pub path: String,
    pub pattern: Option<String>,
}

pub fn collect(opt: &Opt, env: &Env, reg_exists: &dyn Fn(&RegItem) -> bool) -> Collected {
    collect_with_excludes(opt, env, &[], reg_exists)
}

/// Come `collect`, con in più i file che la regola (Winapp2 `ExcludeKey`) deve
/// lasciare: `pattern: None` esclude il percorso (file o cartella) e tutto ciò
/// che contiene; con un modello esclude, in quella cartella e sotto, i file
/// i cui nomi lo soddisfano (`;` separa più modelli).
pub fn collect_with_excludes(
    opt: &Opt,
    env: &Env,
    excludes: &[ExcludeRule],
    reg_exists: &dyn Fn(&RegItem) -> bool,
) -> Collected {
    let mut ctx = Ctx {
        env,
        excludes: prepare_excludes(excludes, env),
        out: Collected::default(),
        seen: Vec::new(),
    };
    for action in &opt.actions {
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
                path,
                reg_exists,
            ),
            Action::RegValue { root, path, value } => ctx.reg_item(
                RegItem::Value {
                    root: *root,
                    path: path.clone(),
                    value: value.clone(),
                },
                path,
                reg_exists,
            ),
        }
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

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

struct Ctx<'a> {
    env: &'a Env,
    excludes: Vec<Exclude>,
    out: Collected,
    /// Chiavi dei file già elencati (azioni che si sovrappongono).
    seen: Vec<String>,
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
            let bases = match resolve_glob(&expanded, self.env) {
                Ok(b) => b,
                Err(()) => {
                    self.out.skipped += 1;
                    continue;
                }
            };
            for base in bases {
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
        if protected(base, self.env).is_some() {
            self.out.skipped += 1;
            return;
        }
        let Ok(md) = std::fs::symlink_metadata(base) else {
            return; // non esiste: niente da fare
        };
        if chain_has_link(base, self.env) {
            // La cartella o un suo antenato è un collegamento: non si segue.
            self.out.skipped += 1;
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
            let all = self.walk(base, &spec);
            if all {
                self.out.dirs_to_remove.push(base.to_path_buf());
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
            self.out.dirs_to_remove.push(base.to_path_buf());
        }
    }

    /// Scorre `dir` senza mai seguire collegamenti. `true` se tutto ciò che
    /// contiene è stato elencato per la rimozione (la cartella resterebbe vuota).
    fn walk(&mut self, dir: &Path, spec: &Spec) -> bool {
        let Ok(rd) = std::fs::read_dir(dir) else {
            self.out.skipped += 1;
            return false;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        let mut all = true;
        for e in entries {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(md) = std::fs::symlink_metadata(&path) else {
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
                if protected(&path, self.env).is_some() {
                    self.out.skipped += 1;
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
                    if name_matches(spec.patterns, &name) && self.walk(&path, &whole) {
                        self.out.dirs_to_remove.push(path);
                    } else {
                        all = false;
                    }
                } else if spec.recurse {
                    let sub_all = self.walk(&path, spec);
                    if sub_all && spec.want_dirs {
                        self.out.dirs_to_remove.push(path);
                    } else {
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
        if protected(path, self.env).is_some() {
            self.out.skipped += 1;
            return false;
        }
        if self.excluded(path) {
            return false;
        }
        let key = key_comps(path).join("\\");
        if self.seen.contains(&key) {
            return true;
        }
        self.seen.push(key);
        self.out.files.push((path.to_path_buf(), size));
        self.out.bytes += size;
        true
    }

    fn excluded(&self, path: &Path) -> bool {
        let c = key_comps(path);
        let name = c.last().cloned().unwrap_or_default();
        self.excludes.iter().any(|ex| match &ex.patterns {
            None => c.starts_with(&ex.comps),
            Some(p) => {
                c.len() > ex.comps.len()
                    && c.starts_with(&ex.comps)
                    && p.iter().any(|p| name_match(p, &name))
            }
        })
    }

    fn dir_excluded(&self, dir: &Path) -> bool {
        let c = key_comps(dir);
        self.excludes
            .iter()
            .any(|ex| ex.patterns.is_none() && c.starts_with(&ex.comps))
    }

    fn reg_item(&mut self, item: RegItem, path: &str, reg_exists: &dyn Fn(&RegItem) -> bool) {
        if !registry_path_allowed(path) {
            self.out.skipped += 1;
            return;
        }
        if reg_exists(&item) && !self.out.reg.contains(&item) {
            self.out.reg.push(item);
        }
    }
}

/// Solo sotto `Software` (HKCU e HKLM, `Classes` compreso), mai `Software` né
/// `Software\Classes` interi, mai `..`.
fn registry_path_allowed(path: &str) -> bool {
    let comps: Vec<String> = path
        .replace('/', "\\")
        .split('\\')
        .filter(|c| !c.is_empty())
        .map(str::to_lowercase)
        .collect();
    comps.first().map(String::as_str) == Some("software")
        && !comps.iter().any(|c| c == "..")
        && comps.len() >= 2
        && !(comps.len() == 2 && comps[1] == "classes")
}

fn prepare_excludes(excludes: &[ExcludeRule], env: &Env) -> Vec<Exclude> {
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
    pattern == "*.*" || wildcard(pattern, name)
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
                    if wildcard(&name, &e.file_name().to_string_lossy()) {
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

/// Case-insensitive match with `*` and `?` (stessa regola di
/// `cleaner::wildcard` dell'app: il crate non dipende dall'app).
fn wildcard(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni, mut star, mut mark) = (0usize, 0usize, usize::MAX, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            mark = ni;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
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
}
