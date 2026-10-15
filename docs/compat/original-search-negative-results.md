# Original search boundary and empty-data verification

The private restoration matches the deployed Python wrapper in **82 additional cases**: 56 exact JSON results and 26 matching refusals. Refusal checks compare error presence, not literal error text. Revised Rust source also reruns the earlier 105 positive cases with exact JSON/ranking equality. See [boundary results](pi-original-search-negative-20261002/comparison-summary.json) and [positive regression](pi-original-search-negative-20261002/positive-regression-summary.json).

| Cases | Count |
| --- | ---: |
| Groups without canonical entities | 20 |
| Groups without canonical facts | 21 |
| Blank/whitespace queries | 4 |
| Stopword/punctuation-only queries | 4 |
| Semantic queries without lexical matches | 2 |
| Empty, duplicated and unauthorized group lists | 12 |
| Nonpositive result limits | 4 |
| Valid, empty and invalid type labels | 10 |
| Empty groups/types with nonblank/blank queries | 4 |
| Multiple groups with a center UUID | 1 |

The [probe](../../scripts/python-original-search-negative-probe.py) loads the guarded reference setup without rerunning its completed corpus campaign. It uses the actual SDK/deployed AST wrappers, preconstructed group drivers and read-only command interceptor. It reuses a captured vector and creates a separate owner-private fixture. The [reference run](pi-original-search-negative-20261002/python-summary.json) records 232 SDK GRAPH.RO_QUERY commands and zero provider calls. The 41 empty-population cases still retain inherited opaque records behind the namespace guard.

Before correction, [63/82 matched](pi-original-search-negative-20261002/before-fix.json). Nineteen differed: blank queries were rejected, while malformed labels/empty type filters and nonblank queries without fulltext terms silently produced results or emptiness instead of the SDK refusal.

Public search now accepts blank text and returns empty before embeddings/DB retrieval, after group/limit checks. Node labels follow the pinned ASCII identifier rule. Empty groups exit before filter validation; blank queries permit empty type lists, whereas nonblank queries with empty types are refused, matching the SDK invalid label-expression path. Python's regex permits a final LF; labels normalize that terminal whitespace before parameterized matching. Existing request byte/count limits remain enforced.

Nonblank stopword/punctuation-only input is refused without executing the malformed fulltext expression or calling a provider. This replaces the earlier deliberate empty-result deviation, which masked a deployed refusal. Long valid queries retain the SDK token-limit behavior. Normal ranking remains unchanged, as the 105-case positive regression confirms.

The isolated provider-contract regression checks these early exits/refusals before any mock model request: both LLM and embedding counters stay zero. It also checks terminal-LF label equivalence. All 30 nonignored all-feature tests and nine isolated compatibility DB/HTTP tests pass, with fmt and all-target/all-feature clippy warnings denied. Local DB is FalkorDB 4.20.4; original-data replay uses the observed 4.16.6 on the actual Pi. Owned local disposable resources are removed. See [local checks](pi-original-search-negative-20261002/local-tests.json).

Compared ARM64 maintenance binary: SHA-256 12433d5b966a4559e3b2b73289071dc58560c6cc1c72a391cd84ce2ea90bd927. [Source hashes](pi-original-search-negative-20261002/source-hashes.json) identify the run. Fixtures, queries, UUIDs, vectors and response diagnostics remain private on the Pi; only sanitized evidence enters Git.

[Re-audit](pi-original-search-negative-20261002/preservation-summary.json) confirms unchanged identity/vector digests, scope counts and time/provenance validation across 63 policy groups. The backup hash and original operational container/start time remain unchanged; nine outside-policy graphs remain retained. This does not claim bytewise equality of every property.

Scope is typed requests within recorded limits and known policy graphs. Exhaustive malformed-schema coverage, unknown-graph/index lifecycle, arbitrary corpus quality and new-query embedding semantics remain unproven. Real-provider writes, OAuth integration and a separately authorized operational cutover remain adoption gates. No Gemini/ER2 requests, new paid alternative or operational switch is included.
