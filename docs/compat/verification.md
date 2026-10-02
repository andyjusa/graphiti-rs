# Compatibility verification — 2026-10-02

**Decision: no production replacement.** C01–C30 are acceptance categories, not independently passed tests. [Pi evidence](pi-results.md), [implementation limits](implementation.md) and [remaining validation plan](pi-validation.md) define the practical scope.

## Current evidence

- Local fmt, all-target/all-feature clippy and all-feature tests pass; 29 nonignored tests.
- Four differential tests execute 116 synthetic inputs against hashed selected Python0.28.2 bodies; fake driver/model decisions remain distinct from complete pipeline execution.
- [CI on staged Rust source af52674](https://github.com/andyjusa/graphiti-rs/actions/runs/36989372558) passes all 51 distinct executable tests and ARM64 default/local/compatibility release builds, using FalkorDB4.14.11.
- All 51 release tests pass on actual Pi4 with observed Redis8.2.3/module41606/image/module hashes, including actual CPU MiniLM. Current-artifact export is fixed in build helper commit fb919f6.
- [Staged ingestion](staged-ingestion.md): actual observed patched Python add_episode and actual Pi Rust agree after seven message/JSON/text/English/Korean synthetic episodes with 27 supplied model responses. Actual Pydantic schemas match; selected aliases, duplicate identity/provenance, contradiction discovery, temporal invalidation and summaries pass.
- [Fulltext follow-up](fulltext-results.md): existing indexes, DB score order, explicit construction-state errors, and complete chronological episode scan. Six queries produce identical actual Python SDK/Rust MCP fact/node UUID lists on 1200 synthetic records. An exclusively synthetic cold RDB restore retains the selected identity/endpoints and indexed MCP target.
- Historical HTTP/resource/SIGKILL/DB outage/AOF and vector-outlier results retain their [separate source and evidence](vector-retrieval.md).
- Wider full-pipeline cases, operational original-memory restore, real LLM/Gemini comparison, live gateway/cutover and rollback remain open. An initial private original-memory backup for restoration rehearsal is retained on the Pi; it is not a final drained-queue snapshot. The separate isolated DB loads the original snapshot successfully (72 graph groups; 8,839 nodes including 7,953 entities and 884 episodes; 6,886 facts). Full Rust reader/schema/vector integrity and provider/gateway evidence are still pending.

## Cleanup and historical scope

Earlier Mac resources graphiti-rs-compat-db/data/net were removed on 2026-10-02 after checking each exact task label. Earlier Mac/CI 4.14.11 results did not certify the actual Pi module. New Pi container/network/relay/private-root cleanup completed after preserving only synthetic evidence; no global Docker restart/prune is used.

## Acceptance coverage

`verified` covers only the stated synthetic/policy checks. `partial` names remaining work; `blocked` names a live integration prerequisite; `not_executed` names an operational gate.

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
| C11 | partial | Complete 128-row UUID-keyset episode scan, parsed RFC3339 ordering and global bound pass with 201 mixed-offset records across groups. 30-second deadline returns errors, not partial lists. Intentional improvement over lexical-window behavior; restored schemas/duplicate UUIDs/concurrent writers/full Python ordering parity untested. |
| C12 | partial | Observed MCP recipes exclude BFS. Global semantic ranking fixes the historical outlier. Existing operational fulltext indexes now supply keyword order to RRF; 116 selected-source inputs pass and six queries yield identical Python SDK/Rust MCP fact/node UUID lists on 1200 records. Missing/incomplete-index substring fallback and broad retrieval quality/tie parity remain gaps. |
| C13 | verified | Gated worker distinguishes queue acceptance from exact committed lookup. |
| C14 | verified | Concurrent same-UUID acceptance dispatches once; committed retry leaves original record intact. |
| C15 | verified | Changed same-UUID input fails closed. Observed Python loads old body/metadata instead of replacing it; strict rejection is intentional. |
| C16 | verified | Private WAL/FULL journal, single worker, strict prepare/completion transitions, uncertain model refusal and destructive tombstone history tested. |
| C17 | partial | Actual Pi committed/dispatch-ambiguous SIGKILL, DB outage/reconnection and AOF restart tests pass; source fixtures cover prepared/destructive recovery. Disk-full/OOM/runtime-error rollback and external writers remain untested. |
| C18 | verified | Invalid provider response is journaled as failed, produces no visible episode, and is not automatically replayed. |
| C19 | verified | UUIDs and independent created/valid/invalid/expired RFC3339 strings roundtrip without precision loss. |
| C20 | partial | Legacy attributes/labels/vecf32, existing UUID updates and relationship-first range-index reads pass on observed module41606 as well as CI 4.14.11. Restored production schema breadth remains untested. |
| C21 | partial | Actual MCP wrapper calls node.delete, not Graphiti.remove_episode. Synthetic store deletion retains entities/facts and dangling provenance; clear retains Saga/unknown labels. Journal pending recovery/tombstones added; production alias/schema behavior untested. |
| C22 | partial | Observed exact fact reuse/shared provenance and supplied duplicate/contradiction decisions match selected Python bodies. The default staged worker now wires entropy/MinHash/model entities, summary updates, source-specific extraction and contradiction discovery; seven actual Python SDK/Pi Rust synthetic episodes match. Wider/custom-model and provider quality cases remain open. |
| C23 | partial | Mock gpt-6-luna temperature=1/max_tokens<=4096/enable_thinking=false and observed decoder match. Observed staged prompts/language/input cleanup/schema suffix and actual Pydantic schemas match; bounded retry/timeout differ. The explicit combined preview remains a separate regression profile; no real generation comparison. |
| C24 | partial | Native Gemini 3072/count/finite/nonzero and LF-only preprocessing covered with dummy credentials. Batch limit 8 differs from observed 32; no numerical provider parity. |
| C25 | partial | Missing/wrong bearer, Host, Origin, oversized input and two protocol-version initializations tested; existing OAuth relay not exercised. |
| C26 | not_executed | Separate authenticated bridge/session/routing validation remains unperformed; the active replacement objective requires a verified gateway and rollback before cutover. |
| C27 | verified | Legacy connect/status/read create no native graph/index; indexed keyword reads preserve fixture index metadata. Index construction is explicit test ownership only; Python SDK constructor separately schedules fixture indexes. |
| C28 | partial | Same observed Pi image/Redis8.2.3/module41606/hash: 51 release tests pass. Exclusively synthetic RDB cold restore retains the selected 1200 fact identity/endpoints and indexed MCP target. Selected seven-episode full Python pipeline agrees with Rust; original-memory restore, wider ingestion and gateway remain open. |
| C29 | not_executed | Concrete gateway/reconnect/freeze/accounting/rollback gates documented; no routing/deployment/write freeze performed. |
| C30 | partial | Pi4/aarch64/3795MiB synthetic timings, RSS, CPU, overload and restart recorded with 1CPU/512MiB DB. 1200 keyword facts and four English local-model facts do not certify production budgets or multilingual/large-corpus quality. |
