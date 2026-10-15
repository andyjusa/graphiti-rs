# Raspberry Pi operating cutover — 2026-10-03

The existing memory/MCP upstream now runs the ARM64 Rust compatibility server. The previous Python ingestion container is stopped, its restart policy is disabled, and ordinary Compose startup excludes it through a legacy profile. Its original data directory and image remain available. The Rust server and separate FalkorDB have persistent private directories and restart policies; neither publishes a host port.

The deployed code is commit `d97fe1a4f7d481510778240da150f29642ca1c2a`. The Rust binary SHA256 is `9e5849094ec47685f7f952dc4da22d2c199b9800dbd6fc65912a7e400ff2b255`. Documentation-only commits after that point do not change the operating binary. The database retains the observed FalkorDB4.16.6 / Redis8.2.3 module and the existing raw-text3072-dimensional embedding space. Independent standalone mode still supports optional local CPU embeddings.

## Preserved data and switch

Intake was blocked before stopping the old writer. Its complete retained logs contained no episode-worker processing events; the frozen checkpoint confirmed no in-flight ingestion. A final native snapshot was saved privately, and the stopped service's full data directory was copied into a separate operating directory, including AOF and embedding-profile metadata.

Bounded, paginated comparisons verified all72 original graphs,8839 nodes and19293 relationships. Node/edge identities, labels, properties, vectors and unknown records match. The only excluded equality checks concern the native `telemetry{graph}` streams: read-only queries add their own query-history entries. Their strict key/type/schema pattern was verified, and they remain in the native backups. No other opaque Redis key was excluded.

The initial unpaged comparison exhausted maintenance-process memory headroom and SSH became unavailable. The host recovered after a restart. The corrected comparator pages32 records, limits its own address space to384MiB and handles parent death; staging and the final operating comparison then passed. Earlier failed diagnostics are retained privately.

Ingress now routes `/mcp` to Rust and retains the validated backend Authorization header. The reviewed gateway patch forwards repeated `Mcp-Method` and `Mcp-Name` values unchanged. Existing OAuth SQLite state and credentials are retained. The ten original tool schemas have SHA256 `623add94c2569f39915bbd12c36db2f6fbc250c6a751c625c229e34896fcd129`.

## Operating checks

- Both installed Graphiti connector paths returned healthy status through the switched service. The OAuth connector read two original episodes and an exclusively owned Korean test episode created after the switch.
- Direct ingress and the existing stateless bridge passed modern discovery, exact ten-tool schemas, status and original episode reads. Direct legacy2025-11-25 initialization/list/status passed; initialization identifies the server as `graphiti-rs`.
- Twelve operating refusal cases passed: missing/mismatched/duplicate principal headers and missing/invalid authentication on the direct, bridge and OAuth paths. Existing OAuth discovery metadata remains available. Earlier isolated gateway tests also cover expired credentials, Origin rejection and differing/identical duplicate forwarding.
- Two controlled English/Korean episodes committed through the actual operating ingress. Fresh embeddings retrieved the expected facts and entities in each language. Both journal entries reached `committed`; acceptance alone was not treated as completion.
- The operating DB, Rust worker, OAuth gateway and bridge were restarted. Both controlled episodes persisted, identical submissions returned committed receipts without another generation dispatch, original episodes remained readable and OAuth state row counts were preserved. Active OAuth connector status passed again after restart.
- A separate bounded provider probe completed two generation requests and one four-text embedding batch. All vectors had3072 dimensions. English paraphrase cosine was0.9632 versus0.2699 for an unrelated sentence. This is a small synthetic quality cohort.

After post-switch write/search requests, Rust used6.762MiB and the DB442.3MiB. These are Docker memory snapshots, not peak RSS or throughput measurements. Final local checks passed32 ordinary tests,15 isolated DB compatibility tests, formatting, all-target/all-feature Clippy and Compose validation. Both push and PR CI passed for the deployed code, including actual optional local CPU inference and the ARM builds.

## Recovery

Retain the old container, original data directory, final native snapshot, configuration backups and private journal. The current-data recovery script is kept with the private operating checkpoint. It freezes intake, refuses unreconciled accepted work, stops Rust, saves the current DB, preserves the old directory, restores the current complete durable state into the Python service's path, restores its gateway/ingress configuration, and waits for old-service readiness before reopening intake. This preserves memories committed after the switch.

A native RDB alone omits the file-based embedding-profile marker. Preserve the existing `gemini-embedding-2/active-profile.json` alongside native data; otherwise the old startup attempts reembedding. The isolated rehearsal blocked external network access and exposed that missing-metadata failure before its restoration. After restoring that metadata, the exact previous Python service started with its original configuration in an isolated container. Its legacy session initialization, healthy status, ten-tool schemas, two original episode reads and both post-switch Rust episode reads passed. No provider network access was possible. The actual recovery procedure copies the full directory, including the marker. Fresh GitHub OAuth login and a live switch back to Python were not performed. Wider custom schemas, exhaustive overlapping-write/fault parity, disk-full/OOM behavior and general model quality remain outside this bounded adoption evidence.

The owned test group and its two native Redis keys were removed after committed journaled cleanup. Final original graph/node/edge counts and144 Redis keys match the baseline. All owned staging/rehearsal containers were removed; the prior Python container and original data remain preserved. A final synchronous DB snapshot briefly delayed a connector health check; the save completed and both active connectors returned healthy afterward. Prefer a background or native-replication backup for online maintenance.

See the [aggregate report](pi-cutover-20261003/summary.json). Private reports contain original-memory fingerprints and detailed runtime diagnostics. Public evidence contains aggregates only; original memories, credentials, OAuth grants and native snapshots are excluded from Git and CI.
