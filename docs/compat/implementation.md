# Graphiti 0.28.2 compatibility preview

This optional feature adds the ten recorded MCP tool schemas and legacy-schema reads. **It is not ready to replace the Python server. Production writes, updates, deletion, and clearing are disabled.** The standalone implementation and its optional local embeddings remain available independently.

## Run and configuration

```sh
cargo build --release --locked --features graphiti-compat
COMPAT_DB_URL=redis://127.0.0.1:16379 \
  ./target/release/graphiti-rs --storage-format graphiti-0282 --transport stdio
```

The example URL denotes a disposable test DB. No database address is inferred. `COMPAT_WRITE_MODE` defaults to `read-only`; reads make no model calls and create no indexes. HTTP uses `/mcp`, the existing bearer/Host/Origin/request-size checks, `GRAPHITI_BIND`, and `HTTP_ALLOWED_HOSTS`. Compatibility HTTP credentials come from an owner-private regular file selected by `COMPAT_TOKEN_FILE`.

`synthetic` mode additionally requires a DB on loopback port 16379, loopback `COMPAT_LLM_BASE` and `COMPAT_GEMINI_BASE`, an explicit `COMPAT_MAX_TOKENS` of 1–4096, and an owner-private `COMPAT_JOURNAL_DIR`. It permits only new-episode ingestion through MCP. A loopback address alone does not prove a DB is disposable: inspect resource ownership before using this mode. Destructive store methods are for the isolated fixture harness; MCP clearing remains disabled because journal invalidation has not been certified.

The recorded LLM profile remains `gpt-6-luna`, with the FactChat base configurable through `COMPAT_LLM_BASE`. Separate `COMPAT_LLM_KEY_FILE` and `COMPAT_GEMINI_KEY_FILE` select private files; keys are absent from configuration Debug output, response errors, and request-body logs. Redirects and automatic model retries are disabled. The native embedding adapter uses `gemini-embedding-2:batchEmbedContents`, 3072 dimensions, batches of at most eight, and newline-to-space preprocessing. Count, dimensionality, finiteness, and nonzero-vector checks reject drift. These are mocked wire tests, not a certification of production generation limits or extraction equivalence. The compatibility embedding space cannot silently switch to the standalone local model.

## Storage and processing

Legacy group IDs map directly to graph keys. ACLs allow five named read groups and the recorded project-name pattern; `hermes` and `family_shared` remain read-only. Destructive requests require an explicit group and identity; invalid groups are rejected before querying or calling a model.

The reader handles `Episodic`, `Entity`, `RELATES_TO`, compact property maps, lists, and `vecf32`. It keeps UUIDs, RFC3339 strings with their original precision and offset, separate temporal fields, structural/property labels, and flat scalar/list attributes. Public DTOs omit embeddings; internal prepared records retain vectors for recovery. Unknown graph topology is left untouched. Unsupported property types fail closed; this is not a general graph export/import implementation.

The synthetic writer accepts only a complete graph of newly identified entities/facts. One guarded query creates the episode, facts, entities, and MENTIONS. Existing entity/edge UUIDs cause an error without inserting an episode. Entity names are never used as the storage identity. Model extraction is a small preview pipeline, not Python Graphiti's resolution, deduplication, or invalidation pipeline.

The private SQLite journal uses WAL/FULL synchronization and an exclusive worker lock. It records `queued`, `running`, `prepared`, `committed`, and `failed`. One serial worker processes accepted input; acceptance does not imply graph completion. Exact `get_episode` checks committed graph data. Uncertain model dispatches become failed on restart and are never automatically replayed. Prepared work can reconcile an already committed graph without another model call; reconciliation compares the episode, entities, facts, and vectors. Changed content for an existing UUID is rejected. Journal retention/compaction and operator reconciliation remain future work.

Search uses bounded keyword candidates, optional synthetic semantic candidates, deterministic ranking, and round-robin group selection. Candidate work is divided across groups. Read-only mode uses keywords only. Episode lists merge and sort a bounded per-group window; the initial DB selection orders stored timestamp strings. Mixed offsets or large corpora can therefore differ from Python's retrieval and global ordering. Full retrieval quality, temporal filtering, and baseline semantics remain release gates.

## Verified scope and remaining gates

See [verification and C01–C30 status](verification.md). Tests used synthetic text, new dummy credentials, local model mocks, and a newly created isolated FalkorDB. They did not read operational credentials, transfer user memories, call paid providers, or change a gateway/route/Pi container.

Before a production replacement, the patched Python baseline must establish extraction/update/deduplication/deletion/invalidation parity, generation limits, real schema/vector compatibility, and retrieval quality. The production module version must be tested on an isolated restore on the same Pi; actual restore success, gateway reconnect/rollback, accepted-work accounting, and Pi resource/latency budgets must all be verified. The existing combined Python/DB container must remain running during any future canary.

Primary references: [Graphiti v0.28.2 FalkorDB driver](https://github.com/getzep/graphiti/blob/v0.28.2/graphiti_core/driver/falkordb_driver.py), [Graphiti episode removal](https://github.com/getzep/graphiti/blob/v0.28.2/graphiti_core/graphiti.py), [FalkorDB compact scalar parser](https://github.com/FalkorDB/falkordb-py/blob/master/falkordb/query_result.py), and [Google native embedding API](https://ai.google.dev/api/embeddings). The sanitized recorded MCP schemas are in `tests/fixtures/production-mcp-contract.json`.
