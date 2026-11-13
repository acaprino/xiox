//! Lettura dei file CleanerML (cleaner descritti in XML: opzioni e azioni).
//!
//! Significato dei `Search` nelle azioni prodotte (lo applica `collect`):
//! - `File`: il file indicato; `Glob`: i file che corrispondono al modello nel
//!   percorso (anche con `*` e `?` nei segmenti); nessuna ricorsione.
//! - `WalkAll`: si tolgono i figli della cartella, **file e sottocartelle**
//!   (ricorsivo); la cartella radice resta (`remove_self` è sempre `false`).
//! - `WalkFiles`: solo i **file** sotto la cartella (ricorsivo), le cartelle
//!   restano.
//! - `WalkTop`: solo i figli diretti della cartella, **non ricorsivo**.
//!
//! `patterns` (da `regex`) filtra i nomi dei file; vuoto = tutti.
//!
//! Regola di sicurezza: un filtro che non si capisce non allarga mai l'azione.
//! Ogni attributo di `<action>` fuori da `command`, `search`, `path`, `regex`,
//! `nregex`, `type`, `os`, `name` rende l'azione non supportata; `nregex`,
//! `wholeregex` e `nwholeregex` sono riconosciuti ma non supportati. L'azione si
//! scarta e l'opzione compare con `unsupported`.

use std::collections::HashMap;
use std::iter::Peekable;
use std::str::Chars;

use roxmltree::{Document, Node};

use crate::expand::expand;
use crate::model::{Action, Cleaner, Env, Opt, Origin, RegRoot, Search};

/// Gruppo con cui compaiono i cleaner CleanerML nell'elenco.
const GROUP: &str = "Sviluppo e AI";

/// Legge un cleaner CleanerML. Tutte le opzioni sono spente di default; ciò che
/// il motore non sa fare (altri comandi, `search` non elencati, regex non
/// traducibili) non si scarta: l'opzione resta con `unsupported`. I percorsi
/// restano con le `%Variabili%` (le espande `collect`); di un `<var>` si tengono
/// solo i valori le cui variabili d'ambiente sono definite in `env`.
pub fn parse_cleaner(xml: &str, origin: &'static str, env: &Env) -> Result<Cleaner, String> {
    let doc = Document::parse(xml.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("XML non valido: {e}"))?;
    let root = doc.root_element();
    if !root.has_tag_name("cleaner") {
        return Err("manca l'elemento <cleaner>".into());
    }
    if !os_ok(root.attribute("os")) {
        return Err("os non windows".into());
    }
    let id = root
        .attribute("id")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("il cleaner non ha l'attributo id")?
        .to_string();

    let vars = read_vars(root, env);
    // Una guardia di sicurezza non sparisce in silenzio: un `<running>` di un tipo
    // che non si gestisce (es. `pathname`) rende non supportate tutte le opzioni.
    let bad_running: Option<String> = root
        .descendants()
        .filter(|n| n.has_tag_name("running") && os_ok(n.attribute("os")))
        .find_map(|n| match n.attribute("type").map(str::trim) {
            Some("exe") => None,
            other => Some(format!("tipo di processo «{}»", other.unwrap_or(""))),
        });
    let running = root
        .descendants()
        .filter(|n| n.has_tag_name("running"))
        .filter(|n| n.attribute("type").map(str::trim) == Some("exe") && os_ok(n.attribute("os")))
        .map(text)
        .filter(|t| !t.is_empty())
        .fold(Vec::new(), |mut acc, t| {
            if !acc.contains(&t) {
                acc.push(t);
            }
            acc
        });
    let options = root
        .children()
        .filter(|n| n.has_tag_name("option"))
        .filter_map(|n| parse_option(n, &vars, bad_running.as_deref()))
        .collect();

    Ok(Cleaner {
        label: child_text(root, "label").unwrap_or_else(|| id.clone()),
        description: child_text(root, "description").unwrap_or_default(),
        group: GROUP.to_string(),
        origin: Origin::CleanerMl(origin),
        id,
        running,
        options,
    })
}

