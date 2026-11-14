//! I cleaner CleanerML inglobati nell'eseguibile (cartella `cleaners/` alla
//! radice del progetto).

use crate::cleanerml::parse_cleaner;
use crate::model::{Cleaner, Env};

/// Nome e testo di ogni cleaner inglobato. Il gruppo con cui compare
/// nell'elenco lo dà [`group_of`] dal nome.
const CLEANERS: [(&str, &str); 61] = [
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
    ("android", include_str!("../../../cleaners/android.xml")),
    ("bazel", include_str!("../../../cleaners/bazel.xml")),
    ("bun", include_str!("../../../cleaners/bun.xml")),
    ("bundler", include_str!("../../../cleaners/bundler.xml")),
    ("cabal", include_str!("../../../cleaners/cabal.xml")),
    ("ccache", include_str!("../../../cleaners/ccache.xml")),
    ("composer", include_str!("../../../cleaners/composer.xml")),
    ("conan", include_str!("../../../cleaners/conan.xml")),
    ("conda", include_str!("../../../cleaners/conda.xml")),
    ("cypress", include_str!("../../../cleaners/cypress.xml")),
    ("deno", include_str!("../../../cleaners/deno.xml")),
    ("dotnet_bundle", include_str!("../../../cleaners/dotnet_bundle.xml")),
    ("electron", include_str!("../../../cleaners/electron.xml")),
    ("firebase", include_str!("../../../cleaners/firebase.xml")),
    ("gradle", include_str!("../../../cleaners/gradle.xml")),
    ("hex", include_str!("../../../cleaners/hex.xml")),
    ("huggingface", include_str!("../../../cleaners/huggingface.xml")),
    ("julia", include_str!("../../../cleaners/julia.xml")),
    ("keras", include_str!("../../../cleaners/keras.xml")),
    ("kotlin_native", include_str!("../../../cleaners/kotlin_native.xml")),
    ("kubectl", include_str!("../../../cleaners/kubectl.xml")),
    ("maven", include_str!("../../../cleaners/maven.xml")),
    ("minikube", include_str!("../../../cleaners/minikube.xml")),
    ("nuget", include_str!("../../../cleaners/nuget.xml")),
    ("ollama", include_str!("../../../cleaners/ollama.xml")),
    ("pipenv", include_str!("../../../cleaners/pipenv.xml")),
    ("platformio", include_str!("../../../cleaners/platformio.xml")),
    ("playwright", include_str!("../../../cleaners/playwright.xml")),
    ("pnpm", include_str!("../../../cleaners/pnpm.xml")),
    ("poetry", include_str!("../../../cleaners/poetry.xml")),
    ("precommit", include_str!("../../../cleaners/precommit.xml")),
    ("pub", include_str!("../../../cleaners/pub.xml")),
    ("puppeteer", include_str!("../../../cleaners/puppeteer.xml")),
    ("pyenv", include_str!("../../../cleaners/pyenv.xml")),
    ("pytest", include_str!("../../../cleaners/pytest.xml")),
    ("pytorch", include_str!("../../../cleaners/pytorch.xml")),
    ("rustup", include_str!("../../../cleaners/rustup.xml")),
    ("sbt", include_str!("../../../cleaners/sbt.xml")),
    ("scoop", include_str!("../../../cleaners/scoop.xml")),
    ("terraform", include_str!("../../../cleaners/terraform.xml")),
    ("uv", include_str!("../../../cleaners/uv.xml")),
    ("vagrant", include_str!("../../../cleaners/vagrant.xml")),
    ("vcpkg", include_str!("../../../cleaners/vcpkg.xml")),
    ("virtualenv", include_str!("../../../cleaners/virtualenv.xml")),
    ("visualstudio", include_str!("../../../cleaners/visualstudio.xml")),
    ("vscode", include_str!("../../../cleaners/vscode.xml")),
    ("yarn", include_str!("../../../cleaners/yarn.xml")),
    ("zig", include_str!("../../../cleaners/zig.xml")),
    ("windows_extra", include_str!("../../../cleaners/windows_extra.xml")),
    ("safari", include_str!("../../../cleaners/safari.xml")),
];

