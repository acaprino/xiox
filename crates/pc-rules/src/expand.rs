//! Espansione di `%Variabile%` nei percorsi delle regole.

use std::path::PathBuf;

use crate::model::Env;

/// Sostituisce `%Nome%` (senza badare alle maiuscole). `%ProgramFiles%` dà sia
/// `Program Files` sia `Program Files (x86)` se presenti. Una variabile non
/// definita scarta il percorso: il risultato è vuoto.
pub fn expand(path: &str, env: &Env) -> Vec<PathBuf> {
    // Ogni percorso parziale ottenuto finora; ogni variabile li moltiplica per
    // il numero dei suoi valori.
    let mut partial: Vec<String> = vec![String::new()];
    let mut rest = path;
    while let Some(start) = rest.find('%') {
        let (literal, after) = rest.split_at(start);
        push_all(&mut partial, literal);
        let after = &after[1..];
        let name = after
            .find('%')
            .map(|end| &after[..end])
            .filter(|n| !n.is_empty() && !n.contains(['\\', '/']));
        match name {
            Some(name) => {
                let values = values_of(name, env);
                if values.is_empty() {
                    return Vec::new();
                }
                partial = partial
                    .iter()
                    .flat_map(|p| values.iter().map(move |v| format!("{p}{v}")))
                    .collect();
                rest = &after[name.len() + 1..];
            }
            None => {
                // `%` isolato: resta com'è.
                push_all(&mut partial, "%");
                rest = after;
            }
        }
    }
    push_all(&mut partial, rest);

    let mut out: Vec<PathBuf> = Vec::new();
    for p in partial {
        let p = PathBuf::from(p.replace('/', "\\"));
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

fn push_all(partial: &mut [String], s: &str) {
    for p in partial {
        p.push_str(s);
    }
}

fn values_of<'a>(name: &str, env: &'a Env) -> Vec<&'a str> {
    if name.eq_ignore_ascii_case("programfiles") {
        let mut v: Vec<&str> = ["programfiles", "programfiles(x86)"]
            .iter()
            .filter_map(|n| env.get(n))
            .collect();
        v.dedup();
        v
    } else {
        env.get(name).into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> Env {
        Env(pairs
            .iter()
            .map(|(k, v)| (k.to_lowercase(), v.to_string()))
            .collect::<HashMap<_, _>>())
    }

    #[test]
    fn expands_known_variables_case_insensitively() {
        let e = env(&[("localappdata", r"C:\U\L")]);
        assert_eq!(
            expand(r"%LOCALAPPDATA%\Foo", &e),
            vec![PathBuf::from(r"C:\U\L\Foo")]
        );
        assert_eq!(
            expand(r"%LocalAppData%\Foo", &e),
            vec![PathBuf::from(r"C:\U\L\Foo")]
        );
    }

    #[test]
    fn unknown_variable_discards_the_path() {
        let e = env(&[("localappdata", r"C:\U\L")]);
        assert!(expand(r"%NOPE%\x", &e).is_empty());
        // una variabile nota non salva una sconosciuta nello stesso percorso
        assert!(expand(r"%LOCALAPPDATA%\%NOPE%\x", &e).is_empty());
    }

    #[test]
    fn programfiles_expands_to_both_roots() {
        let e = env(&[
            ("programfiles", r"C:\Program Files"),
            ("programfiles(x86)", r"C:\Program Files (x86)"),
        ]);
        let got = expand(r"%ProgramFiles%\Foo", &e);
        assert_eq!(got.len(), 2);
        assert!(got.contains(&PathBuf::from(r"C:\Program Files\Foo")));
        assert!(got.contains(&PathBuf::from(r"C:\Program Files (x86)\Foo")));
        // il nome esplicito (x86) dà solo quella radice
        assert_eq!(
            expand(r"%ProgramFiles(x86)%\Foo", &e),
            vec![PathBuf::from(r"C:\Program Files (x86)\Foo")]
        );
    }

    #[test]
    fn programfiles_without_x86_gives_one_root() {
        let e = env(&[("programfiles", r"C:\Program Files")]);
        assert_eq!(
            expand(r"%ProgramFiles%\Foo", &e),
            vec![PathBuf::from(r"C:\Program Files\Foo")]
        );
    }

    #[test]
    fn path_without_variables_is_kept() {
        let e = env(&[]);
        assert_eq!(expand(r"C:\Foo\Bar", &e), vec![PathBuf::from(r"C:\Foo\Bar")]);
    }
}
