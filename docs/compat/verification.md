# Compatibility verification — 2026-10-01

The implementation is a preview. Synthetic success is not production cutover approval or Python-equivalent Graphiti behavior. [Implementation limits](implementation.md) remain release gates.

## Executed checks

- `cargo fmt --check` and `cargo clippy --offline --locked --all-targets --all-features -- -D warnings`.
- `cargo test --offline --locked --all-features`: 22 unit/provider/policy tests passed, including six compatibility tests. Optional real-model tests were not repeated in this compatibility change.
- `TEST_FALKORDB_URL=redis://127.0.0.1:16379 cargo test --offline --locked --test store --test mcp -- --ignored`: nine existing standalone integration tests passed.
- `COMPAT_TEST_DB_URL=redis://127.0.0.1:16379 cargo test --offline --locked --features graphiti-compat --test compat -- --include-ignored`: eight compatibility tests passed, including two DB/HTTP integrations.

These runs cover 33 distinct tests. C01–C30 below are acceptance categories, not 30 independently passed executable tests. The fixture's original design statuses remain unchanged; results are separately recorded in [results.json](results.json).

The disposable DB used a newly created internal network and labelled volume/container, loopback model mocks, no operational mounts, Redis 8.2.2 and FalkorDB 4.14.11. This differs from the recorded production module. Its image ID was `sha256:3e3870883d8cc4a20171d59fbafeb7cf29b57c09d71c29287fdacf4fedb8a078`. It ran as UID 999 with one CPU/512 MiB, a read-only root filesystem, all capabilities dropped, and no-new-privileges. Owner-private dummy key files verified separate LLM/Gemini headers; no actual key was read.

Docker Desktop could not reach a published port on the internal network. Tests retained that network and used a temporary loopback relay through `docker exec` into the owned container. No production network was attached. This relay invalidates latency comparisons; none are claimed. Temporary credential/journal directories and the relay were cleaned up after validation. Docker became unavailable before container/network/volume cleanup; those three labelled resources remain pending cleanup. No global Docker restart or unrelated resource deletion was attempted. Once the daemon is running, verify the `task=graphiti-rs-compat-20261001` labels, then remove only `graphiti-rs-compat-db`, `graphiti-rs-compat-data`, and `graphiti-rs-compat-net`.

An earlier working-tree snapshot also built as a Linux ARM64 Docker image with `graphiti-compat` and ran `--help`. That image predates final journal/clear/response safeguards and is not a deployment artifact. CI now builds the final branch with the compatibility feature on ARM64; its final status must be checked separately. No actual Raspberry Pi measurement or production-module restore was performed.

## Acceptance coverage

`verified` describes the stated synthetic/policy evidence only. `partial` identifies an incomplete acceptance category. `blocked` identifies a missing baseline or live integration prerequisite; `not_executed` identifies operations outside this work's scope.

| Case | Status | Evidence and limitation |
| --- | --- | --- |
| C01 | verified | Exact ten-tool metadata, structured/text layers, status golden text, and protocol error separation. |
| C02 | verified | Exact absent UUID lookup returns found=false/episode=null, without a model request. |
| C03 | verified | Synthetic legacy episode returns original metadata and precise timestamp without embeddings. |
| C04 | partial | Identical UUIDs in user_primary/general_chat stay separate; all real groups/restored records untested. |
| C05 | partial | Read-only ACL policy and rejected additions tested; complete per-tool zero-I/O instrumentation pending. |
| C06 | partial | Unknown-group policy and addition rejection tested; complete per-tool zero-I/O instrumentation pending. |
| C07 | verified | Approved project-name pattern and invalid suffixes tested at the policy boundary. |
| C08 | verified | Null/empty/multiple/duplicate destructive groups rejected; valid MCP clear remains disabled. |
| C09 | verified | Required group enforced by shared Key shape; missing group produces protocol error. |
| C10 | verified | Deduplicated multi-group search and deterministic round-robin/global maximum tested with mocks. |
| C11 | partial | Global bound and sorting covered on small fixture; bounded lexical DB window is not full timestamp ordering parity. |
| C12 | partial | Entity labels and centered facts tested; Python graph expansion/center-distance semantics unverified. |
| C13 | verified | Gated worker distinguishes queue acceptance from exact committed lookup. |
| C14 | verified | Concurrent same-UUID acceptance dispatches once; committed retry leaves original record intact. |
| C15 | verified | Changed body under the same UUID fails closed before extraction. |
| C16 | verified | Private durable journal, single-worker lock, uncertain dispatch refusal, prepared vector/time recovery tested. |
| C17 | verified | Committed DB write with prepared journal recovers to committed without another model dispatch. |
| C18 | verified | Invalid provider response is journaled as failed, produces no visible episode, and is not automatically replayed. |
| C19 | verified | UUIDs and independent created/valid/invalid/expired RFC3339 strings roundtrip without precision loss. |
| C20 | partial | Structural labels, flat scalar/list attributes, vecf32, MENTIONS and retained Saga/Community links tested; real schema breadth pending. |
| C21 | blocked | Patched Python provenance/orphan deletion baseline unavailable; all MCP legacy deletions disabled. |
| C22 | blocked | Invalidation, shared provenance, resolution, and supersession baseline parity not established. |
| C23 | partial | Mock gpt-6-luna/model endpoint/body and bounds tested; production generation limits and extraction pipeline not matched. |
| C24 | partial | Native Gemini batch/count/3072/newline validation and separate private dummy-file headers tested; production profile numerical parity not tested. |
| C25 | partial | Missing/wrong bearer, Host, Origin, oversized input and two protocol-version initializations tested; existing OAuth relay not exercised. |
| C26 | blocked | No actual bridge/session/routing operation authorized or performed. |
| C27 | verified | Legacy connect/status/read create no native graph/index; isolated graph indexes remain empty. |
| C28 | not_executed | No Pi backup, restore, operational data transfer, or production module test performed. |
| C29 | not_executed | No write freeze, route switch, deployment, rollback, or live journal reconciliation performed. |
| C30 | not_executed | No actual Raspberry Pi RSS/CPU/latency benchmark performed. |
