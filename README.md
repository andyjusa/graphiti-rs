# graphiti-rs

Lightweight Rust temporal graph memory for Raspberry Pi, with MCP over stdio or Streamable HTTP. Built for a small always-on machine: one Rust process, an existing FalkorDB instance, bounded work, and optional embeddings.

This is an independent focused implementation inspired by [Graphiti](https://github.com/getzep/graphiti), not a complete port or a drop-in replacement for its Python APIs, database schema, or custom MCP extensions. The default standalone mode uses its own schema; use a separate graph name for an existing Python Graphiti database.

An optional `graphiti-compat` feature adds a **Graphiti 0.28.2 compatibility preview** with ten recorded MCP tool schemas and a legacy-schema reader. Production mutations are disabled. This preview has isolated synthetic tests, but requires further Python baseline, restored-database, gateway, and Raspberry Pi validation before replacing an existing server. See [compatibility details](docs/compat/implementation.md) and [indexed-search validation](docs/compat/fulltext-results.md).

## Features

- Original episodes, named entities, relationships, provenance, and half-open validity intervals.
- Idempotent episode IDs within groups; explicit fact replacement preserves historical facts.
- Keyword search, optional semantic ranking, reciprocal rank fusion, and bounded one-hop graph expansion.
- OpenAI-compatible external extraction; caller-supplied structured extraction works without an LLM.
- Embeddings: **disabled**, **external API**, or **optional local CPU inference** in Rust.
- Standard MCP initialization and six tools; no Python runtime in the service.

## Quick start

Rust 1.95 is the tested toolchain. The manifest declares 1.88; that minimum has not been separately verified. FalkorDB 4.14.11 is tested in CI; the observed 4.16.6 module passed isolated Pi tests.

```sh
cp .env.example .env
# Edit .env: set a long random GRAPHITI_TOKEN.
# MODEL_API_KEY is only needed for automatic extraction or external embeddings.
docker compose up -d --build
# Export the same token for the demonstration client:
export GRAPHITI_TOKEN='your-token-from-.env'
python3 examples/demo.py
```

Compose binds both services to host loopback, stores FalkorDB data in a named volume, and enables append-only persistence. The example uses explicit extraction and disabled embeddings, so it needs no paid model API. Reusing the example's episode ID returns `inserted:false`.

To reuse an existing database instead:

```sh
cargo build --release --locked
export FALKORDB_URL=redis://127.0.0.1:6379
export GRAPHITI_GRAPH=graphiti_rs
# Use stdio with an MCP client:
./target/release/graphiti-rs --transport stdio
# Or serve HTTP:
./target/release/graphiti-rs --transport http
```

The binary reads exported environment variables; it does not load `.env` itself. Diagnostics go to stderr. Redis TLS URLs are not enabled in this initial build; keep FalkorDB on a trusted local network, use its ACL/authentication where needed, or tunnel the connection.

## Optional local embeddings

The default build includes no Candle/tokenizer dependencies. Enable the feature explicitly:

```sh
python3 scripts/download-minilm.py
cargo build --release --locked --features local-embeddings
export EMBEDDING_PROVIDER=local
export EMBEDDING_MODEL=sentence-transformers/all-MiniLM-L6-v2
export EMBEDDING_DIM=384
export LOCAL_MODEL_DIR="$PWD/models/minilm"
./target/release/graphiti-rs --transport http
```

The preparation script explicitly downloads a pinned revision; the running server never downloads weights. The [MiniLM model](https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2) is an English sentence encoder. Its weights occupy about 91 MB on disk; this is not the server's RAM footprint. For Korean, supply a suitable multilingual BERT-format sentence encoder and verify its quality and Raspberry Pi resource cost separately.

The supported local format contains `config.json`, `tokenizer.json`, and `model.safetensors` with Candle-compatible BERT weights. Pooling uses the attention mask, mean aggregation, and L2 normalization, with 256-token truncation. Other architectures and pooling conventions are not automatically supported. One local inference worker processes one sentence at a time on the CPU; no GPU or local generative LLM is required. `MODEL_API_BASE` can also point to an independently operated local OpenAI-compatible embedding service using `EMBEDDING_PROVIDER=external`.

For Compose, set `CARGO_FEATURES=local-embeddings`, `EMBEDDING_PROVIDER=local`, `EMBEDDING_MODEL=sentence-transformers/all-MiniLM-L6-v2`, and `EMBEDDING_DIM=384` in `.env`, then rebuild. The model directory mounts read-only at `/models`.

Embedding identity and dimension are pinned per group. External identity includes the endpoint; local identity hashes weights, config, tokenizer, and inference conventions. Keep external model aliases immutable at the endpoint. Changing providers/models requires a new group or an explicit offline migration; there is no automatic re-embedding tool. Existing facts ingested with embeddings disabled remain keyword-only until migrated. Model failures return errors and never silently change provider.

## MCP tools

Endpoint: `/mcp`. Standard `initialize`, `tools/list`, and `tools/call` are implemented by the Rust MCP SDK. Protocol `2025-11-25` is tested; newer versions are negotiated by the SDK. The Pi-specific `server/discover` handshake is not implemented.

| Tool | Arguments | Result |
| --- | --- | --- |
| `add_episode` | `group_id`, `content`, `reference_time`, optional `episode_id`, `source`, `extraction` | Episode ID, insertion flag, fact IDs |
| `search_facts` | `group_id`, `query`, optional `limit` (1–50), `as_of` | Ranked facts with validity and provenance |
| `search_nodes` | `group_id`, `query`, optional `limit` | Entities matching normalized names |
| `get_episode` | `group_id`, `episode_id` | Original episode or null |
| `delete_episode` | `group_id`, `episode_id` | Whether an episode was deleted |
| `get_status` | none | Process status, database connectivity, embedding identity |

Tools return JSON text in MCP content blocks and `isError:true` on operation failures. Groups isolate operations, but the server's single bearer token grants access to every group; group IDs are not a multi-user authorization boundary.

`reference_time` and `as_of` are RFC 3339 timestamps (database comparisons use millisecond precision). Facts are valid from their reference time, inclusive, until replacement time, exclusive. Omit `as_of` to search current facts. To replace a fact, put its returned ID in the new fact's `supersedes` array. Replacement must occur later than the old fact and cannot change an existing invalidation to a different time.

Episode deletion removes its support; facts survive while another episode supports them. Deleting a replacement does not resurrect old facts. Entity names/summaries use their first occurrence, and orphan entities are retained. Facts with different reference times have separate identities, including repeated text. Entity search is a current name lookup and does not apply `as_of`.

## Resource defaults

| Setting | Default | Bound |
| --- | --- | --- |
| `SEARCH_JOBS` | 2 | 1–8; shared across stdio and HTTP, excess search fails promptly |
| `INGEST_JOBS` | 2 | 1–8; excess ingestion fails promptly for retry |
| `SEARCH_CANDIDATES` | 200 | 50–2000 per retrieval branch |
| `REQUEST_TIMEOUT_SECS` | 30 | 1–120 for model API calls |
| `EMBEDDING_PROVIDER` | disabled | disabled / external / local |
| `GRAPHITI_BIND` | 127.0.0.1:8080 | Non-loopback binding requires `GRAPHITI_TOKEN` |
| HTTP admission | 16 | Excess active requests return 503 |
| HTTP request body | 256 KiB | Larger bodies rejected |
| Episode content | 64 KiB | Explicit input validation |
| Local sequence length | 256 tokens | Truncated before inference |

Database commands have a six-second client deadline and five-second query execution timeout. External embeddings use batches of at most eight. Search fetches a recent candidate window plus up to four independently bounded keyword windows. Semantic search ranks all eligible DB vectors before returning the bounded top candidates, independently of those windows. New facts materialize a native vector alongside the existing JSON DTO. Older JSON-only vectors remain searchable through 128-row keyset pages with a 30-second overall semantic deadline; timeout returns an error instead of partial results. This fallback is read-only and can be slower on large old corpora. Graph expansion is limited to the fetched candidates. Keyword matching is normalized substring matching, not BM25. Storage volume, FalkorDB index/query memory, and local-model memory still grow with data/model size. Bounded [Pi4 synthetic measurements](docs/compat/vector-retrieval.md) are recorded separately; they do not establish a production budget.

HTTP bearer authentication applies when a token is configured, including loopback. `HTTP_ALLOWED_HOSTS` is a comma-separated host allowlist; configure your reverse-proxy hostname for remote use. Browser origins are rejected by default. Put TLS in your reverse proxy; the service itself speaks HTTP. The CLI uses two async workers and up to four blocking threads. Shutdown handles Ctrl-C/SIGTERM with a ten-second transport drain and a ten-second runtime shutdown deadline; a running local CPU forward pass cannot be interrupted cleanly.

## Verification

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo test --features local-embeddings
# Start a disposable test database, separate from production:
docker run -d --name graphiti-rs-test -p 127.0.0.1:16379:6379 falkordb/falkordb:v4.14.11
cargo test --test store --test mcp -- --ignored
LOCAL_TEST_MODEL_DIR=models/minilm cargo test --features local-embeddings --test models real_local_model -- --ignored
```

Live paid API tests are not required; API contract tests use a local HTTP server. CI tests optional local inference and builds on Linux ARM64. See [verification record](docs/verification.md) for actual observed results. Isolated Pi4 ARM64 release tests and local CPU inference passed; see [Pi evidence](docs/compat/pi-results.md). ARMv7 and existing production Pi deployment are not verified. Community generation, ontology learning, full Graphiti API parity, and production-data migration are outside this release.

## License

Apache-2.0. Original Rust implementation; Graphiti's architecture is credited above. Downloaded models retain their own licenses and are excluded from Git.
