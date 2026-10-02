# Same-Pi validation plan

Current decision: **no production replacement**. The isolated synthetic phase ran on 2026-10-02; see [measured evidence and remaining gates](pi-results.md). The optional compatibility preview still prohibits production mutations and real model calls. CI on a different module establishes synthetic correctness only. [observed-production.json](observed-production.json) records read-only metadata of the running Pi/container; [source excerpt manifest](../../tests/fixtures/python-0282/manifest.json) identifies the observed Python method bodies. Source collection did not export graph data or read credentials.

## 1. Isolated synthetic validation

The 2026-10-02 user-requested test window executed this phase on the actual Pi with synthetic data. For subsequent runs, use an isolated test window. The running Python/FalkorDB container, its volumes, production network, gateway, credentials and routes stay in place. The compose file uses a **new empty directory**, an internal network, a loopback port, and an explicit Redis entrypoint. Never start the image's application entrypoint: it can initialize the operational Python adapters. Do not attach operational mounts or inherit production environment variables.

On the Pi, review the checked-out PR commit and current image/module metadata. Recheck that port 16379 is unused. Abort if the image ID, module hash, module version or Redis version differs from the observed record; update the record and test plan instead of substituting a newer image. Review available memory and concurrent work before allocating the proposed one CPU/512 MiB DB limit; these limits are provisional, not a proven Pi budget.

```sh
umask 077
export COMPAT_TEST_ROOT="$(mktemp -d "$HOME/graphiti-rs-compat-pi.XXXXXXXX")"
mkdir "$COMPAT_TEST_ROOT/db"
export COMPAT_TEST_UID="$(id -u)"
export COMPAT_TEST_GID="$(id -g)"
export COMPAT_TEST_FALKOR_IMAGE=sha256:ed8f5d566429152635aaf6795ffd4e5bfd11f704ea03d6cc79fa25c41384d854
docker compose -f compose.compat-pi.yaml config
# Review rendered mounts, entrypoint, image, loopback port and labels before this step.
docker compose -f compose.compat-pi.yaml up -d
```

Verify `redis-server --version`, `redis-cli MODULE LIST` and the module SHA256 **inside the new test container**. Run `cargo build --release --locked --features graphiti-compat` and the checked-in compatibility tests with `COMPAT_TEST_DB_URL=redis://127.0.0.1:16379`; the model servers in the tests use synthetic responses and dummy keys. Also run the source-function differential tests with Python 3. Do not enable actual model endpoints. Each ignored integration fixture must own its graph keys; never point the harness at a restored production database because it intentionally creates/deletes its test graph keys.

```sh
cargo test --locked --features graphiti-compat --test compat_differential
COMPAT_TEST_DB_URL=redis://127.0.0.1:16379 \
  cargo test --locked --features graphiti-compat --test compat -- --ignored --test-threads=1
```

On 2026-10-02 release test executables built in the ARM64 validation builder ran on this Pi because no Pi Rust compiler was installed. All 42 tests passed. The internal-network published port was unreachable, so the tested `compose.compat-pi-relay.yaml` overlay removes it and `scripts/isolated-db-relay.py` supplies a labelled loopback transport. Measured latency includes relay cost. `scripts/pi-synthetic-validation.py` additionally exercised HTTP search/local inference/SIGKILL/DB outage/AOF restart. These results are synthetic evidence; restore/provider/full-pipeline/gateway/production-budget gates remain open.

## 2. Resource, retrieval and restart gates

Create a separate persistent synthetic benchmark namespace and journal; keep the fixture harness out of that namespace. Measure Python and Rust sequentially against identical synthetic graphs and identical mock provider outputs. Record image/module/source/commit hashes, corpus size, label/attribute shapes, embedding space, candidate limits, concurrency, warmup, sample count and workload seed. Record the DB separately from the server: RSS, peak RSS, CPU seconds, response p50/p95/p99, errors/timeouts, idle CPU, journal size, and swap growth. Use host `pidstat`/`/proc/<pid>` and `docker stats` if available. Proposed canary gates require a reviewable measured budget; no latency or RSS threshold has been approved or passed yet.