/// Senza `os` vale per Windows.
fn os_ok(os: Option<&str>) -> bool {
    os.is_none_or(|o| o.trim().eq_ignore_ascii_case("windows"))
}

fn text(node: Node) -> String {
    node.text().unwrap_or("").trim().to_string()
}

fn child_text(node: Node, tag: &str) -> Option<String> {
    node.children()
        .find(|c| c.has_tag_name(tag))
        .map(text)
        .filter(|t| !t.is_empty())
}

/// `<var id|name="x"><value>…</value>…</var>`: valori per Windows le cui
/// variabili d'ambiente esistono. Una variabile con tutti i valori scartati
/// resta nella mappa, vuota: le azioni che la usano spariscono.
fn read_vars(root: Node, env: &Env) -> HashMap<String, Vec<String>> {
    let mut vars: HashMap<String, Vec<String>> = HashMap::new();
    for var in root.descendants().filter(|n| n.has_tag_name("var")) {
        let Some(name) = var
            .attribute("id")
            .or_else(|| var.attribute("name"))
            .map(str::trim)
            .filter(|n| !n.is_empty())
        else {
            continue;
        };
        let values = var
            .children()
            .filter(|n| n.has_tag_name("value") && os_ok(n.attribute("os")))
            .map(text)
            .filter(|v| !v.is_empty() && !expand(v, env).is_empty());
        vars.entry(name.to_string()).or_default().extend(values);
    }
    vars
}

fn parse_option(
    node: Node,
    vars: &HashMap<String, Vec<String>>,
    extra_reason: Option<&str>,
) -> Option<Opt> {
    if !os_ok(node.attribute("os")) {
        return None;
    }
    let id = node.attribute("id").map(str::trim).filter(|s| !s.is_empty())?;
    let mut actions = Vec::new();
    let mut reasons: Vec<String> = extra_reason.map(str::to_string).into_iter().collect();
    for action in node
        .children()
        .filter(|n| n.has_tag_name("action") && os_ok(n.attribute("os")))
    {
        match parse_action(action, vars) {
            Ok(list) => actions.extend(list),
            Err(reason) => {
                if !reasons.contains(&reason) {
                    reasons.push(reason);
                }
            }
        }
    }
    if actions.is_empty() && reasons.is_empty() {
        // niente da fare su Windows (azioni di altri sistemi o variabili vuote)
        return None;
    }
    Some(Opt {
        id: id.to_string(),
        label: child_text(node, "label").unwrap_or_else(|| id.to_string()),
        description: child_text(node, "description").unwrap_or_default(),
        warning: child_text(node, "warning"),
        default_on: false,
        actions,
        excludes: Vec::new(),
        unsupported: (!reasons.is_empty())
            .then(|| format!("non supportato: {}", reasons.join(", "))),
    })
}

/// `Err(motivo)`: l'azione non è supportata; il motivo è il comando, il
/// `search` o la caratteristica non gestita.
fn parse_action(a: Node, vars: &HashMap<String, Vec<String>>) -> Result<Vec<Action>, String> {
    // Whitelist: un attributo che non si capisce potrebbe essere un filtro, e
    // ignorarlo allargherebbe l'azione.
    const KNOWN: [&str; 8] = [
        "command", "search", "path", "regex", "nregex", "type", "os", "name",
    ];
    for attr in a.attributes() {
        let name = attr.name();
        if KNOWN.contains(&name) {
            continue;
        }
        return Err(match name {
            "wholeregex" | "nwholeregex" => name.to_string(),
            _ => format!("attributo {name}"),
        });
    }
    match a.attribute("command").map(str::trim).unwrap_or("") {
        "delete" => parse_delete(a, vars),
        "winreg" => parse_winreg(a, vars),
        "" => Err("azione senza comando".into()),
        other => Err(other.to_string()),
    }
}

