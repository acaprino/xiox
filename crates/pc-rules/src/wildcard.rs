//! Confronto con caratteri jolly (`*`, `?`) senza badare alle maiuscole.

/// `true` se `text` corrisponde a `pattern`, senza badare alle maiuscole
/// (minuscolo Unicode di entrambi). `*` vale zero o più caratteri, `?`
/// esattamente uno; ogni altro carattere vale sé stesso.
///
/// `*.*` si legge alla lettera: corrisponde solo a un testo che contiene un
/// punto (`a.txt` sì, `LICENSE` no). Non è la regola di `FindFirstFileW`, per
/// cui `*.*` vale tutto: chi vuole quel comportamento lo gestisce prima.
///
/// Algoritmo iterativo a due indici con un solo punto di ritorno (l'ultimo
/// `*` visto): al più `len(pattern) × len(text)` passi, mai esponenziale,
/// anche con molti `*` di fila o alternati a lettere.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    if pattern.is_ascii() && text.is_ascii() {
        // Caso comune (percorsi): sui byte, senza allocare.
        return match_symbols(pattern.as_bytes(), text.as_bytes(), u8::to_ascii_lowercase, b'*', b'?');
    }
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    match_symbols(&p, &t, |c: &char| *c, '*', '?')
}

/// L'algoritmo su sequenze di simboli: `low` porta un simbolo in minuscolo,
/// `run` e `one` sono `*` e `?`.
fn match_symbols<T: Copy + PartialEq>(
    p: &[T],
    t: &[T],
    low: impl Fn(&T) -> T,
    run: T,
    one: T,
) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    // Posizione dopo l'ultimo `*` e punto del testo da cui quel `*` riprende.
    let mut star: Option<usize> = None;
    let mut resume = 0usize;
    while ti < t.len() {
        if pi < p.len() && p[pi] == run {
            // Un `*` di fila a un altro non aggiunge nulla.
            while pi < p.len() && p[pi] == run {
                pi += 1;
            }
            star = Some(pi);
            resume = ti;
        } else if pi < p.len() && (p[pi] == one || low(&p[pi]) == low(&t[ti])) {
            pi += 1;
            ti += 1;
        } else if let Some(after_star) = star {
            // Il `*` assorbe un simbolo in più e si riprova da lì.
            resume += 1;
            ti = resume;
            pi = after_star;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn literal_and_case_insensitive() {
        assert!(wildcard_match("abc", "abc"));
        assert!(wildcard_match("ABC", "abc"));
        assert!(wildcard_match("Program Files", "program files"));
        assert!(wildcard_match("ÀÈÌ", "àèì"));
        assert!(!wildcard_match("abc", "abd"));
        assert!(!wildcard_match("abc", "ab"));
        assert!(!wildcard_match("ab", "abc"));
        assert!(wildcard_match("", ""));
        assert!(!wildcard_match("", "a"));
    }

    #[test]
    fn star_matches_any_run_including_empty() {
        assert!(wildcard_match("*", ""));
        assert!(wildcard_match("*", "anything"));
        assert!(wildcard_match("a*", "a"));
        assert!(wildcard_match("a*", "abc"));
        assert!(wildcard_match("*c", "abc"));
        assert!(wildcard_match("a*c", "abbbbc"));
        assert!(wildcard_match("a*b*c", "axxbyyc"));
        assert!(wildcard_match("AndroidStudio*", "androidstudio2024.1"));
        assert!(!wildcard_match("a*c", "abcd"));
        assert!(!wildcard_match("*x", "abc"));
        assert!(wildcard_match("**", "x"));
        assert!(wildcard_match("a**b", "ab"));
    }

    #[test]
    fn backtracking_finds_later_occurrences() {
        assert!(wildcard_match("*ab", "aab"));
        assert!(wildcard_match("*aab", "aaab"));
        assert!(wildcard_match("a*abc", "aabababc"));
        assert!(!wildcard_match("a*abd", "aabababc"));
        assert!(wildcard_match("*.log", "app.old.log"));
        assert!(!wildcard_match("*.log", "app.log.old"));
    }

    #[test]
    fn question_mark_matches_exactly_one() {
        assert!(wildcard_match("?", "a"));
        assert!(!wildcard_match("?", ""));
        assert!(!wildcard_match("?", "ab"));
        assert!(wildcard_match("down?oads", "downloads"));
        assert!(wildcard_match("a?c", "abc"));
        assert!(!wildcard_match("a?c", "ac"));
        assert!(wildcard_match("*?", "a"));
        assert!(!wildcard_match("*?", ""));
        assert!(wildcard_match("?*?", "ab"));
    }

    #[test]
    fn star_dot_star_is_literal_and_needs_a_dot() {
        assert!(wildcard_match("*.*", "a.txt"));
        assert!(wildcard_match("*.*", "."));
        assert!(wildcard_match("*.*", "archive.tar.gz"));
        assert!(!wildcard_match("*.*", "LICENSE"));
        assert!(!wildcard_match("*.*", ""));
    }

    #[test]
    fn many_stars_against_long_text_are_fast() {
        let pattern = format!("{}b", "*a".repeat(40));
        let text = "a".repeat(5000);
        let start = Instant::now();
        assert!(!wildcard_match(&pattern, &text));
        let forty = "*".repeat(40);
        assert!(wildcard_match(&forty, &text));
        let sparse = format!("{}x", "*".repeat(40));
        assert!(!wildcard_match(&sparse, &text));
        assert!(start.elapsed() < Duration::from_millis(50), "{:?}", start.elapsed());
    }
}
