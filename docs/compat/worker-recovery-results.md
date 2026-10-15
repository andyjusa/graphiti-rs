# Worker completion failure and restart recovery

The compatibility worker could terminate after a journal error while `get_status` still reported a healthy DB and `add_memory` continued accepting jobs. The worker now owns a lifetime guard. Every worker exit marks it stopped; write tools check that state before and after acquiring the mutation lock. `get_status` reports an error with a restart instruction when the ingestion worker has stopped. Graph reads remain available, and read-only health responses retain their existing contract.

## Failure injection

`tests/support/worker_failure.rs` installs a real SQLite trigger in its private synthetic journal. The trigger aborts completion updates after the graph commit, leaving the accepted job in `prepared`. No production test hook is added. Before the fix, the new test failed with `dead ingestion worker still reports healthy`.

After the fix, the test asserts that:

- The committed episode remains readable while health reports the stopped worker.
- All four write tools refuse new work; the journal still contains only the first accepted job.
- Removing the synthetic fault and restarting the engine reconciles `prepared` to `committed`.
- LLM and embedding request counts remain unchanged during refusals, restart and a same-UUID replay. The replay also verifies the original durable receipt.
- Health returns to `ok`; the owned synthetic graph and journal are removed after validation.

The bounded fixture initially makes three mock LLM requests and four mock embedding requests. Restart makes zero requests of either kind. All endpoints are loopback mocks; no real provider is invoked.

## Validation

Local checks passed: formatting, all-target/all-feature Clippy, 30 non-ignored tests and all 11 disposable-DB compatibility tests. Local DB verification uses FalkorDB 4.14.11. The ARM64 release test binary is built from the same source with all features. Actual Pi results and source/binary hashes are recorded in [the sanitized evidence](pi-worker-recovery-20261002/summary.json).

The Pi fixture uses an empty disposable DB with the original runtime's pinned FalkorDB module, an internal Docker network, and an owned loopback relay. Credentials and operational configuration are absent; Python relay execution clears the environment and disables site initialization. DB storage uses bounded temporary memory, including an explicit `/data` mount. The fixture starts with zero DB keys and finishes with two keys: one empty graph (zero nodes and edges) and its observed `telemetry{…}` key; it then removes its container, network and relay. It rechecks the original service and gateway container identities/start times, the read-only restored service, and initial backup/audit hashes.

To reproduce locally, use an exclusively owned disposable FalkorDB listening on loopback port 16379:

```sh
COMPAT_TEST_DB_URL=redis://127.0.0.1:16379 cargo test --locked --all-features --test compat worker_failure::stopped_worker_refuses_new_jobs_and_reconciles_prepared_commit_on_restart -- --ignored --exact --nocapture
```

This validates one Rust durability failure path, not exhaustive Python/Rust fault parity. It does not enable operational writes, certify fresh OAuth login or provider quality, or complete operational replacement. An in-flight request accepted before worker termination remains a durable job for restart recovery; the guard refuses work once termination is observed. No automatic provider retry is introduced.
