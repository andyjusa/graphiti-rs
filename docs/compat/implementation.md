# Graphiti 0.28.2 compatibility preview

The optional feature exposes the ten recorded MCP tools and legacy-schema reads. **Production mutations and real provider calls remain disabled. It is not ready to replace the operational Python server.** The independent lightweight server and its optional local embeddings remain available on `main`.

## Configuration

```sh
cargo build --release --locked --features graphiti-compat
COMPAT_DB_URL=redis://127.0.0.1:16379 \
  ./target/release/graphiti-rs --storage-format graphiti-0282 --transport stdio
```

The example DB is disposable. The address must be explicit. `COMPAT_WRITE_MODE=read-only` is the default: reads make no model calls and create no indexes. HTTP uses `/mcp`, bearer/Host/Origin/request-size checks, `GRAPHITI_BIND`, `HTTP_ALLOWED_HOSTS`, and the private credential file `COMPAT_TOKEN_FILE`.

`synthetic` mode requires loopback DB port 16379, loopback LLM/Gemini mocks, an explicit `COMPAT_MAX_TOKENS` of 1–4096 and a private `COMPAT_JOURNAL_DIR`. It enables new episode ingestion, existing entity/fact UUID reuse, and journaled deletion/clearing **only in an exclusively owned disposable DB**. Loopback alone is not proof of ownership. No production write mode exists.

ACLs authorize five recorded named read groups and the project-name pattern; `hermes` and `family_shared` stay read-only. Default group is `user_primary`. Duplicate read groups preserve first occurrence; an explicit empty group list returns empty without a model call. Destructive calls require an explicit UUID/group or exactly one raw clear group, and never inherit a default.

## Observed baseline

[Production metadata](observed-production.json) and the [excerpt manifest](../../tests/fixtures/python-0282/manifest.json) record the actual running 0.28.2 package/runtime patch hashes. Source/version collection was read-only; no graph data, environment or credentials were exported. The stdlib Python runner verifies excerpt hashes, removes type annotations and executes selected original bodies with synthetic DTOs, mocked model decisions, a fake driver and a frozen clock. It never imports operational startup modules.

Three previous assumptions were corrected:

- The MCP `delete_episode` wrapper calls `episode.delete(driver)`, **not** `Graphiti.remove_episode`. It detaches matching node UUIDs with Entity/Episodic/Community labels; remaining entities/facts and their provenance arrays are not cleaned up. Edge deletion similarly leaves episode `entity_edges` arrays intact. Rust adds explicit property-group predicates; malformed mixed-group records/UUID aliasing are not certified.
- Scoped clear removes only Entity/Episodic/Community nodes in that group. Saga and unknown labels remain; their links to removed nodes detach.
- An existing episode UUID loads the **old** episode metadata/body in Python. It does not replace the body supplied to the tool. Rust refuses changed input, and avoids repeated model dispatch on identical committed retries. Stable accepted timestamps also deliberately differ from Python's worker-time reference timestamp.

## Writes and journal

Relationship-first fact reads match the observed runtime patch for bound-node edge-index omissions. Queries derive and validate both Entity endpoints before filtering; reads still create no indexes. Tests explicitly create the relationship range index only in their owned fixture graph. Actual indexed reads and global semantic queries pass on the observed Pi module in isolated synthetic fixtures; restored original-schema breadth remains a gate.

Readers preserve original UUIDs, timestamp strings/precision/offset, independent temporal fields, flat scalar/list attributes, structural/property labels and `vecf32` vectors. Public DTOs omit embeddings. Unsupported shapes and ambiguous exact UUIDs fail closed.

The worker's bounded exact resolution reuses a unique normalized entity name and an exact fact with the same directed endpoints; shared episode provenance appends once. Ambiguous names/facts or exhausted candidate windows refuse resolution. Existing entity summaries/labels/attributes are retained in this fast path. **Python fuzzy/model entity resolution, summary updating, custom relation typing, complete extraction prompts and contradiction-candidate discovery are not ported.** A separate deterministic resolver matches observed duplicate-index/temporal-invalidation rules with supplied mocked decisions; it is not wired to infer contradictions in ingestion. Generation parity remains a release gate.

