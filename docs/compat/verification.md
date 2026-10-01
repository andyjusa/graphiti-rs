# Compatibility verification — 2026-10-01

Decision: **no production replacement**. Evidence below separates implementation, synthetic validation, same-Pi preparation and actual cutover. C01–C30 are acceptance categories, not 30 independently passed tests. [Implementation limits](implementation.md) and [Pi validation plan](pi-validation.md) remain release gates.

## Current evidence

- Local `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings` and `cargo test --all-features` passed; 26 nonignored tests passed. DB and actual local-model tests are ignored locally because Docker is unavailable.
- Two differential tests run 34 synthetic inputs against selected observed Python 0.28.2 bodies. Source excerpt hashes are checked; fake DTO/driver/model decisions and frozen time are used. This is not full Python pipeline, graph-driver, numerical provider or transport parity.
- [CI on source commit `d4fac944`](https://github.com/andyjusa/graphiti-rs/actions/runs/36819906853) passed: three compatibility DB/HTTP integrations, nine independent-server DB/HTTP integrations, the source-function tests, and two real local MiniLM tests. The test log contains **40 distinct passing executable test names** across the feature runs. ARM64 release builds passed for default, local embeddings and compatibility. This uses FalkorDB4.14.11, not the observed production module.
- Same-Pi image/module/source/binary metadata was read-only verified. `compose.compat-pi.yaml config --format json` rendered the isolated configuration correctly; no test container was created on the Pi. No original memories/secrets were transferred, no provider call was made, and no operational container/gateway/auth/route was changed.

## Historical evidence and cleanup

The previous preview commit `728c4ec` passed its 33 local distinct tests and GitHub checks; those results do not certify newly changed code. Its disposable Mac DB used Redis8.2.2/FalkorDB4.14.11, UID999, one CPU/512MiB, internal networking, read-only root, dropped capabilities and new dummy credentials. Image ID: `sha256:3e3870883d8cc4a20171d59fbafeb7cf29b57c09d71c29287fdacf4fedb8a078`.

Docker Desktop could not reach the internal network's published port, so a temporary loopback relay through the owned container was used. The relay and dummy files were removed; no latency comparison is claimed. Docker then became unavailable. Owned container `graphiti-rs-compat-db`, volume `graphiti-rs-compat-data` and network `graphiti-rs-compat-net` remain pending cleanup. Verify each exact `task=graphiti-rs-compat-20261001` label before removing only these resources when Docker returns. No global restart/prune was attempted.

An earlier ARM64 Docker build predates the final guards and is not a deployable artifact. ARM64 CI is a build gate only; the actual same-Pi module/persistence/resource/restore/gateway/rollback gates remain unexecuted.

## Acceptance coverage

`verified` refers only to the stated synthetic/policy evidence. `partial` identifies remaining parity/schema/resource work. `blocked` identifies a live integration prerequisite. `not_executed` identifies an operational gate outside this change.

| Case | Status | Evidence and limitation |
| --- | --- | --- |
| C01 | verified | Exact ten-tool metadata, structured/text layers, status golden text, and protocol error separation. |
| C02 | verified | Exact absent UUID lookup returns found=false/episode=null, without a model request. |
| C03 | verified | Synthetic legacy episode returns original metadata and precise timestamp without embeddings. |
| C04 | partial | Identical UUIDs in user_primary/general_chat stay separate; all real groups/restored records untested. |
| C05 | partial | Read-only ACL policy and rejected additions tested; complete per-tool zero-I/O instrumentation pending. |
| C06 | partial | Unknown-group policy and addition rejection tested; complete per-tool zero-I/O instrumentation pending. |
| C07 | verified | Approved project-name pattern and invalid suffixes tested at the policy boundary. |
| C08 | verified | Explicit raw clear target enforcement and durable destructive ledger tested; only synthetic mode permits valid clear. |
| C09 | verified | Required group enforced by shared Key shape; missing group produces protocol error. |
| C10 | verified | Default/empty/duplicate read groups and round-robin merging matched observed helper behavior; empty groups call no models. |
| C11 | partial | Global bound and sorting covered on small fixture; bounded lexical DB window is not full timestamp ordering parity. |
| C12 | partial | Observed MCP recipes exclude BFS. RRF=1, source-node center ordering, single-group center rule and type filters tested; full BM25/vector retrieval quality differs. |
| C13 | verified | Gated worker distinguishes queue acceptance from exact committed lookup. |
| C14 | verified | Concurrent same-UUID acceptance dispatches once; committed retry leaves original record intact. |
| C15 | verified | Changed same-UUID input fails closed. Observed Python loads old body/metadata instead of replacing it; strict rejection is intentional. |
| C16 | verified | Private WAL/FULL journal, single worker, strict prepare/completion transitions, uncertain model refusal and destructive tombstone history tested. |
| C17 | partial | Full prepared after-state/vector/stable-MENTIONS and lost graph-completion recovery passed synthetic DB/worker tests. Durable destructive pending retry/tombstones passed; disk-full/OOM/runtime-error rollback and other writers remain untested. |
| C18 | verified | Invalid provider response is journaled as failed, produces no visible episode, and is not automatically replayed. |
| C19 | verified | UUIDs and independent created/valid/invalid/expired RFC3339 strings roundtrip without precision loss. |
| C20 | partial | Legacy scalar/list attributes, labels, vecf32 and existing entity/fact UUID updates preserve created_at/topology in synthetic DB test; restored schema breadth untested. |
| C21 | partial | Actual MCP wrapper calls node.delete, not Graphiti.remove_episode. Synthetic store deletion retains entities/facts and dangling provenance; clear retains Saga/unknown labels. Journal pending recovery/tombstones added; production alias/schema behavior untested. |
| C22 | partial | Observed exact fact reuse/shared provenance and supplied duplicate/contradiction decisions match selected Python bodies. Fuzzy/model entity resolution, summary updates, complete extraction and contradiction discovery remain unported. |
| C23 | partial | Mock gpt-6-luna temperature=1/max_tokens<=4096/enable_thinking=false and observed decoder match. Preview extraction prompt and bounded retry/timeout differ; no real generation comparison. |
| C24 | partial | Native Gemini 3072/count/finite/nonzero and LF-only preprocessing covered with dummy credentials. Batch limit 8 differs from observed 32; no numerical provider parity. |
| C25 | partial | Missing/wrong bearer, Host, Origin, oversized input and two protocol-version initializations tested; existing OAuth relay not exercised. |
| C26 | blocked | No actual bridge/session/routing operation authorized or performed. |
| C27 | verified | Legacy connect/status/read create no native graph/index; isolated graph indexes remain empty. |
| C28 | not_executed | Actual Pi image/Redis8.2.3/module41606/source/binary hashes read-only verified. Isolated same-Pi compose/runbook prepared; no backup/restore or current-module test executed. |
| C29 | not_executed | Concrete gateway/reconnect/freeze/accounting/rollback gates documented; no routing/deployment/write freeze performed. |
| C30 | not_executed | Actual Pi4/aarch64/3795MiB metadata observed; resource/workload/latency/restart budget plan prepared but no benchmark executed. |
