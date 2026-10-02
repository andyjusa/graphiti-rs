# Original-memory MCP read verification

Rust source `c663a10` fixes two response differences found against the observed Python 0.28.2 SDK on the private Pi restoration. Fact JSON dates use Pydantic's UTC `Z` and microsecond precision, while private storage timestamps remain unchanged. The fact formatter excludes its named vector attribute and preserves other metadata. Node search was not part of this point/list comparison; the subsequent search replay corrects its separate deployed wrapper format (Python isoformat dates and exclusion of all embedding-named attributes). Episode collection now takes each group's UUID-descending window at the requested limit, then applies Python's stable global time sort. A synthetic fixture checks offset dates, fractional precision and attribute handling. Its current node oracle executes the deployed custom wrapper; the earlier generic SDK node formatter did not describe deployed node search.

## Original reader comparison

The [successful comparison](pi-original-read-20261002/comparison-summary.json) uses the actual SDK models/formatters and observed group-scoped wrapper functions, with the observed FalkorDB module and private original-memory snapshot. Deep JSON comparison performs no date normalization or result filtering to hide differences.

| Result | Compared | Matched |
| --- | ---: | ---: |
| Canonical episode UUID reads | 869 | 869 |
| Canonical fact UUID reads | 6,812 | 6,812 |
| Group episode lists, limit 10 | 63 | 63 |

The 14 mis-scoped episode copies and 70 mis-scoped fact copies remain inaccessible by direct MCP UUID lookup; both readers refuse them. Missing UUIDs are refused in all 63 groups for both point tools. Negative cases compare refusal presence, not literal error text. The collection is the initial restoration, rather than the subsequently changing operational database. Nine outside-policy graphs are retained but excluded from this MCP comparison.

The probes require an explicit private rehearsal marker and observed source bundle. Credentials arrive on stdin; successful output and mismatch diagnostics contain counts/field paths rather than original responses. The host load/recovery helpers must be copied into the owned private rehearsal directory containing its resources manifest and synthetic credentials. They validate owned container identities/labels before accessing or restarting them.

The final harness intercepts the actual async Redis client's commands, maps SDK GRAPH.QUERY calls to GRAPH.RO_QUERY and rejects commands outside its read allowlist. It records **8,080** SDK read-only queries and zero provider calls. Earlier comparison attempts exposed a harness defect: the first overrides targeted the synchronous graph class rather than the actual AsyncGraph. Those attempts matched the corrected responses but failed the read-only counter assertion. The published successful run uses the corrected command guard; its hash matches the checked-in [reader probe](../../scripts/python-original-read-probe.py).

The before-fix run matched all 869 episodes, but every fact had a date serialization difference and 15 group lists differed in selection/order. These findings motivated the implementation and disposable DB regression, including group-order ties and a UUID window that deliberately excludes a newer timestamp outside that window.

## Authentication and transports

The observed OAuth image runs with an isolated SQLite state and synthetic bearer credentials, upstreamed to the private Rust reader. [Sixteen checks](pi-original-read-20261002/auth-summary.json) and a separate [expired-token check](pi-original-read-20261002/auth-expiry-summary.json) pass: missing/wrong/cross-layer tokens, Origin/Host guards, metadata challenge/discovery, legacy initialization and the exact ten tool schemas. **78 representative original responses** also match through OAuth during the corpus comparison. This uses seeded test tokens; fresh GitHub login and operational client identities are not certified.

The observed Node bridge and direct Rust endpoint both pass native 2026-07-28 discovery, exact tool schemas and an original episode-list read. The bridge adds protocol `_meta`; content and structured result match. [Transport evidence](pi-original-read-20261002/stateless-summary.json) records that difference explicitly.

A [known OAuth limitation](pi-original-read-20261002/oauth-modern-limitation.json) remains: its deployed forwarding allowlist omits Mcp-Method and Mcp-Name. Modern discovery through that gateway receives HTTP 400 / -32020. OAuth verification therefore covers 2025-11-25; modern direct/Node transport success does not certify modern OAuth. A future gateway integration must forward those standard headers without weakening bearer or Origin validation.

[Recovery checks](pi-original-read-20261002/reconnect-summary.json) stop only the owned Rust test backend, observe the Node bridge's -32603 and OAuth's 502, then recover unchanged original reads through all three paths. Restarting each test gateway also retains token acceptance. This is a read-service failure/recovery rehearsal; no operational upstream switch or operational rollback was exercised.

## Bounded Pi measurements

After the SDK comparison exits, a host HTTP client repeats two restored-memory point reads 96 times each. All 192 responses remain identical. [Recorded measurements](pi-original-read-20261002/rust-read-load.json): median **6.782 ms**, p95 **14.326 ms**, maximum **16.497 ms**. Six Docker samples show Rust memory **3.398–3.840 MiB**, database **436.9–438.1 MiB**, idle OAuth **1.324 MiB** and idle Node bridge **50.29 MiB**. Rust CPU samples range 2.78–8.00%; database 2.57–5.18%. These are warm, sequential reads of two records, not an ingestion, cold-start, throughput or production peak certification.

The [maintenance re-audit](pi-original-read-20261002/preservation-summary.json) is identical before/after for every inspected group's identity/vector digests, scope counts and timestamp/provenance validation. The initial backup's SHA-256 is unchanged. This does not claim full bytewise property equivalence or audit the nine outside-policy graphs.

[Cleanup](pi-original-read-20261002/cleanup-summary.json) removes only three owned test services and synthetic credentials. Private diagnostic files, original restoration, its network/relay and the initial backup remain on the Pi. The operational original service retains its container/start time and healthy state; its upstream is unchanged. Only sanitized counts, field paths, hashes and fixed synthetic fixtures enter Git.

## Remaining adoption gates

Local regression verification passes 30 nonignored tests, eight disposable compatibility DB/HTTP tests, fmt and all-target/all-feature clippy with warnings denied. The local disposable DB uses FalkorDB 4.20.4; the original-data Pi comparison uses the observed 4.16.6 module. Earlier 50-test Pi evidence from `f8199d1` remains historical evidence, not a new release-test run for this source. [Artifact and harness hashes](pi-original-read-20261002/artifact-hashes.json) identify the tested ARM64 main executable and public serializer fixture.

The subsequent [native-vector search replay](original-search-results.md) matches 105 original node/fact responses including ordering and fixes the deployed node DTO oracle. New-query embeddings, broader corpus/negative search coverage, real multilingual embeddings/generation quality, operational writes, fresh OAuth login, reviewed gateway integration and queue-drained cutover remain separate gates. The latest user restriction prohibits new Gemini/ER2 calls and selecting a new paid alternative; existing 3072-dimensional vectors are retained. The earlier billing diagnosis is historical and is not authorization to retry after funding.
