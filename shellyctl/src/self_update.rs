use std::process::ExitCode;

use axoupdater::AxoUpdater;

pub fn run() -> ExitCode {
    let mut updater = AxoUpdater::new_for("shellyctl");
    if let Err(e) = updater.load_receipt() {
        eprintln!(
            "self-update: not installed via a shellyctl installer ({e}); \
             upgrade via your package manager instead"
        );
        return ExitCode::from(2);
    }
    match updater.run_sync() {
        Ok(Some(result)) => {
            println!("updated to {}", result.new_version);
            ExitCode::SUCCESS
        }
        Ok(None) => {
            println!("already up to date");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("self-update failed: {e}");
            ExitCode::from(1)
        }
    }
}
