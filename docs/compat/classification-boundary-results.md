# Classification boundary compatibility

An optional classification-only model with an empty description previously produced an empty deduplication description in Rust. The observed Python worker applies `model.__doc__ or 'Default Entity Type'`. Rust now applies the same fallback to an empty string, while preserving a whitespace-only description.

The baseline is the actual deployed Graphiti 0.28.2 SDK on Python 3.11.2. Its `None.__doc__` is null, so both an unconfigured classification and an out-of-range ID use `Default Entity Type`. The comparison pins the interpreter and source hashes; another host interpreter's built-in documentation is not an authoritative substitute.

## Cases and results

The new disposable-DB test covers five synthetic episodes in three profiles:

| Profile | Episodes | Supplied classifications |
| --- | --- | --- |
| Actual ten configured classifications | message, JSON, text | `0`, `-1`, first out-of-range `11`, maximum signed 64-bit integer, first and last valid IDs; a later JSON episode changes classifications of the same names |
| Two synthetic descriptions | message | default, empty description, whitespace description and out-of-range ID |
| No configured classifications | text | default, negative, positive and maximum signed 64-bit integer |

All five extraction responses also include an empty name and a whitespace-only name. Both implementations discard all ten before creating nodes. Node counts are `6, 6, 12, 4, 4`; the reclassified names retain their original UUIDs and labels.

A real invocation of the observed Python `Graphiti.add_episode` replays the same eighteen mock responses and basis vectors against a fresh DB on the Pi. All five canonical graph snapshots match, including episode UUIDs/times, names, summaries, classifications and attributes. All five extraction classification contexts, four deduplication contexts and real Pydantic response schemas match. The comparison checks fifteen fallback descriptions, one preserved whitespace description and twelve repeated-name UUID/label checks across snapshots. The first profile uses the private original ten-type definitions, which stay outside Git. Reports contain only sanitized counts and hashes in [the evidence](pi-classification-boundary-20261002/summary.json).

Before correction, the Rust fixture fails on the empty description (`""` versus `"Default Entity Type"`). After correction, formatting, all-target/all-feature Clippy, thirty non-ignored tests and all twelve disposable-DB compatibility tests pass locally. The same test passes as an ARM64 release binary on the Pi. Local DB tests use FalkorDB 4.14.11; the Pi uses the observed 4.16.6 module.

## Isolation and reproduction

The Pi harness runs only in an owned internal Docker network with a fresh DB, read-only container root, bounded `/data` and `/tmp` memory and an owned loopback relay. Python runs with a cleared environment, `-S`, disabled telemetry, injected model/embedder mocks and the observed edge-query patch. The model endpoints used by Rust are loopback mocks. No original memory, credential or operational socket is mounted. Container/network/relay cleanup and original service/gateway identity/start times, initial backup, restored audit and private profile hashes are rechecked.

Use an exclusively owned disposable FalkorDB on loopback port 16379 for the Rust test:

```sh
COMPAT_TEST_DB_URL=redis://127.0.0.1:16379 cargo test --locked --all-features --test compat classification_boundary::default_invalid_and_empty_description_classifications_match_deployed_python -- --ignored --exact --nocapture
```

`COMPAT_PROFILE_TYPES_FILE` optionally supplies the ten private definitions; `COMPAT_BOUNDARY_REPORT` writes a new owner-private report for `scripts/python-pi-classification-boundary.py`. That Python replay also requires the pinned SDK, original edge patch, supplied report and existing `python-pi-pipeline.py` helper inside a disposable container. The ownership checks belong to the Pi orchestration; the script is not an operational ingestion command.

This closes the bounded classification-ID/description comparison. Larger interacting relation cases, wider cross-SDK failure/restart comparisons, actual provider quality, fresh OAuth and operational cutover remain pending. Original Gemini vectors and operational routing are unchanged.
