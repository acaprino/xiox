//! I cleaner CleanerML inglobati nell'eseguibile (cartella `cleaners/` alla
//! radice del progetto).

use crate::cleanerml::parse_cleaner;
use crate::model::{Cleaner, Env};

/// Nome e testo di ogni cleaner inglobato.
const CLEANERS: [(&str, &str); 11] = [
    ("ai_claude_code", include_str!("../../../cleaners/ai_claude_code.xml")),
    ("ai_codex", include_str!("../../../cleaners/ai_codex.xml")),
    ("ai_gemini_cli", include_str!("../../../cleaners/ai_gemini_cli.xml")),
    ("ai_opencode", include_str!("../../../cleaners/ai_opencode.xml")),
    ("bbag_cargo", include_str!("../../../cleaners/bbag_cargo.xml")),
    ("bbag_crash", include_str!("../../../cleaners/bbag_crash.xml")),
    ("bbag_go", include_str!("../../../cleaners/bbag_go.xml")),
    ("bbag_jetbrains", include_str!("../../../cleaners/bbag_jetbrains.xml")),
    ("bbag_npm", include_str!("../../../cleaners/bbag_npm.xml")),
    ("bbag_pip", include_str!("../../../cleaners/bbag_pip.xml")),
    ("bbag_shader", include_str!("../../../cleaners/bbag_shader.xml")),
];

/// Legge gli 11 cleaner inglobati. Un XML che non si legge dà un avviso e non
/// ferma gli altri.
pub fn embedded(env: &Env) -> (Vec<Cleaner>, Vec<String>) {
    let mut cleaners = Vec::new();
    let mut warnings = Vec::new();
    for (name, xml) in CLEANERS {
        match parse_cleaner(xml, name, env) {
            Ok(c) => cleaners.push(c),
            Err(e) => warnings.push(format!("cleaner inglobato {name}: {e}")),
        }
    }
    (cleaners, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::running;
    use std::collections::HashMap;

    fn load() -> (Vec<Cleaner>, Vec<String>) {
        // Senza `LocalAppData` l'opzione JetBrains (che usa un `<var>`) sparirebbe:
        // si passa un ambiente finto completo.
        let env = Env(HashMap::from([
            ("userprofile".to_string(), r"C:\Users\t".to_string()),
            ("localappdata".to_string(), r"C:\Users\t\AppData\Local".to_string()),
            ("temp".to_string(), r"C:\Users\t\AppData\Local\Temp".to_string()),
        ]));
        embedded(&env)
    }

    /// Ogni testo visibile (etichette, descrizioni, avvisi) di ogni cleaner.
    fn all_texts(cleaners: &[Cleaner]) -> Vec<String> {
        let mut out = Vec::new();
        for c in cleaners {
            out.push(c.label.clone());
            out.push(c.description.clone());
            for o in &c.options {
                out.push(o.label.clone());
                out.push(o.description.clone());
                out.extend(o.warning.clone());
            }
        }
        out
    }

    #[test]
    fn all_eleven_embedded_cleaners_parse() {
        let (cleaners, warnings) = load();
        assert_eq!(cleaners.len(), 11);
        assert!(warnings.is_empty(), "avvisi: {warnings:?}");
        assert!(cleaners.iter().all(|c| c.group == "Sviluppo e AI"));
    }

    #[test]
    fn no_label_has_the_old_prefix_or_missing_accents() {
        let (cleaners, _) = load();
        for c in &cleaners {
            assert!(!c.label.starts_with("AGG"), "prefisso nel cleaner {}", c.id);
            for o in &c.options {
                assert!(!o.label.starts_with("AGG"), "prefisso nell'opzione {}", o.id);
            }
        }
        for t in all_texts(&cleaners) {
            for bad in ["sara piu", "possibilita", "puo ", "e stato"] {
                assert!(!t.contains(bad), "«{bad}» in «{t}»");
            }
        }
    }

    #[test]
    fn no_text_mentions_the_donor_project() {
        let (cleaners, _) = load();
        for t in all_texts(&cleaners) {
            assert!(!t.to_lowercase().contains("bleachbit"), "«{t}»");
        }
        for (name, xml) in CLEANERS {
            assert!(!xml.to_lowercase().contains("bleachbit"), "{name}");
        }
    }

    #[test]
    fn embedded_xml_have_no_english_leftovers() {
        let (cleaners, _) = load();
        for t in all_texts(&cleaners) {
            let low = t.to_lowercase();
            for bad in ["downloads", "build cache", "package cache"] {
                assert!(!low.contains(bad), "«{bad}» in «{t}»");
            }
        }
    }

    #[test]
    fn every_option_is_off_by_default() {
        let (cleaners, _) = load();
        assert!(!cleaners.is_empty());
        for c in &cleaners {
            assert!(!c.options.is_empty(), "{} senza opzioni", c.id);
            for o in &c.options {
                assert!(!o.default_on, "{}/{} è acceso di default", c.id, o.id);
            }
        }
    }

    #[test]
    fn claude_code_cleaner_blocks_while_claude_runs() {
        let (cleaners, _) = load();
        let c = cleaners
            .iter()
            .find(|c| c.id == "ai_claude_code")
            .expect("cleaner ai_claude_code");
        assert_eq!(running(c, &["claude.exe".to_string()]), vec!["claude.exe"]);
        assert!(running(c, &["notepad.exe".to_string()]).is_empty());
    }

    /// Le correzioni ai testi non toccano opzioni né azioni: numero di
    /// opzioni e azioni e id delle opzioni di ogni file restano quelli
    /// originali.
    #[test]
    fn xml_structure_unchanged() {
        let expected: [(&str, usize, usize, &[&str]); 11] = [
            ("ai_claude_code", 6, 13, &[]),
            ("ai_codex", 7, 9, &[]),
            ("ai_gemini_cli", 4, 5, &[]),
            ("ai_opencode", 3, 3, &[]),
            ("bbag_cargo", 1, 1, &[]),
            ("bbag_crash", 1, 1, &[]),
            ("bbag_go", 1, 1, &["build"]),
            ("bbag_jetbrains", 1, 1, &[]),
            ("bbag_npm", 3, 3, &[]),
            ("bbag_pip", 1, 3, &[]),
            ("bbag_shader", 3, 5, &[]),
        ];
        for ((name, xml), (want, options, actions, ids)) in CLEANERS.iter().zip(expected) {
            assert_eq!(*name, want);
            let doc = roxmltree::Document::parse(xml).unwrap();
            let count = |tag: &str| doc.descendants().filter(|n| n.has_tag_name(tag)).count();
            assert_eq!(count("option"), options, "{name}: opzioni");
            assert_eq!(count("action"), actions, "{name}: azioni");
            let got: Vec<&str> = doc
                .descendants()
                .filter(|n| n.has_tag_name("option"))
                .filter_map(|n| n.attribute("id"))
                .collect();
            for id in ids {
                assert!(got.contains(id), "{name}: manca l'opzione {id}");
            }
        }
    }

    #[test]
    fn reviewed_italian_wording() {
        let all: String = CLEANERS.iter().map(|(_, x)| *x).collect();
        for bad in [
            "incollature",
            "Perdi ricerca",
            "Trascrizioni sessioni",
            "Checkpoint snapshot",
            "sessioni legacy",
            "V1 / legacy",
            "Log applicazione",
            "Distrugge dati",
            "a supporto tecnico",
            "al successivo build",
            "le build",
            "Cache di build",
        ] {
            assert!(!all.contains(bad), "testo da correggere ancora presente: «{bad}»");
        }
    }
}
