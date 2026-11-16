//! The three shell dialects the lexer understands, as one strategy each.

/// The shell a command was written for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
    Bash,
    PowerShell,
    Cmd,
}

impl Shell {
    /// The lexical rules of this shell.
    pub fn dialect(self) -> &'static dyn Dialect {
        match self {
            Shell::Bash => &Bash,
            Shell::PowerShell => &PowerShell,
            Shell::Cmd => &Cmd,
        }
    }
}

/// What the lexer needs to know about a shell: its quoting, its escapes and its operators.
pub trait Dialect: Sync {
    /// The character that escapes the next one outside quotes.
    fn escape(&self) -> Option<char>;
    /// Whether single quotes quote at all (in PowerShell `''` inside them is a literal `'`).
    fn single_quotes(&self) -> bool;
    /// Whether the escape character escapes `next` inside double quotes.
    fn escapes_in_double(&self, next: char) -> bool;
    /// The operators, longest first, so the first prefix match is the longest one.
    fn operators(&self) -> &'static [&'static str];
    fn shell(&self) -> Shell;
    /// D13: whether `@'` or `@"` ending its line opens a here-string: one word up to a line starting `'@` or `"@`.
    fn here_strings(&self) -> bool {
        false
    }
}

struct Bash;
struct PowerShell;
struct Cmd;

impl Dialect for Bash {
    fn escape(&self) -> Option<char> {
        Some('\\')
    }
    fn single_quotes(&self) -> bool {
        true
    }
    fn escapes_in_double(&self, next: char) -> bool {
        matches!(next, '"' | '\\' | '$' | '`')
    }
    fn operators(&self) -> &'static [&'static str] {
        &["&&", "||", "|&", "&>", ">>", ">&", "<<-", "<<", ";", "|", "&", ">", "<", "(", ")"]
    }
    fn shell(&self) -> Shell {
        Shell::Bash
    }
}

impl Dialect for PowerShell {
    fn escape(&self) -> Option<char> {
        Some('`')
    }
    fn single_quotes(&self) -> bool {
        true
    }
    fn escapes_in_double(&self, _next: char) -> bool {
        true
    }
    fn operators(&self) -> &'static [&'static str] {
        &["&&", "||", ">>", ">&", ";", "|", ">", "<", "(", ")", "{", "}", ","]
    }
    fn shell(&self) -> Shell {
        Shell::PowerShell
    }
    fn here_strings(&self) -> bool {
        true
    }
}

impl Dialect for Cmd {
    fn escape(&self) -> Option<char> {
        Some('^')
    }
    fn single_quotes(&self) -> bool {
        false
    }
    fn escapes_in_double(&self, _next: char) -> bool {
        false
    }
    fn operators(&self) -> &'static [&'static str] {
        &["&&", "||", ">>", ">&", "&", "|", ">", "<", "(", ")"]
    }
    fn shell(&self) -> Shell {
        Shell::Cmd
    }
}
