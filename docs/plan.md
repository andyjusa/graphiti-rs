# Lightweight Graphiti Rust Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Deliver a lightweight Rust temporal-memory MCP server for Raspberry Pi in `andyjusa/graphiti-rs`.

**Architecture:** One bounded Tokio process connects to FalkorDB and external extraction APIs. Embeddings are disabled, external, or an optional local CPU backend. Domain operations are independent of MCP transport.

**Tech Stack:** Rust, Tokio, serde, reqwest with rustls, Redis protocol client for FalkorDB, rmcp for MCP, optional Candle/tokenizers CPU inference.

**Spec:** `../specs/2026-10-01-graphiti-rs-design.md`

## Global Constraints

- Raspberry Pi Linux ARM64 is the target; no unmeasured performance claims.
- Two ingestion jobs by default; candidate limits, request limits, and timeouts are mandatory.
- Local embedding model files are explicit and never downloaded silently.
- All operations are group-scoped; facts preserve time history and episode provenance.
- No production deployment, memory migration, or credential copying.
- Create the GitHub repository as private by default.

## Review Focus

- Malformed extraction must fail before any graph mutation.
- Concurrent retries of the same episode must not duplicate derived data.
- A fact supported by two episodes must survive deletion of one episode.
- Model changes must reject incompatible embedding spaces.
- Non-loopback HTTP without authentication must fail at startup.

### Task 1: Domain and configuration

**Files:** `Cargo.toml`, `src/lib.rs`, `src/config.rs`, `src/domain.rs`, `tests/domain.rs`.

**Interfaces:** `Config::from_env() -> Result<Config>`; `Extraction::validate() -> Result<()>`; `Fact::valid_at(DateTime<Utc>) -> bool`.

- [ ] Write failing tests for dangling entity references, empty content, validity boundaries, request limits, and unauthenticated public binding.
- [ ] Run `cargo test --test domain` and verify the missing behavior fails.
- [ ] Implement typed episodes, entities, facts, extraction requests, and validated configuration.
- [ ] Run domain tests and commit the domain foundation.

### Task 2: FalkorDB persistence and temporal ingestion

**Files:** `src/store.rs`, `src/ingest.rs`, `tests/store.rs`, `compose.yaml`.

**Interfaces:** `Store::ingest(&Episode, &Extraction) -> Result<IngestResult>`; `Store::episode(group, id) -> Result<Option<Episode>>`; `Store::delete_episode(group, id) -> Result<bool>`.

- [ ] Write integration tests for persistence, group isolation, concurrent idempotency, explicit replacement, and shared provenance deletion.
- [ ] Run against disposable FalkorDB and observe failures before implementation.
- [ ] Implement parameterized queries, atomic mutations, bounded connections, and stable group-scoped identity.
- [ ] Verify all store tests and commit persistence.

### Task 3: Extraction and optional embeddings

**Files:** `src/models.rs`, `src/embeddings.rs`, `src/embeddings/local.rs`, `tests/models.rs`.

**Interfaces:** `Extractor::extract(&Episode) -> Result<Extraction>`; `Embedder::embed(&[String]) -> Result<Vec<Vec<f32>>>`; provider metadata includes model identity and dimension.

- [ ] Write failing tests for API response validation, timeouts, embedding shape mismatch, missing local files, and incompatible model identity.
- [ ] Run model tests and verify failures.
- [ ] Implement an OpenAI-compatible extraction client and embedding client. Add optional Candle BERT CPU inference with explicit tokenizer/config/weight paths, attention-mask-aware pooling and normalization, and one bounded blocking worker.
- [ ] Run tests with and without the local feature. Run an explicit real-model smoke test if weights are available; report limitations otherwise.
- [ ] Commit model providers.

### Task 4: Search

**Files:** `src/search.rs`, `tests/search.rs`.

**Interfaces:** `Search::facts(SearchRequest) -> Result<Vec<FactHit>>`; `Search::entities(EntitySearchRequest) -> Result<Vec<EntityHit>>`.

- [ ] Write failing tests for reciprocal rank fusion, cosine dimension validation, historical filtering, group isolation, candidate bounds, and provenance output.
- [ ] Run search tests and verify failures.
- [ ] Implement bounded keyword, optional semantic reranking, graph expansion, and rank fusion. Document that bounded candidate scoring is not exhaustive vector retrieval.
- [ ] Run search and store tests and commit retrieval.

### Task 5: MCP service

**Files:** `src/mcp.rs`, `src/main.rs`, `tests/mcp.rs`.

**Interfaces:** `MemoryService` exposes add_episode, search_facts, search_nodes, get_episode, delete_episode, and get_status through stdio and Streamable HTTP.

- [ ] Write failing protocol tests for initialization, discovery, tool calls, invalid arguments, authentication, and disconnected database status.
- [ ] Run MCP tests and verify failures.
- [ ] Implement standard MCP transports, bearer protection, body limits, bounded ingestion admission, stderr-only diagnostics for stdio, and bounded shutdown.
- [ ] Verify protocol tests and an end-to-end ingestion/search example and commit the service.

### Task 6: Packaging and publication

**Files:** `README.md`, `LICENSE`, `.env.example`, `Dockerfile`, `.github/workflows/ci.yml`, `examples/`, design and plan copies.

- [ ] Document setup, model preparation, tool contracts, resource defaults, limitations, and source attribution.
- [ ] Add CI for formatting, Clippy, tests, and the optional local feature, plus disposable FalkorDB integration testing.
- [ ] Run `cargo fmt --check`, `cargo test`, `cargo test --features local-embeddings`, and `cargo clippy --all-targets --all-features -- -D warnings`.
- [ ] Build release and ARM64 container when a builder is available; distinguish build verification from actual Pi runtime measurements.
- [ ] Review the complete diff against the spec, resolve material issues, and commit.
- [ ] Create private `andyjusa/graphiti-rs`, push, verify the remote commit and report the repository URL and any unverified hardware checks.
