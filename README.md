# verified-ledger

A small, aggressively verified order/execution ledger core in Rust.

**Thesis:** a trading ledger records *external evidence*, not internal
intentions. Late fills, duplicates, overfills and contradictory reports
must be representable and classified — never made "impossible" by types,
never silently dropped. Correctness is enforced by construction where it
belongs (arithmetic, ordering, transitions) and by quarantine where it
doesn't (integrity conflicts).

This is not an execution engine and connects to no broker. It is the
correctness core such an engine would sit on.

A companion C++20 project (private, pre-v0.1) carries the same contract
into live-execution engineering — durable journaling with an fsync
dispatch-eligibility barrier, broker reconciliation, IBKR TWS API — and
will be published as a sanitized mirror at v0.1.

## Design laws

- **LAW-01** — the core is pure: no clock, no RNG, no IO, no ambient
  identity. IDs and timestamps are validated *data* from outside.
- **Evidence over intention** — anomalies (fill-before-ack, fill during
  cancel, late fill after terminal, overfill) are representable,
  classified (`TransitionClass::Anomalous`), and quarantined
  (`IntegrityConflict`) with evidence preserved.
- **Sequence is the ordering authority** — timestamps are evidence;
  the contiguous `SequenceNumber` chain is law, enforced at apply time
  and re-verified at read time.
- **INV-1** — an `ExecutionId` has economic effect at most once.
- **INV-17** — undeclared transitions fail with a typed error and
  provably zero mutation of the authoritative reducer state.
- **Crash contract** — a torn final journal line is treated as a
  recoverable crash artifact: recover the longest valid prefix and heal
  the trailing fragment. Any integrity failure before the final line is
  treated as fatal; the system does not infer, repair, or silently discard
  historical evidence.

## Architecture
crates/ledger-core      types, events, transition table, pure reducer
crates/ledger-storage   hash-chained append-only durable journal (`fsync`)
crates/ledger-cli       (minimal; replay/verify)
fuzz/                   libFuzzer target on the byte-level parser

State rebuild = `replay(journal)`. Purity ⇒ determinism ⇒
crash-recovery-by-replay is sound (property-tested: prefix + suffix
equals full replay).

## Verification ladder (all reproducible)

| Level | Tool | Claim | Reproduce |
|---|---|---|---|
| 1 | `cargo test` | 32 example tests | `cargo test --workspace` |
| 2 | proptest | 3 properties on arbitrary journals | same |
| 3 | Miri | zero UB on all exercised paths | `cargo +nightly miri test -p ledger-core --lib` |
| 4 | libFuzzer+ASAN | 17.4M runs, 0 crashes on `parse_journal_bytes` | `cargo +nightly fuzz run fuzz_target_1 -- -max_total_time=120` |
| 5 | Kani (CBMC) | 3 bounded proofs, exhaustive in-domain | `cargo kani -p ledger-core` |

Kani proofs: money arithmetic total & exact over **all** `i64` pairs
(`i128` oracle); sequence successor law over **all** `u64`; transition
table over the **full** state×event product — never panics,
no-mutation-on-error.

Details: [`docs/VERIFICATION.md`](docs/VERIFICATION.md).

## Non-goals (V1, frozen)

Multi-currency, broker connectivity, typestate on authoritative state,
async, persistence beyond a single NDJSON journal.

## License

MIT
