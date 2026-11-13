//! `xiox`: everything on this PC that belongs to one project, from the command line.
//!
//! A console program with an `asInvoker` manifest: it never asks for elevation.

mod args;
mod json;
mod render;
mod run;
#[cfg(test)]
mod testdata;

fn main() {
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    let code = run::run(&args, &mut std::io::stdout().lock(), &mut std::io::stderr().lock());
    std::process::exit(code);
}
