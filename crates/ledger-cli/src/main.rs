#![forbid(unsafe_code)]

use std::env;
use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

use ledger_core::reducer::replay;
use ledger_storage::journal::read_journal;

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args();
    let _program = args.next();
    let action = args.next();
    let path = args.next();
    if args.next().is_some()
        || !matches!(action.as_deref(), Some("verify" | "replay"))
        || path.is_none()
    {
        return Err("usage: ledger-cli <verify|replay> <journal-path>".into());
    }

    let action = action.ok_or("missing command")?;
    let path = path.ok_or("missing journal path")?;
    let path = Path::new(&path);
    if !path.is_file() {
        return Err(format!(
            "journal must be an existing regular file: {}",
            path.display()
        )
        .into());
    }

    // read_journal is read-only. JournalWriter::open is intentionally NOT used:
    // it may recover a torn tail by modifying the underlying file.
    let journal = read_journal(path)?;
    if journal.recovered_truncated_tail {
        return Err("journal has a damaged final record; no repair was attempted".into());
    }
    // Verify both structural journal integrity and semantic reducer validity.
    let state = replay(&journal.records)?;

    match action.as_str() {
        "verify" => println!(
            "verified {} records; replay valid; journal unchanged",
            journal.records.len()
        ),
        "replay" => println!("{state:#?}"),
        _ => unreachable!("command validated above"),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("ledger-cli: {err}");
            ExitCode::FAILURE
        }
    }
}