fn parse_delete(a: Node, vars: &HashMap<String, Vec<String>>) -> Result<Vec<Action>, String> {
    let mut search = match a.attribute("search").map(str::trim).unwrap_or("") {
        "file" => Search::File,
        "glob" => Search::Glob,
        "walk.all" => Search::WalkAll,
        "walk.files" => Search::WalkFiles,
        "walk.top" => Search::WalkTop,
        "" => return Err("azione senza search".into()),
        other => return Err(other.to_string()),
    };
    let path = a
        .attribute("path")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or("azione senza percorso")?;
    if a.has_attribute("nregex") {
        return Err("nregex".into());
    }
    match a.attribute("type").map(str::trim) {
        None | Some("") => {}
        // solo file: `walk.all` con `type="f"` è `walk.files`
        Some("f") => {
            if search == Search::WalkAll {
                search = Search::WalkFiles;
            }
        }
        Some(other) => return Err(format!("type={other}")),
    }
    let patterns = match a.attribute("regex").map(str::trim).filter(|r| !r.is_empty()) {
        None => Vec::new(),
        Some(re) if matches!(search, Search::WalkAll | Search::WalkFiles | Search::WalkTop) => {
            translate_regex(re).map_err(|_| format!("regex {re}"))?
        }
        Some(re) => return Err(format!("regex {re} con questo search")),
    };
    let recurse = matches!(search, Search::WalkAll | Search::WalkFiles);
    Ok(substitute(path, vars)?
        .into_iter()
        .map(|path| Action::Files {
            path,
            patterns: patterns.clone(),
            recurse,
            remove_self: false,
            search,
        })
        .collect())
}

fn parse_winreg(a: Node, vars: &HashMap<String, Vec<String>>) -> Result<Vec<Action>, String> {
    let full = a
        .attribute("path")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or("azione senza percorso")?;
    let (root_name, rest) = full.split_once('\\').unwrap_or((full, ""));
    if rest.trim_matches('\\').is_empty() {
        return Err("winreg senza chiave".into());
    }
    let root = match root_name.to_ascii_uppercase().as_str() {
        "HKCU" | "HKEY_CURRENT_USER" => RegRoot::Hkcu,
        "HKLM" | "HKEY_LOCAL_MACHINE" => RegRoot::Hklm,
        _ => return Err(format!("winreg {root_name}")),
    };
    let value = a.attribute("name").map(str::trim).filter(|v| !v.is_empty());
    Ok(substitute(rest, vars)?
        .into_iter()
        .map(|path| match value {
            Some(value) => Action::RegValue {
                root,
                path,
                value: value.to_string(),
            },
            None => Action::RegKey { root, path },
        })
        .collect())
}

/// Sostituisce ogni `$$nome$$` con i valori della variabile (prodotto
/// cartesiano se ce ne sono più d'una). `Err` se la variabile non esiste nel
/// file; vuoto se tutti i suoi valori sono stati scartati.
fn substitute(path: &str, vars: &HashMap<String, Vec<String>>) -> Result<Vec<String>, String> {
    let mut out = vec![String::new()];
    let mut rest = path;
    while let Some(start) = rest.find("$$") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("$$") else { break };
        let name = &after[..end];
        let values = vars
            .get(name)
            .ok_or_else(|| format!("variabile $${name}$$ sconosciuta"))?;
        out = out
            .iter()
            .flat_map(|p| values.iter().map(move |v| format!("{p}{}{v}", &rest[..start])))
            .collect();
        // il testo prima di `$$` è già stato aggiunto in ogni combinazione
        rest = &after[end + 2..];
    }
    for p in &mut out {
        p.push_str(rest);
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Lit(char),
    Star,
    One,
    Start,
    End,
}

