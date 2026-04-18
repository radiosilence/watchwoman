use std::process::ExitCode;

use watchwoman::cli;

fn main() -> ExitCode {
    match cli::run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("watchwoman: {e:#}");
            ExitCode::from(1)
        }
    }
}
