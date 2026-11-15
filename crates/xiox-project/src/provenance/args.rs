//! Parameter parsing of simple commands: POSIX options and PowerShell parameters.

use std::collections::{HashMap, HashSet};

use super::command::Arg;

/// Positional arguments and options of a POSIX command.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Posix {
    pub positional: Vec<String>,
    /// `None` is a switch.
    pub options: HashMap<String, Option<String>>,
}

/// Splits `args` into positionals and options; `with_value` lists the options that take the next
/// word. `--` ends the options, `--key=value` carries its value inline.
pub fn posix(args: &[Arg], with_value: &[&str]) -> Posix {
    let mut parsed = Posix::default();
    let mut words = args.iter().filter_map(Arg::text);
    while let Some(word) = words.next() {
        if word == "--" {
            parsed.positional.extend(words.map(str::to_owned));
            break;
        }
        match word.split_once('=') {
            Some((key, value)) if word.starts_with("--") => {
                parsed.options.insert(key.into(), Some(value.into()));
            }
            _ if word.starts_with('-') && word.len() > 1 => {
                let value = if with_value.contains(&word) {
                    words.next().map(str::to_owned)
                } else {
                    None
                };
                parsed.options.insert(word.into(), value);
            }
            _ => parsed.positional.push(word.into()),
        }
    }
    parsed
}

/// The parameters of a PowerShell command.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PsParams {
    /// Each positional argument is a comma-separated array of one or more values.
    pub positional: Vec<Vec<String>>,
    pub named: HashMap<&'static str, Vec<String>>,
    /// Lowercase names of the parameters seen without a value, including value parameters left
    /// without one and names matching no parameter or several.
    pub switches: HashSet<String>,
}

/// Parses PowerShell parameters. `params` pairs each accepted lowercase name or alias with its
/// canonical key; a name is resolved exactly or as a prefix that points to a single key.
pub fn powershell(args: &[Arg], params: &[(&str, &'static str)]) -> PsParams {
    let mut parsed = PsParams::default();
    let mut i = 0;
    while i < args.len() {
        let Arg::Text(word) = &args[i] else {
            i += 1;
            continue;
        };
        let Some(rest) = parameter_name(word) else {
            let (values, next) = take_array(args, i);
            parsed.positional.push(values);
            i = next;
            continue;
        };
        let (name, inline) = rest.split_once(':').unwrap_or((rest, ""));
        let name = name.to_lowercase();
        let key = resolve(params, &name);
        match key {
            Some(key) if !inline.is_empty() => {
                parsed.named.insert(key, vec![inline.into()]);
                i += 1;
            }
            Some(key) if matches!(args.get(i + 1), Some(Arg::Text(_))) => {
                let (values, next) = take_array(args, i + 1);
                parsed.named.insert(key, values);
                i = next;
            }
            _ => {
                parsed.switches.insert(name);
                i += 1;
            }
        }
    }
    parsed
}

/// The text after the dash when `word` is a parameter name (`-Path`, not `-1` or `-`).
fn parameter_name(word: &str) -> Option<&str> {
    let rest = word.strip_prefix('-')?;
    rest.starts_with(|c: char| c.is_alphabetic())
        .then_some(rest)
}

/// The canonical key of `name`: an exact name or alias, else the only key it is a prefix of.
fn resolve(params: &[(&str, &'static str)], name: &str) -> Option<&'static str> {
    if let Some((_, key)) = params.iter().find(|(alias, _)| *alias == name) {
        return Some(key);
    }
    let mut keys = params
        .iter()
        .filter(|(alias, _)| alias.starts_with(name))
        .map(|(_, key)| *key);
    let first = keys.next()?;
    keys.all(|key| key == first).then_some(first)
}

/// The values of the array starting at `start` (`a , b , c`) and the index after it.
fn take_array(args: &[Arg], start: usize) -> (Vec<String>, usize) {
    let mut values = Vec::new();
    let mut i = start;
    while let Some(Arg::Text(value)) = args.get(i) {
        values.push(value.clone());
        i += 1;
        match (args.get(i), args.get(i + 1)) {
            (Some(Arg::Comma), Some(Arg::Text(_))) => i += 1,
            _ => break,
        }
    }
    (values, i)
}
