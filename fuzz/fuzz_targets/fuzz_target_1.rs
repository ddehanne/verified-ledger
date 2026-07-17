//! Contract under fuzz: parse_journal_bytes(arbitrary bytes) never
//! panics — it returns Ok (with or without tail recovery) or a typed
//! JournalError. Any panic/abort here is a bug by definition.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = ledger_storage::journal::parse_journal_bytes(data);
});