The store can update explicitly supplied existing entity/fact UUIDs, preserve original `created_at`, retain unknown flat properties and structural labels, append shared provenance, and reject fact endpoint retargeting. A durable prepared plan holds exact before-state properties/labels/endpoints and stable MENTIONS UUIDs. One guarded query checks every before-state predicate before any upsert and inserts the new episode plus MENTIONS. Recovery checks the full prepared after-state and vectors/mentions. Property races refuse the write without creating an episode. A single guarded query is not evidence of disk-full/OOM/runtime-error rollback; those fault-injection gates remain open.

SQLite uses private files, WAL/FULL and an exclusive worker lock. Queue phases are queued/running/prepared/committed/failed. Completion requires durable preparation; uncertain model dispatch becomes failed on restart and never automatically calls the model again. Exact committed receipt checks tolerate later legitimate shared-provenance updates while still checking identities, supporting episode and MENTIONS. Changed episode body remains rejected.

Destructive work records a durable pending operation before graph changes, blocks ingestion to the group, and refuses groups with pending or failed prepared jobs. Recovery repeats only that idempotent synthetic operation before processing jobs, then marks receipt history deleted/cleared/stale. Deleting a fact conservatively makes all committed receipts in that group stale; episode rows can remain present. Tombstoned UUIDs cannot replay automatically. Journal history is retained rather than dropped. Destructive recovery assumes a single writer to the disposable DB; retention/compaction, external-writer recovery and production operator reconciliation are not certified.

## Search and providers

Observed MCP node/fact recipes use BM25 + cosine, RRF rank constant 1, and semantic score strictly above 0.6. They do **not** enable BFS. Centered fact search uses RRF first, groups facts by **source** UUID and orders sources as center/direct undirected RELATES_TO neighbor/disconnected. At the observed reranker score 0, disconnected candidates remain. Center search requires exactly one effective group. Both the active legacy fallback (the Falkor driver leaves `search_interface` unset) and the newer operation body are compared with synthetic inputs. Rust implements these ranking rules, type filtering and round-robin multi-group merging.

Candidate retrieval still differs for keywords: bounded substring matching replaces Python fulltext retrieval. Semantic node/fact queries rank every supported vector in the selected group before applying the candidate limit, using the observed Falkor normalized score `(2 - vec.cosineDistance(...))/2` strictly above 0.6. Node type filtering happens before ranking; relationship reads retain the deployed relationship-first workaround. Unsupported vector types/dimensions fail the query instead of returning a partial ranked set. Read-only mode is keyword-only. Episode lists sort a bounded lexical timestamp window before global parsed sorting; mixed offsets/large corpora can differ. Full retrieval quality/recall and corpus-window behavior remain gates. Other Graphiti recipes with BFS are outside these recorded MCP defaults.

The mock wire profile uses `gpt-6-luna`, `temperature=1`, max tokens at most 4096, JSON-object output and `enable_thinking=false`. Decoding handles observed fences/prefixes, first complete JSON and exact schema/value envelopes. The extraction prompt is still the preview prompt. FactChat base is configurable; LLM and Gemini keys come from separate private files. Redirects, transport retries and automatic structured-output retries are disabled; bounded timeout is at most 120s. These differ deliberately from the observed Python retry/900s LLM timeout profile.

Native Gemini uses `gemini-embedding-2:batchEmbedContents`, 3072 dimensions and LF-only replacement (`\n` to space, preserving `\r`). Batches are at most 8 rather than Python's 32. Count/dimensions/finite/nonzero validation and separate mock headers are tested. Actual provider numerical/generation parity is untested. The legacy embedding space cannot switch silently to the independent server's local embedding model.

## Release gates

See [verification](verification.md), [results.json](results.json), and the [same-Pi validation plan](pi-validation.md). Remaining gates include full patched-Python ingestion comparison, restored schema/vector compatibility, real provider comparison, actual Pi resources/persistence/failure behavior, and separate gateway/cutover/rollback validation. The production container, Dot/Graphiti tools, authentication and routes were not modified.