/// Traduce la regex di `walk.*` (applicata al nome del file) in modelli con
/// `*` e `?`. Si capiscono solo caratteri letterali (anche con `\`), `.`, `.*`,
/// `^`, `$` e gruppi con `|`; il resto è `Err` e l'opzione resta non supportata.
/// I modelli non distinguono le maiuscole, la regex sì: la traduzione è un
/// po' più larga. Senza `^` il modello comincia con `*`, senza `$` finisce con `*`.
fn translate_regex(re: &str) -> Result<Vec<String>, String> {
    let mut chars = re.chars().peekable();
    let alts = parse_alts(&mut chars, 0)?;
    if chars.next().is_some() {
        return Err("parentesi non bilanciate".into());
    }
    let mut out: Vec<String> = Vec::new();
    for alt in alts {
        let last = alt.len().saturating_sub(1);
        let mut pattern = String::new();
        let anchored_start = alt.first() == Some(&Tok::Start);
        let anchored_end = alt.last() == Some(&Tok::End);
        if !anchored_start {
            pattern.push('*');
        }
        for (i, tok) in alt.iter().enumerate() {
            match tok {
                Tok::Lit(c) => pattern.push(*c),
                Tok::Star => pattern.push('*'),
                Tok::One => pattern.push('?'),
                Tok::Start if i == 0 => {}
                Tok::End if i == last => {}
                _ => return Err("ancora `^` o `$` in mezzo al modello".into()),
            }
        }
        if !anchored_end {
            pattern.push('*');
        }
        while pattern.contains("**") {
            pattern = pattern.replace("**", "*");
        }
        // `*.*` vuol dire «tutto ciò che ha un punto»: una regex come `\.` non lo chiede
        if pattern == "*.*" {
            return Err("tradotta in *.* (troppo larga)".into());
        }
        if !out.contains(&pattern) {
            out.push(pattern);
        }
    }
    Ok(out)
}

/// Alternative separate da `|` fino a `)` (o a fine testo al livello 0). Ogni
/// gruppo moltiplica le alternative già lette.
fn parse_alts(chars: &mut Peekable<Chars>, depth: usize) -> Result<Vec<Vec<Tok>>, String> {
    const MAX_ALTERNATIVES: usize = 64;
    let mut done: Vec<Vec<Tok>> = Vec::new();
    let mut current: Vec<Vec<Tok>> = vec![Vec::new()];
    loop {
        let Some(c) = chars.next() else {
            if depth > 0 {
                return Err("parentesi non chiusa".into());
            }
            done.extend(current);
            return Ok(done);
        };
        match c {
            ')' => {
                if depth == 0 {
                    return Err("parentesi chiusa in più".into());
                }
                done.extend(current);
                return Ok(done);
            }
            '|' => {
                done.extend(std::mem::replace(&mut current, vec![Vec::new()]));
            }
            '(' => {
                if chars.peek() == Some(&'?') {
                    chars.next();
                    if chars.next() != Some(':') {
                        return Err("gruppo speciale non gestito".into());
                    }
                }
                let sub = parse_alts(chars, depth + 1)?;
                current = current
                    .iter()
                    .flat_map(|p| {
                        sub.iter().map(move |s| {
                            let mut v = p.clone();
                            v.extend(s.iter().copied());
                            v
                        })
                    })
                    .collect();
                if current.len() + done.len() > MAX_ALTERNATIVES {
                    return Err("troppe alternative".into());
                }
            }
            '^' => push_all(&mut current, Tok::Start),
            '$' => push_all(&mut current, Tok::End),
            '.' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    push_all(&mut current, Tok::Star);
                } else {
                    push_all(&mut current, Tok::One);
                }
            }
            '\\' => match chars.next() {
                Some(e) if !e.is_alphanumeric() && e != '*' && e != '?' => {
                    push_all(&mut current, Tok::Lit(e));
                }
                _ => return Err("escape non gestito".into()),
            },
            '*' | '+' | '?' | '{' | '}' | '[' | ']' => {
                return Err(format!("operatore «{c}» non gestito"));
            }
            // il raccoglitore separa i modelli con `;`
            ';' => return Err("«;» letterale non traducibile".into()),
            c => push_all(&mut current, Tok::Lit(c)),
        }
    }
}