/// Gruppo di un cleaner inglobato, dal nome: le voci di Windows e i browser
/// hanno un gruppo proprio, tutti gli altri sono «Sviluppo e AI» (il gruppo
/// che `parse_cleaner` assegna).
fn group_of(name: &str) -> Option<&'static str> {
    match name {
        "windows_extra" => Some("Windows"),
        "safari" => Some("Browser"),
        _ => None,
    }
}

/// Legge i cleaner inglobati. Un XML che non si legge dà un avviso e non
/// ferma gli altri.
pub fn embedded(env: &Env) -> (Vec<Cleaner>, Vec<String>) {
    let mut cleaners = Vec::new();
    let mut warnings = Vec::new();
    for (name, xml) in CLEANERS {
        match parse_cleaner(xml, name, env) {
            Ok(mut c) => {
                if let Some(group) = group_of(name) {
                    c.group = group.to_string();
                }
                cleaners.push(c);
            }
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
            ("username".to_string(), "t".to_string()),
            ("appdata".to_string(), r"C:\Users\t\AppData\Roaming".to_string()),
            ("programdata".to_string(), r"C:\ProgramData".to_string()),
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
    fn all_embedded_cleaners_parse() {
        let (cleaners, warnings) = load();
        assert_eq!(cleaners.len(), CLEANERS.len());
        assert_eq!(cleaners.len(), 61);
        assert!(warnings.is_empty(), "avvisi: {warnings:?}");
        for c in &cleaners {
            let want = match c.id.as_str() {
                "windows_extra" => "Windows",
                "safari" => "Browser",
                _ => "Sviluppo e AI",
            };
            assert_eq!(c.group, want, "gruppo di {}", c.id);
        }
        assert!(cleaners.iter().any(|c| c.id == "windows_extra"));
        assert!(cleaners.iter().any(|c| c.id == "safari"));
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
    fn embedded_xml_have_no_italian_leftovers() {
        let (cleaners, _) = load();
        for t in all_texts(&cleaners) {
            let low = t.to_lowercase();
            for bad in ["cartella", "rimuove", "elimina", "riscaric", "cache di ", "pulisci", "chiudi "] {
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

    /// I primi 11 file (quelli di partenza) hanno solo le opzioni indicate:
    /// i testi corretti non tolgono opzioni né azioni, e le aggiunte della
    /// Parte B (cargo, go, npm, shader) sono contate qui.
    #[test]
    fn xml_structure_unchanged() {
        let expected: [(&str, usize, usize, &[&str]); 11] = [
            ("ai_claude_code", 6, 13, &[]),
            ("ai_codex", 7, 9, &[]),
            ("ai_gemini_cli", 4, 5, &[]),
            ("ai_opencode", 3, 3, &[]),
            ("bbag_cargo", 5, 5, &[]),
            ("bbag_crash", 1, 1, &[]),
            ("bbag_go", 2, 2, &["build"]),
            ("bbag_jetbrains", 1, 1, &[]),
            ("bbag_npm", 4, 6, &[]),
            ("bbag_pip", 1, 3, &[]),
            ("bbag_shader", 4, 6, &[]),
        ];
        for ((name, xml), (want, options, actions, ids)) in CLEANERS.iter().take(11).zip(expected) {
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
    fn reviewed_wording() {
        let all: String = CLEANERS.iter().map(|(_, x)| *x).collect();
        for bad in [
            "incollature",
            "Perdi ricerca",
            "Trascrizioni sessioni",
            "Checkpoint snapshot",
            "sessioni legacy",
            "Log applicazione",
            "Distrugge dati",
            "a supporto tecnico",
            "al successivo build",
            "Cache di build",
        ] {
            assert!(!all.contains(bad), "testo da correggere ancora presente: «{bad}»");
        }
    }
}