Workloads must include exact/keyword/semantic/multihop queries, center absent/center source/center target/direct neighbor/disconnected sources, types, empty/duplicated/multiple groups, shared provenance, duplicate names, outdated facts, offset/precision timestamps, a corpus above candidate windows, and queue/read saturation. Score expected UUID recall@k and MRR against hand-labelled synthetic truth, then compare Python/Rust results. The preview has bounded substring candidates, no Python fulltext/vector retrieval: passing method-level ranking tests is insufficient for retrieval acceptance. The observed MCP recipes do not enable BFS; other Graphiti recipes with BFS are outside this tool contract.

Use the actual observed patched Python pipeline in its isolated test process with mock LLM/embedding clients. The checked-in stdlib runner executes selected method bodies with fake DTOs and a fake driver; it does **not** run full episode extraction, summary updating, fuzzy entity resolution, custom relation typing, graph writes or transport. Test these separately before describing the replacement as equivalent. Real generation and Gemini numerical comparisons require a separately agreed bounded provider run with credentials kept on the Pi; mock arrays never establish model equivalence. Compare DB `vecf32` values after float32 conversion, reject mixed embedding spaces, and record disagreements rather than rounding them away.

Restart/failure tests must distinguish: model-dispatch ambiguity; prepared before-state persistence; graph commit with lost journal completion; missing/altered MENTIONS; concurrent changed properties/endpoints; pending delete/clear with lost completion; SQLite busy/full; DB disconnect; disk-full and process termination. Stop/restart **only the isolated service**, keep the journal/data directories on persistent storage, and verify no additional model calls, no partial episode insertion, stable temporal fields/provenance, and truthful receipt states. Pending destructive work blocks the affected group; tombstoned UUIDs never silently replay. Synthetic destructive recovery assumes a single writer to the disposable DB.

## 3. Separate restore/provider/transport boundary

Actual memory backups/restores, secret access/transmission, paid provider calls, gateway/session changes and route switching are separate operational actions; none were performed in this change. Prepare their concrete commands and destinations for review before requesting their execution. Do not copy a production volume into the synthetic harness or include backups/secrets in Git/CI artifacts.

For an agreed restore test, retain the original backup and hash, restore into a fresh isolated DB with the **same** module, perform read-only integrity/schema/vector/count checks, and confirm restore success before permitting isolated mutations. Account for extra disk/RAM and protect the backup with private permissions. An exported file or successful backup command alone is not a restore test. Compare unknown labels, Saga/Community topology, arrays/scalars, UUID collisions and vector dimensions; any unsupported shape remains a no-go.

For a future test-only gateway, use a separate listener/client/session and a synthetic bearer file; test initialization, tool schemas/result wrapping, Host/Origin/OAuth forwarding, auth failure, session reconnect and restart. Keep the operational upstream unchanged. Record the prior gateway configuration hash and exact rollback command before a proposed canary. Actual cutover requires ingestion freeze/drain and accepted-job reconciliation, then a narrowly scoped route change with explicit success criteria. Rollback must restore the prior route, reconnect the client, verify exact known episode reads and account for all accepted jobs. Do not infer rollback readiness from compiling a binary.

## 4. Owned-resource cleanup

For this new Pi compose project, first preserve needed synthetic evidence/journal files and verify `task=graphiti-rs-compat-pi-synthetic` labels and `com.docker.compose.project=graphiti-rs-compat-pi`. Then remove only that project's containers/network. Remove only the newly created `$COMPAT_TEST_ROOT` after confirming its contents and provenance. Never run `docker system prune`, stop production containers, or delete unrelated volumes.

Earlier Mac resources were removed on 2026-10-02 after Docker became available and each exact task label was verified: `graphiti-rs-compat-db`, `graphiti-rs-compat-data`, `graphiti-rs-compat-net`, all originally labelled `task=graphiti-rs-compat-20261001`. No global restart/prune or unrelated-resource cleanup was performed.