fn push_all(alts: &mut [Vec<Tok>], tok: Tok) {
    for a in alts {
        a.push(tok);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Action, Opt, Origin, RegRoot, Search};
    use std::collections::HashMap;

    const SAMPLE: &str = include_str!("../tests/fixtures/cleanerml_sample.xml");

    fn env(pairs: &[(&str, &str)]) -> Env {
        Env(pairs
            .iter()
            .map(|(k, v)| (k.to_lowercase(), v.to_string()))
            .collect::<HashMap<_, _>>())
    }

    fn full_env() -> Env {
        env(&[
            ("userprofile", r"C:\Users\u"),
            ("appdata", r"C:\Users\u\AppData\Roaming"),
            ("localappdata", r"C:\Users\u\AppData\Local"),
            ("xdg_data_home_missing", r"C:\Xdg"),
        ])
    }

    /// Come `full_env`, ma senza la variabile usata da un solo valore di `roots`.
    fn env_without_xdg() -> Env {
        env(&[
            ("userprofile", r"C:\Users\u"),
            ("appdata", r"C:\Users\u\AppData\Roaming"),
        ])
    }

    fn parse(env: &Env) -> Cleaner {
        parse_cleaner(SAMPLE, "sample", env).expect("il file di esempio si legge")
    }

    fn opt<'a>(c: &'a Cleaner, id: &str) -> &'a Opt {
        c.options
            .iter()
            .find(|o| o.id == id)
            .unwrap_or_else(|| panic!("opzione {id} assente"))
    }

    fn files(path: &str, patterns: &[&str], recurse: bool, search: Search) -> Action {
        Action::Files {
            path: path.to_string(),
            patterns: patterns.iter().map(|s| s.to_string()).collect(),
            recurse,
            remove_self: false,
            search,
        }
    }

    #[test]
    fn reads_label_options_and_warning() {
        let c = parse(&full_env());
        assert_eq!(c.id, "sample_tool");
        assert_eq!(c.label, "Strumento di prova (AI)");
        assert_eq!(c.description, "Pulizia selettiva di cache, trascrizioni e cronologia.");
        assert_eq!(c.origin, Origin::CleanerMl("sample"));
        let ids: Vec<&str> = c.options.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(
            ids,
            ["cache", "transcripts", "history", "multi", "database", "registry"]
        );
        let t = opt(&c, "transcripts");
        assert_eq!(t.label, "Trascrizioni sessioni");
        assert_eq!(t.description, "Elimina i file JSONL delle conversazioni.");
        assert_eq!(
            t.warning.as_deref(),
            Some("Non potrai riprendere le conversazioni eliminate.")
        );
        assert_eq!(opt(&c, "cache").warning, None);
    }

    #[test]
    fn running_processes_are_collected() {
        let c = parse(&full_env());
        // il processo con os="linux" non conta
        assert_eq!(c.running, vec!["claude.exe", "claude-helper.exe"]);
    }

    #[test]
    fn search_kinds_map_to_actions() {
        let c = parse(&full_env());
        assert_eq!(
            opt(&c, "cache").actions,
            vec![files(r"%USERPROFILE%\.sample\cache", &[], true, Search::WalkAll)]
        );
        // walk.files + regex: il modello è a caratteri jolly, la regex si traduce
        assert_eq!(
            opt(&c, "transcripts").actions,
            vec![files(
                r"%USERPROFILE%\.sample\projects",
                &["*.jsonl", "*.jsonl.*"],
                true,
                Search::WalkFiles
            )]
        );
        assert_eq!(
            opt(&c, "history").actions,
            vec![
                files(r"%USERPROFILE%\.sample\history.jsonl", &[], false, Search::File),
                files(r"%USERPROFILE%\.sample\logs_*.sqlite*", &[], false, Search::Glob),
                files(r"%USERPROFILE%\.sample\tmp", &[], false, Search::WalkTop),
            ]
        );
        assert!(opt(&c, "history").unsupported.is_none());
    }

    #[test]
    fn registry_actions_map_to_reg_key_and_value() {
        let c = parse(&full_env());
        assert_eq!(
            opt(&c, "registry").actions,
            vec![
                Action::RegKey {
                    root: RegRoot::Hkcu,
                    path: r"Software\Sample\Recent".into()
                },
                Action::RegValue {
                    root: RegRoot::Hkcu,
                    path: r"Software\Sample".into(),
                    value: "LastFile".into()
                },
            ]
        );
    }

    #[test]
    fn var_with_several_values_multiplies_the_actions() {
        let c = parse(&full_env());
        // tre valori validi (il quarto è solo per Linux)
        assert_eq!(
            opt(&c, "multi").actions,
            vec![
                files(r"%USERPROFILE%\.sample\cache", &[], true, Search::WalkAll),
                files(r"%XDG_DATA_HOME_MISSING%\sample\cache", &[], true, Search::WalkAll),
                files(r"%APPDATA%\Sample\cache", &[], true, Search::WalkAll),
            ]
        );
    }

    #[test]
    fn var_value_with_undefined_env_is_dropped() {
        let c = parse(&env_without_xdg());
        assert_eq!(
            opt(&c, "multi").actions,
            vec![
                files(r"%USERPROFILE%\.sample\cache", &[], true, Search::WalkAll),
                files(r"%APPDATA%\Sample\cache", &[], true, Search::WalkAll),
            ]
        );
        assert!(opt(&c, "multi").unsupported.is_none());
    }

    #[test]
    fn unsupported_commands_are_flagged_not_dropped() {
        let c = parse(&full_env());
        let db = opt(&c, "database");
        assert_eq!(db.unsupported.as_deref(), Some("non supportato: sqlite.vacuum"));
        assert_eq!(db.label, "Database (avanzato)");
        assert!(!db.default_on);
    }

    #[test]
    fn unsupported_search_regex_and_variables_are_flagged() {
        let xml = r#"<cleaner id="x" os="windows"><label>X</label>
          <option id="a"><label>A</label>
            <action command="delete" search="deep" path="%TEMP%"/>
            <action command="delete" search="file" path="%TEMP%\ok.txt"/>
          </option>
          <option id="b"><label>B</label>
            <action command="delete" search="walk.files" path="%TEMP%" nregex="\.keep$"/>
          </option>
          <option id="c"><label>C</label>
            <action command="delete" search="walk.files" path="%TEMP%" regex="[0-9]+\.tmp"/>
          </option>
          <option id="d"><label>D</label>
            <action command="delete" search="walk.all" path="$$nope$$\x"/>
          </option>
          <option id="e"><label>E</label>
            <action command="delete" search="walk.all" path="%TEMP%" type="d"/>
          </option>
        </cleaner>"#;
        let c = parse_cleaner(xml, "x", &env(&[("temp", r"C:\T")])).unwrap();
        let a = opt(&c, "a");
        assert_eq!(a.unsupported.as_deref(), Some("non supportato: deep"));
        // l'azione supportata della stessa opzione non si perde
        assert_eq!(a.actions, vec![files(r"%TEMP%\ok.txt", &[], false, Search::File)]);
        assert_eq!(opt(&c, "b").unsupported.as_deref(), Some("non supportato: nregex"));
        assert!(
            opt(&c, "c")
                .unsupported
                .as_deref()
                .unwrap()
                .starts_with("non supportato: regex ")
        );
        assert!(
            opt(&c, "d")
                .unsupported
                .as_deref()
                .unwrap()
                .contains("$$nope$$")
        );
        assert!(opt(&c, "e").unsupported.is_some());
    }

    #[test]
    fn unknown_filter_attributes_are_never_ignored() {
        // un filtro che non si capisce non deve allargare l'azione (fail-open)
        let xml = r#"<cleaner id="x" os="windows"><label>X</label>
          <option id="w"><label>W</label>
            <action command="delete" search="walk.files" path="%TEMP%\a" wholeregex=".*\.log"/>
          </option>
          <option id="n"><label>N</label>
            <action command="delete" search="walk.files" path="%TEMP%\a" nwholeregex=".*\.keep"/>
          </option>
          <option id="f"><label>F</label>
            <action command="delete" search="walk.files" path="%TEMP%\a" foo="1"/>
          </option>
          <option id="r"><label>R</label>
            <action command="winreg" path="HKCU\Software\X" foo="1"/>
          </option>
          <option id="ok"><label>OK</label>
            <action command="delete" os="windows" search="walk.files" path="%TEMP%\a" regex="\.log$" type="f"/>
          </option>
        </cleaner>"#;
        let c = parse_cleaner(xml, "x", &env(&[("temp", r"C:\T")])).unwrap();
        for (id, reason) in [
            ("w", "non supportato: wholeregex"),
            ("n", "non supportato: nwholeregex"),
            ("f", "non supportato: attributo foo"),
            ("r", "non supportato: attributo foo"),
        ] {
            let o = opt(&c, id);
            assert_eq!(o.unsupported.as_deref(), Some(reason), "{id}");
            assert!(o.actions.is_empty(), "{id}: l'azione non deve restare");
        }
        let ok = opt(&c, "ok");
        assert!(ok.unsupported.is_none());
        assert_eq!(ok.actions.len(), 1);
    }

    #[test]
    fn winreg_without_a_key_path_is_unsupported() {
        let xml = r#"<cleaner id="x"><label>X</label>
          <option id="a"><label>A</label><action command="winreg" path="HKCU"/></option>
          <option id="b"><label>B</label><action command="winreg" path="HKCU\"/></option>
        </cleaner>"#;
        let c = parse_cleaner(xml, "x", &env(&[])).unwrap();
        for id in ["a", "b"] {
            let o = opt(&c, id);
            assert!(o.unsupported.is_some(), "{id}");
            assert!(o.actions.is_empty(), "{id}");
        }
    }

    #[test]
    fn running_of_another_type_flags_the_options() {
        let xml = r#"<cleaner id="x"><label>X</label>
          <running type="pathname" os="windows">%TEMP%\x.lock</running>
          <running type="exe" os="windows">x.exe</running>
          <option id="a"><label>A</label><action command="delete" search="file" path="%TEMP%\a"/></option>
        </cleaner>"#;
        let c = parse_cleaner(xml, "x", &env(&[])).unwrap();
        assert_eq!(c.running, vec!["x.exe"]);
        let reason = opt(&c, "a").unsupported.as_deref().unwrap();
        assert!(reason.starts_with("non supportato: "), "{reason}");
        assert!(reason.contains("tipo di processo"), "{reason}");
        // un `running` di un altro sistema operativo non conta
        let xml = r#"<cleaner id="x"><label>X</label>
          <running type="pathname" os="linux">/tmp/x</running>
          <option id="a"><label>A</label><action command="delete" search="file" path="%TEMP%\a"/></option>
        </cleaner>"#;
        let c = parse_cleaner(xml, "x", &env(&[])).unwrap();
        assert!(opt(&c, "a").unsupported.is_none());
    }

    #[test]
    fn walk_all_with_type_f_becomes_walk_files() {
        let xml = r#"<cleaner id="x"><label>X</label><option id="a"><label>A</label>
            <action command="delete" search="walk.all" path="%TEMP%\a" type="f"/>
          </option></cleaner>"#;
        let c = parse_cleaner(xml, "x", &env(&[])).unwrap();
        assert_eq!(
            opt(&c, "a").actions,
            vec![files(r"%TEMP%\a", &[], true, Search::WalkFiles)]
        );
    }

    #[test]
    fn var_may_use_name_and_value_search_attributes() {
        // come bbag_jetbrains.xml: `name` al posto di `id`, `search` sul valore
        let xml = r#"<cleaner id="jb" os="windows"><label>JB</label>
          <var name="targets"><value search="glob" os="windows">%LocalAppData%\JetBrains\*\caches</value></var>
          <option id="i"><label>I</label>
            <action command="delete" search="walk.files" path="$$targets$$"/>
          </option></cleaner>"#;
        let c = parse_cleaner(xml, "jb", &env(&[("localappdata", r"C:\L")])).unwrap();
        assert_eq!(
            opt(&c, "i").actions,
            vec![files(
                r"%LocalAppData%\JetBrains\*\caches",
                &[],
                true,
                Search::WalkFiles
            )]
        );
    }

    #[test]
    fn non_windows_cleaner_is_rejected() {
        let xml = r#"<cleaner id="x" os="linux"><label>X</label></cleaner>"#;
        assert_eq!(
            parse_cleaner(xml, "x", &env(&[])),
            Err("os non windows".to_string())
        );
        // senza `os` si accetta
        let xml = r#"<cleaner id="x"><label>X</label></cleaner>"#;
        assert!(parse_cleaner(xml, "x", &env(&[])).is_ok());
    }

    #[test]
    fn malformed_input_is_an_error() {
        assert!(parse_cleaner("<cleaner id=", "x", &env(&[])).is_err());
        assert!(parse_cleaner("<altro/>", "x", &env(&[])).is_err());
        assert!(parse_cleaner(r#"<cleaner os="windows"/>"#, "x", &env(&[])).is_err());
    }

    #[test]
    fn everything_is_off_by_default() {
        let c = parse(&full_env());
        assert!(!c.options.is_empty());
        assert!(c.options.iter().all(|o| !o.default_on));
    }

    #[test]
    fn os_specific_actions_are_skipped() {
        let xml = r#"<cleaner id="x"><label>X</label>
          <option id="a"><label>A</label>
            <action command="delete" os="linux" search="file" path="/tmp/x"/>
            <action command="delete" os="windows" search="file" path="%TEMP%\x"/>
          </option>
          <option id="b"><label>B</label>
            <action command="delete" os="linux" search="file" path="/tmp/y"/>
          </option></cleaner>"#;
        let c = parse_cleaner(xml, "x", &env(&[])).unwrap();
        assert_eq!(c.options.len(), 1);
        assert_eq!(c.options[0].actions.len(), 1);
    }

    #[test]
    fn bom_is_accepted() {
        let xml = "\u{feff}<cleaner id=\"x\"><label>X</label></cleaner>";
        assert!(parse_cleaner(xml, "x", &env(&[])).is_ok());
    }

    #[test]
    fn regexes_translate_to_wildcards() {
        let t = |re: &str| translate_regex(re);
        assert_eq!(t(r"\.jsonl($|\.)").unwrap(), ["*.jsonl", "*.jsonl.*"]);
        assert_eq!(t(r"\.log$").unwrap(), ["*.log"]);
        assert_eq!(t(r"^cache.*\.bin$").unwrap(), ["cache*.bin"]);
        assert_eq!(t(r"\.(log|tmp)$").unwrap(), ["*.log", "*.tmp"]);
        assert_eq!(t("a.c").unwrap(), ["*a?c*"]);
        assert!(t(r"\d+").is_err());
        assert!(t("[ab]").is_err());
        assert!(t("a+").is_err());
        assert!(t("a$b").is_err());
        // `;` separa i modelli nel raccoglitore: un `;` letterale non si traduce
        assert!(t("a;b").is_err());
        // `*.*` vorrebbe dire «qualsiasi cosa con un punto»: mai da una regex come `\.`
        assert!(t(r"\.").is_err());
        assert!(t(r"\.(log|tmp)").is_ok());
    }

    /// I file veri dell'utente: sola lettura della cartella in `PC_RULES_CLEANERS`.
    #[test]
    #[ignore]
    fn parses_the_eleven_user_files() {
        let dir = std::env::var("PC_RULES_CLEANERS")
            .expect("PC_RULES_CLEANERS deve puntare alla cartella dei file XML");
        let env = full_env();
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "xml"))
            .collect();
        names.sort();
        assert_eq!(names.len(), 11, "attesi 11 file XML in {dir}");
        for path in names {
            let text = std::fs::read_to_string(&path).unwrap();
            let c = parse_cleaner(&text, "utente", &env)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(!c.options.is_empty(), "{}: nessuna opzione", path.display());
            for o in &c.options {
                assert!(!o.default_on);
                assert!(o.unsupported.is_none(), "{}/{}: {:?}", c.id, o.id, o.unsupported);
                assert!(!o.actions.is_empty(), "{}/{}: nessuna azione", c.id, o.id);
            }
            println!(
                "{}: {} opzioni, {} azioni, {} processi",
                c.id,
                c.options.len(),
                c.options.iter().map(|o| o.actions.len()).sum::<usize>(),
                c.running.len()
            );
        }
    }
}
