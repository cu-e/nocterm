//! `nocterm plugin check <dir>`: the author's check of user plugin packages.
use std::{ffi::OsString, io, path::PathBuf};

const USAGE: &str = "usage: nocterm plugin check <directory>";

/// Runs the subcommand with the arguments after `plugin`; the result is the exit status.
pub(crate) fn run(mut args: impl Iterator<Item = OsString>) -> i32 {
    match (args.next(), args.next(), args.next()) {
        (Some(command), Some(directory), None) if command == "check" => check(directory.into()),
        _ => {
            say(&mut io::stderr(), USAGE);
            2
        }
    }
}

fn check(directory: PathBuf) -> i32 {
    let catalog = match crate::shell_setup::plugin_catalog() {
        Ok(catalog) => catalog,
        Err(error) => {
            let text = format!("the application's plugin catalog is invalid: {error}");
            say(&mut io::stderr(), &text);
            return 1;
        }
    };
    let report = crate::shell_setup::check_directory(&directory, &catalog);
    say(&mut io::stdout(), &report.to_string());
    if report.passed() { 0 } else { 1 }
}

/// Writes a line to the terminal. A closed pipe has nobody to tell, so a failure is ignored.
fn say(out: &mut impl io::Write, text: &str) {
    let _ = writeln!(out, "{}", text.trim_end());
}
