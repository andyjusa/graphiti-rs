# Lightweight Graphiti for Raspberry Pi

## Goal and scope

Create `andyjusa/graphiti-rs`: a native Rust memory server for Raspberry Pi that stores and searches temporal knowledge and exposes MCP tools. The user selected a server replacement, and clarified that low resource usage on Raspberry Pi is the defining goal. This is a focused implementation inspired by Graphiti, not a claim of complete Python API or database compatibility.

The initial delivery includes source, meaningful tests, GitHub CI, a container build suitable for ARM64, configuration examples, and English usage documentation. Create the repository as private by default. Do not deploy to the existing Pi or migrate production memories as part of this task.

## Architecture

One Tokio process contains the memory core, FalkorDB adapter, model clients, and MCP transport. Reuse FalkorDB, which is already used by the user's Pi deployment. Keep memory and service concurrency bounded; avoid a second vector database, Python runtime, local generative LLM inference, background community generation, and unbounded ingestion queues. Embedding providers are selectable: disabled, external API, or optional local inference.

Use small focused modules for configuration, domain models, database queries, ingestion, search, model requests, and MCP. The core exposes typed operations and does not depend on the transport. Database and model interfaces permit deterministic tests without paid API calls.

## Stored data and ingestion

Episodes retain original content, source metadata, reference time, group ID, and creation time. Entities retain stable group-scoped SHA-256 IDs, group IDs, names, and summaries. Fact embeddings are stored when configured; entity embeddings are omitted to reduce memory and model calls. Facts connect entities and retain fact text, provenance episode IDs, creation time, valid-from time, and optional valid-to time.

An OpenAI-compatible external API extracts structured entities and facts from episode content. Validate returned fields and reject malformed relationships before writing. Entity matching is group-scoped and uses normalized names initially. Facts may supersede an existing fact only when extraction explicitly identifies that relationship as replaced; unrelated facts must not be invalidated automatically. Historical facts remain queryable.

Store an episode and its derived graph in a database transaction where supported; otherwise use one atomic parameterized Cypher mutation. Idempotent ingestion accepts a caller-supplied episode ID and must not duplicate its graph on retry. Network and model failures return explicit errors, never a successful ingestion result.

## Retrieval

Provide keyword retrieval first, plus optional embedding similarity and bounded graph expansion. Fuse independent result ranks with reciprocal rank fusion. The initial vector path may score a bounded candidate set in Rust; document that this is not exhaustive vector retrieval. Bound candidate counts and result limits through validated configuration.

Apply group isolation to all reads and writes. Support current facts and an explicit as-of timestamp using validity intervals. Return facts with entity names, timestamps, and episode provenance. Embeddings are optional so basic operation does not require embedding costs or local inference.

### Optional local embeddings

Provide an optional Rust-native CPU embedding backend, enabled through a Cargo feature and selected through configuration. Load its model only when the local provider is selected. Accept explicit model and tokenizer paths; document a reproducible model preparation procedure. Do not download weights silently at startup. Use bounded batches and a single inference worker by default, keeping blocking inference off Tokio's async executor. External API embeddings and disabled embeddings remain available in builds without the local feature.

Store the embedding model identity and vector dimension with the graph's embedding configuration. Reject incompatible vectors; switching models requires an explicit re-embedding operation rather than mixing spaces. Missing model files or inference failures return actionable errors; never silently substitute a different provider. Verify a real local-model inference and ARM64 build before claiming Raspberry Pi support for this optional backend. Document model disk size and measured memory usage when those measurements are available.

## MCP and operational interface

Implement standard MCP initialization, tool discovery, and tool invocation with stdio for local clients and Streamable HTTP for remote clients. Document actual supported protocol versions and avoid claiming compatibility with the Pi's custom server/discover handshake without verifying it.

Tools cover episode ingestion, fact search, entity search, episode retrieval, episode deletion, and status. Document tool schemas and differences from Python Graphiti. HTTP binds to loopback by default; require a configured bearer token for non-loopback use. Credentials come from environment variables and never enter source control or logs.

Deletion removes episode provenance and deletes derived facts only when no other episode supports them. Status distinguishes process availability from database connectivity. Shutdown closes listeners and outstanding work gracefully with a bounded timeout.

## Raspberry Pi constraints

Target Linux ARM64 first; document ARMv7 support only after an actual build. Supply multi-stage Docker packaging and conservative defaults: two ingestion jobs, limited search candidates, model timeouts, request-size limits, and bounded connection pools. No performance numbers may be advertised without measurements. Record host test results separately from actual Pi measurements.

## Verification and acceptance

Unit tests cover extraction validation, rank fusion, temporal filtering, and configuration bounds. Integration tests exercise ingestion, idempotency, group isolation, historical facts, and deletion against a disposable FalkorDB instance where available. MCP tests verify initialize/discovery/call behavior and HTTP authentication. Live model tests are opt-in.

Run formatting, Rust compilation, tests, and Clippy before pushing. Verify ARM64 container builds when a builder is available; otherwise explicitly report that verification limitation. A runnable example demonstrates ingesting an episode and retrieving its facts. The README clearly states the initial feature set and limitations.

## Existing deployment boundary

Read-only inspection of existing public tool schemas may guide compatibility. Keep the existing Pi service, production memories, local plugin credentials, and private marketplace outside the repository. Deployment and data migration require a separate explicit request.
