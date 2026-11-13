//! `xiox`: everything on this PC that belongs to one project, from the command line.
//!
//! A console program with an `asInvoker` manifest: it never asks for elevation.

mod args;
mod json;
mod mcp;
mod observe;
mod render;
mod run;
mod size;
#[cfg(test)]
mod testdata;

fn main() {
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    // stderr is not locked for the whole run: a worker thread (or the panic hook) that writes to it
    // while the main thread holds the lock would wait for ever. Each write locks it briefly.
    let code = run::run(&args, &mut std::io::stdout().lock(), &mut std::io::stderr());
    std::process::exit(code);
}
