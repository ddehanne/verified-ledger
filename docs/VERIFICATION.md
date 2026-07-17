# Verification Evidence

## Level 1 — Example tests
32 unit/integration tests across the workspace (see crates' test files).

## Level 2 — Property tests (proptest)
3 properties on arbitrary event journals (256 cases each per run):
replay determinism (byte-identical), position = sum of distinct applied
executions (independent recomputation), prefix+suffix = full replay.

## Level 3 — Interpreter (Miri, nightly)
`cargo +nightly miri test -p ledger-core --lib` → 18 passed
`cargo +nightly miri test -p ledger-core --test reducer_invariants` → 6 passed
Zero undefined behavior on all exercised paths (unsafe is forbidden
workspace-wide; Miri confirms the interpreter sees none).

## Level 4 — Coverage-guided fuzzing (cargo-fuzz / libFuzzer + ASAN)
Target: `parse_journal_bytes(&[u8])` — the disk-facing attack surface.
Contract: arbitrary bytes never panic (Ok with recovery, or typed error).
Result: 17,461,635 runs in 121s, zero crashes, zero leaks.

## Level 5 — Bounded model checking (Kani) — see kani proofs
