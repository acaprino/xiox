use crate::provenance::{Arg, posix, powershell};

fn text(words: &[&str]) -> Vec<Arg> {
    words
        .iter()
        .map(|word| {
            if *word == "," {
                Arg::Comma
            } else {
                Arg::Text((*word).into())
            }
        })
        .collect()
}

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).into()).collect()
}

const PARAMS: [(&str, &str); 7] = [
    ("path", "path"),
    ("literalpath", "path"),
    ("destination", "dst"),
    ("dest", "dst"),
    ("itemtype", "itemtype"),
    ("it", "itemtype"),
    ("pattern", "filter"),
];

#[test]
fn posix_values_switches_and_double_dash() {
    let parsed = posix(&text(&["-m", "755", "-p", "a", "--", "-b"]), &["-m"]);
    assert_eq!(parsed.positional, strings(&["a", "-b"]));
    assert_eq!(parsed.options.get("-m"), Some(&Some("755".to_string())));
    assert_eq!(parsed.options.get("-p"), Some(&None));
    assert_eq!(parsed.options.len(), 2);
}

#[test]
fn posix_inline_values_and_dangling_value_option() {
    let parsed = posix(&text(&["--out=dist", "-m"]), &["-m"]);
    assert_eq!(parsed.options.get("--out"), Some(&Some("dist".to_string())));
    assert_eq!(parsed.options.get("-m"), Some(&None));
    assert!(parsed.positional.is_empty());
}

#[test]
fn powershell_prefix_alias_inline_and_arrays() {
    let cases: [(&[&str], &str, &[&str]); 5] = [
        (&["-Dest", r"D:\x"], "dst", &[r"D:\x"]),
        (&["-it", "Directory"], "itemtype", &["Directory"]),
        (&[r"-Path:D:\a"], "path", &[r"D:\a"]),
        (&["-Path", "a", ",", "b"], "path", &["a", "b"]),
        (&["-Lit", "x"], "path", &["x"]),
    ];
    for (args, key, expected) in cases {
        let parsed = powershell(&text(args), &PARAMS);
        assert_eq!(parsed.named.get(key), Some(&strings(expected)), "{args:?}");
    }
    let parsed = powershell(&text(&["-Recurse", "a", ",", "b", "c"]), &PARAMS);
    assert!(parsed.switches.contains("recurse"));
    assert_eq!(
        parsed.positional,
        vec![strings(&["a", "b"]), strings(&["c"])]
    );
}

#[test]
fn powershell_value_param_without_value_is_a_switch() {
    let parsed = powershell(&text(&["-Path"]), &PARAMS);
    assert!(parsed.named.get("path").is_none());
    assert!(parsed.switches.contains("path"));
}

#[test]
fn powershell_ambiguous_prefix_is_a_switch() {
    let parsed = powershell(&text(&["-P", "x"]), &PARAMS);
    assert!(parsed.named.is_empty());
    assert!(parsed.switches.contains("p"));
    assert_eq!(parsed.positional, vec![strings(&["x"])]);
}
