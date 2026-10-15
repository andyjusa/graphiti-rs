# Latest upstream comparison — 2026-10-15

Compared Rust `main` at `77e1b49cec3a482cc5a570fac50faba72f8678a5` with
[getzep/graphiti main](https://github.com/getzep/graphiti/tree/aca369364226a666ca541b061ff2a1a54b80f2ff)
at `aca369364226a666ca541b061ff2a1a54b80f2ff` (0.31.0). The previous observed
Python baseline remains v0.28.2, `77b16096baf755d6c6102f4b50a3725d3e7f7ac7`.
The upstream range contains 235 commits. This is a selected behavioral backport,
not a claim of complete Python 0.31 API, prompt, or model-quality parity.

The operating goal remains a lightweight Raspberry Pi server. The ten-tool MCP
contract, explicit group authorization, journal/CAS checks, existing graph schema,
and `gemini-embedding-2` / 3072 through the Luna gateway are retained. No migration,
re-embedding, new model calls, or Python runtime is required by these changes.
Optional independent-server local embeddings remain available.

## Applied changes

| Change | Upstream source | Rust behavior and protection |
| --- | --- | --- |
| Exact-name resolution before entropy | [dedup helpers](https://github.com/getzep/graphiti/blob/aca369364226a666ca541b061ff2a1a54b80f2ff/graphiti_core/utils/maintenance/dedup_helpers.py), [#1964](https://github.com/getzep/graphiti/pull/1964) | Unique short/Korean/Chinese exact matches resolve without an extra deduplication call. Ambiguous exact names remain unresolved. |
| Same-episode duplicate folding | [node operations](https://github.com/getzep/graphiti/blob/aca369364226a666ca541b061ff2a1a54b80f2ff/graphiti_core/utils/maintenance/node_operations.py) | Normalized equal names with compatible classifications collapse before candidate retrieval. Strictly more specific classifications win. Conflicting specific classifications stay distinct rather than choosing upstream's label-count tie breaker. |
| Canonical type promotion | Same | A strict superset of specific labels can enrich an existing identity; UUID, creation time and unknown attributes survive. The original DB snapshot is captured before promotion so concurrent edits still fail CAS. |
| Normalized edge endpoints | Same | Case/whitespace spelling differences resolve to canonical UUIDs. Ambiguous names never select an arbitrary endpoint. Unknown/ambiguous endpoints retain the existing skip policy. |
| Future assertion bounds | [temporal helper](https://github.com/getzep/graphiti/blob/aca369364226a666ca541b061ff2a1a54b80f2ff/graphiti_core/utils/maintenance/temporal_edge_utils.py) | A future plan starts at the statement reference time; a soft deadline is not an invalidation. Explicit ongoing-state endings remain. The staged pipeline uses the same English patterns as upstream; this does not add a Korean temporal-language heuristic. Historical timestamp spellings are not rewritten. |
| Backticks and empty lexical queries | [#1475](https://github.com/getzep/graphiti/pull/1475), [#1590](https://github.com/getzep/graphiti/pull/1590) | Backticks delimit terms. Stopword/punctuation-only input produces an empty lexical branch while semantic retrieval continues. Too-long input remains distinct, preserving the existing bounded substring fallback on unindexed graphs. ACL validation still runs first. |
| Saga clear | [#1688](https://github.com/getzep/graphiti/pull/1688) | Explicit scoped clear includes Saga nodes. Other groups and unknown node labels remain protected. This does not enable saga generation. |

`regex` is optional and enabled only with `graphiti-compat`; it was already a
locked transitive dependency for optional local inference. The independent
server's default dependency feature selection does not gain regex.

## Reviewed differences retained or deferred

- Upstream driver/group-routing fixes do not require a mutable shared driver in
  Rust: each store query selects its group explicitly. Relationship-first reads
  already derive endpoints directly, matching the relevant edge-search optimization.
- New MCP tools and options (saga summaries, communities, triplets, episode entity
  queries, reference-time and temporal/type filters, scalar/default groups) expand
  the public API or workload. They are not silently enabled in the ten-tool profile.
- Combined C2 extraction, LLMRuntime, additional provider clients, custom structured
  attributes/edges and prompt changes need a separately validated extraction/model
  policy. The pinned staged prompts and bounded calls remain.
- BFS and additional rerankers are outside the recorded MCP RRF recipes. Saga and
  community generation would add work that is unsuitable to enable implicitly on Pi.
- Recursive NUL stripping is not adopted: silently rewriting input can disagree
  with journal digests and prepared replay. Existing boundary validation remains;
  no claim is made that every unknown attribute/source field rejects NUL.
- Upstream fulltext group escaping is not copied speculatively over the existing
  quoted, allowlisted expression. A disposable FalkorDB 4.14.11 experiment
  compared raw quoted, escaped quoted and escaped bare underscore-group queries:
  all returned the same sole in-group record (score 1.5), excluding a cross-group
  decoy. The fixture graph was deleted afterward. No DB upgrade or index creation
  is added to runtime reads.
- Python dependency/security changes and FalkorDB 6 index-DDL changes do not map to
  Rust runtime dependencies or the current no-read-side-index-creation contract.

## Verification and reproducibility

Old v0.28.2 fixtures remain immutable. Unchanged prompt, similarity, temporal
resolution, fulltext, UUID and destructive-scope observations continue to be
checked against them. Short exact-name resolution, lexical-empty behavior and
Saga deletion intentionally supersede selected old expectations; historical Pi
reports document their original runs and are not rewritten as new evidence.

A separately pinned full upstream `temporal_edge_utils.py` plus SHA-256 manifest
supports a stdlib-only AST oracle. `future_temporal_bounds_match_pinned_latest_upstream`
compares 13 cases by timestamp instant, including deadlines, explicit ends, past
bounds, offsets, absent reference time and nonmatching language. Python is used
only by tests, never the server.

The synthetic DB regression
`exact_names_collapse_promote_and_normalize_endpoints_with_original_snapshot`
uses local mock providers and a disposable graph. It checks actual storage/CAS,
classification promotion, duplicate folding, endpoint spelling, temporal pipeline
wiring and rejection after a real concurrent mutation. Search/clear regressions
cover indexed and unindexed lexical-empty behavior, semantic continuation, long
queries, backticks, scoped Saga deletion and unknown-label preservation.

Validation results are recorded after completion below. No operating production
DB, credentials, or paid model calls are used in this verification.

### Local results

- `cargo fmt --check`: pass.
- `cargo clippy --all-targets --all-features -- -D warnings`: pass.
- `cargo test --all-features`: 44 passed; 31 environment-dependent tests ignored.
- The 16 compatibility DB tests passed on disposable FalkorDB 4.14.11. The first
  attempt ran before the DB image download finished and failed connections. After
  readiness, 14 passed immediately; two old empty-query assertions were adjusted
  to the intended new contract and both then passed. The audit test still checks
  full-page scoped retrieval with a searchable term rather than empty input.
- The reformatted upstream temporal oracle was rerun independently: pass.
- Standalone MCP/store disposable-DB tests: 13 passed (6 MCP, 7 storage).
  Combined local coverage is 44 non-DB plus 29 DB tests. Optional real local-model
  inference is left to the existing GitHub workflow; it is not claimed locally.
