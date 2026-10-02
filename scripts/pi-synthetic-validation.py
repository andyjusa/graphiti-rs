#!/usr/bin/env python3
"""Run only in the disposable compose project described in pi-validation.md.

Uses stdlib, synthetic graphs, explicit extraction and loopback model mocks.
Measures the release server separately from the DB. It is not a production
backup/restore, real-provider comparison, or a full Python pipeline test.
"""
import argparse
import concurrent.futures
import hashlib
import http.server
import json
import os
from pathlib import Path
import random
import socket
import subprocess
import threading
import time
import urllib.error
import urllib.request

CONTAINER = "graphiti-rs-compat-pi-falkordb-1"
GROUP = "project_pi_benchmark_20261002"
NATIVE_GRAPH = "pi_synthetic_native_20261002"
COUNT = 1200


def corpus_identity():
    rows = [{"id": uid(i+100000), "fact": f"marker{i:06d} synthetic employment"} for i in range(COUNT)]
    return hashlib.sha256(json.dumps(rows, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


class Redis:
    def __init__(self):
        self.sock = socket.create_connection(("127.0.0.1", 16379), timeout=10)
        self.stream = self.sock.makefile("rb")

    def read(self):
        line = self.stream.readline()
        kind, value = line[:1], line[1:-2]
        if kind == b"+":
            return value.decode()
        if kind == b"-":
            raise RuntimeError(value.decode())
        if kind == b":":
            return int(value)
        if kind == b"$":
            size = int(value)
            if size == -1:
                return None
            result = self.stream.read(size)
            assert self.stream.read(2) == b"\r\n"
            return result.decode()
        if kind == b"*":
            size = int(value)
            return None if size == -1 else [self.read() for _ in range(size)]
        raise RuntimeError("invalid Redis response")

    def call(self, *args):
        args = [str(a).encode() for a in args]
        body = b"*%d\r\n" % len(args)
        body += b"".join(b"$%d\r\n" % len(a) + a + b"\r\n" for a in args)
        self.sock.sendall(body)
        return self.read()

    def query(self, group, text):
        return self.call("GRAPH.QUERY", group, text, "TIMEOUT", 5000)

    def close(self):
        self.stream.close()
        self.sock.close()


def uid(i):
    return f"00000000-0000-4000-8000-{i:012x}"


def quantiles(values):
    ordered = sorted(values)
    return {name: ordered[min(len(ordered)-1, int(q * len(ordered)))]
            for name, q in (("p50_ms", .5), ("p95_ms", .95), ("p99_ms", .99))}


def metrics(pid):
    status = Path(f"/proc/{pid}/status").read_text()
    memory = {}
    for line in status.splitlines():
        if line.startswith(("VmRSS:", "VmHWM:")):
            memory[line.split(":")[0] + "_KiB"] = int(line.split()[1])
    stat = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    memory["cpu_seconds"] = (int(stat[11]) + int(stat[12])) / os.sysconf("SC_CLK_TCK")
    return memory


def host_memory():
    rows = {r.split(":")[0]: int(r.split()[1])
            for r in Path("/proc/meminfo").read_text().splitlines()}
    return {k + "_KiB": rows[k] for k in ("MemAvailable", "SwapTotal", "SwapFree")}


class Server:
    def __init__(self, root, label, extra, compat=False):
        self.root = root
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            self.port = s.getsockname()[1]
        self.env = {"PATH": "/usr/local/bin:/usr/bin:/bin", "LANG": "C.UTF-8",
                    "GRAPHITI_BIND": f"127.0.0.1:{self.port}",
                    "FALKORDB_URL": "redis://127.0.0.1:16379",
                    "GRAPHITI_GRAPH": NATIVE_GRAPH,
                    "GRAPHITI_TOKEN": "synthetic-fixture-token",
                    "EMBEDDING_PROVIDER": "disabled", "COMPAT_INGESTION_PIPELINE": "combined-preview", **extra}
        self.argv = [str(root / "bin/graphiti-rs"), "--transport", "http"]
        if compat:
            self.argv += ["--storage-format", "graphiti-0282"]
        self.log = (root / f"evidence/{label}.log").open("ab")
        self.process = None
        self.start()

    def start(self):
        self.process = subprocess.Popen(self.argv, env=self.env, cwd=self.root / "source",
                                        stdin=subprocess.DEVNULL, stdout=self.log, stderr=self.log)
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError(f"server exited: {self.process.returncode}")
            try:
                self.rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                       "clientInfo": {"name": "pi-synthetic", "version": "1"}})
                return
            except (ConnectionError, urllib.error.URLError):
                time.sleep(.1)
        raise RuntimeError("server did not initialize")

    def rpc(self, method, params):
        request = urllib.request.Request(
            f"http://127.0.0.1:{self.port}/mcp",
            json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(),
            {"Content-Type": "application/json", "Accept": "application/json, text/event-stream",
             "MCP-Protocol-Version": "2025-11-25", "Authorization": "Bearer synthetic-fixture-token"})
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        with opener.open(request, timeout=30) as response:
            body = json.load(response)
        if "error" in body:
            raise RuntimeError(body["error"])
        result = body["result"]
        if result.get("isError"):
            raise RuntimeError(result)
        return result

    def tool(self, name, arguments):
        result = self.rpc("tools/call", {"name": name, "arguments": arguments})
        if "structuredContent" in result:
            value = result["structuredContent"]
            value = value.get("result", value)
        else:
            value = json.loads(result["content"][0]["text"])
        if isinstance(value, dict) and "error" in value:
            raise RuntimeError(value["error"])
        return value

    def stop(self, kill=False):
        if self.process.poll() is None:
            if kill:
                self.process.kill()
            else:
                self.process.terminate()
            self.process.wait(timeout=25)


class Mock(http.server.BaseHTTPRequestHandler):
    counts = {"llm": 0, "embeddings": 0}
    block = threading.Event()
    entered = threading.Event()
    lock = threading.Lock()

    def log_message(self, *args):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.path == "/chat/completions":
            with self.lock:
                self.counts["llm"] += 1
            text = body["messages"][-1]["content"].split("Episode: ", 1)[1]
            if text == "dispatch ambiguity":
                self.entered.set()
                self.block.wait(20)
            value = {"entities": [{"name": "Pi synthetic worker", "summary": "Fixture", "labels": ["Person"]},
                                  {"name": "Pi synthetic target", "summary": "Fixture", "labels": ["Company"]}],
                     "facts": [{"source": 0, "target": 1, "name": "WORKS_AT",
                                "fact": "Pi synthetic worker works at Pi synthetic target"}]}
            result = {"choices": [{"message": {"content": json.dumps(value)}}]}
        elif self.path == "/embeddings":
            with self.lock:
                self.counts["embeddings"] += 1
            result = {"data": [{"index": i, "embedding": [0., 1.]
                               if text in ("semantic outlier", "A cyclist on a road") else [1., 0.]}
                               for i, text in enumerate(body["input"])]}
        elif self.path == "/models/gemini-embedding-2:batchEmbedContents":
            with self.lock:
                self.counts["embeddings"] += 1
            result = {"embeddings": [{"values": [0.0, 1.0] + [0.0]*3070
                                     if r["content"]["parts"][0]["text"] == "semantic outlier"
                                     else [1.0] + [0.0]*3071}
                                    for r in body["requests"]]}
        else:
            self.send_error(404)
            return
        data = json.dumps(result).encode()
        try:
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass


def idle_and_search(server, name, queries, key):
    start = metrics(server.process.pid)
    time.sleep(2)
    idle = metrics(server.process.pid)
    samples, successes, reciprocal_ranks, errors, disagreements = [], 0, [], [], []
    # Five warmup calls do not enter the latency distribution.
    for args, expected in queries[:5]:
        server.tool(name, args)
    cpu = metrics(server.process.pid)["cpu_seconds"]
    for args, expected in queries:
        began = time.perf_counter()
        try:
            result = server.tool(name, args)
            found = result if isinstance(result, list) else result[key]
            ids = [item.get("uuid", item.get("id")) for item in found]
            successes += bool(expected in ids[:10])
            reciprocal_ranks.append(1 / (ids.index(expected)+1) if expected in ids[:10] else 0)
            if ids[:1] != [expected] and len(disagreements) < 5:
                disagreements.append({"query": args["query"], "expected": expected,
                                      "returned": [{"id": item.get("uuid", item.get("id")), "fact": item.get("fact")} for item in found]})
        except Exception as exc:
            errors.append(str(exc))
            reciprocal_ranks.append(0)
        samples.append((time.perf_counter() - began) * 1000)
    final = metrics(server.process.pid)
    return {"samples": len(samples), "warmup": 5, **quantiles(samples),
            "exact_uuid_recall_at_10": successes / len(queries), "errors": errors,
            "mrr_at_10": sum(reciprocal_ranks) / len(queries),
            "first_rank_disagreement_examples": disagreements,
            "idle_cpu_seconds_per_2s": idle["cpu_seconds"] - start["cpu_seconds"],
            "search_cpu_seconds": final["cpu_seconds"]-cpu, **final}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-root", required=True, type=Path)
    args = parser.parse_args()
    root = args.test_root.resolve()
    assert root.parent == Path("/home/pi4-3") and root.name.startswith("graphiti-rs-compat-pi.")
    info = json.loads(subprocess.check_output(["docker", "inspect", CONTAINER]))[0]
    assert info["Config"]["Labels"]["task"] == "graphiti-rs-compat-pi-synthetic"
    assert info["Config"]["Labels"]["com.docker.compose.project"] == "graphiti-rs-compat-pi"
    assert info["Config"]["Entrypoint"] == ["redis-server"]
    assert info["Mounts"][0]["Source"] == str(root / "db")
    report = {"scope": "same-Pi synthetic release binary, not replacement certification",
              "status": "running",
              "corpus_facts": COUNT, "seed": 20261002, "host_before": host_memory(),
              "legacy_text_identity_corpus_sha256": corpus_identity(),
              "legacy_embedding_space": "mock 3072-dim unit basis; first 64=basis0, final=basis1, others absent",
              "binary_sha256": hashlib.sha256((root / "bin/graphiti-rs").read_bytes()).hexdigest(),
              "production_mutations": False, "real_provider_calls": False}
    servers = []
    mock = None
    redis = Redis()
    assert redis.call("PING") == "PONG"
    try:
        native = Server(root, "native", {})
        servers.append(native)
        all_ids = []
        for first in range(0, COUNT, 50):
            entities = [{"name": "Synthetic Hub"}] + [{"name": f"Synthetic Person {i}"} for i in range(first, first+50)]
            facts = [{"source": f"Synthetic Person {i}", "target": "Synthetic Hub", "relation": "WORKS_AT",
                      "fact": f"marker{i:06d} synthetic employment"} for i in range(first, first+50)]
            result = native.tool("add_episode", {"group_id": "pi_bench", "episode_id": f"batch{first}",
                "content": f"Synthetic batch {first}", "source": "synthetic",
                "reference_time": "2026-10-01T00:00:00Z", "extraction": {"entities": entities, "facts": facts}})
            all_ids += result["fact_ids"]
        order = random.Random(20261002).sample(range(COUNT), 100)
        native_queries = [({"group_id": "pi_bench", "query": f"marker{i:06d}", "limit": 10}, all_ids[i]) for i in order]
        report["standalone_keyword"] = idle_and_search(native, "search_facts", native_queries, "facts")
        assert report["standalone_keyword"]["exact_uuid_recall_at_10"] == 1
        assert not report["standalone_keyword"]["errors"]
        def saturated_read(_):
            try:
                native.tool("search_facts", native_queries[0][0])
                return "admitted"
            except Exception as exc:
                return str(exc)
        with concurrent.futures.ThreadPoolExecutor(max_workers=32) as pool:
            saturation = list(pool.map(saturated_read, range(100)))
        report["standalone_search_saturation"] = {"concurrency": 32, "requests": 100,
            "admitted": saturation.count("admitted"),
            "rejected": len(saturation)-saturation.count("admitted"),
            "rejection_examples": list(dict.fromkeys(s for s in saturation if s != "admitted"))}
        exact = native.tool("get_episode", {"group_id": "pi_bench", "episode_id": "batch0"})
        native.stop(kill=True)
        native.start()
        assert native.tool("get_episode", {"group_id": "pi_bench", "episode_id": "batch0"}) == exact
        report["standalone_sigkill_restart"] = "passed; exact episode unchanged"
        native.stop()

        local = Server(root, "local-embeddings", {"GRAPHITI_GRAPH": "pi_synthetic_local_20261002",
            "EMBEDDING_PROVIDER": "local", "EMBEDDING_MODEL": "all-MiniLM-L6-v2",
            "EMBEDDING_DIM": "384", "LOCAL_MODEL_DIR": str(root / "models/minilm")})
        servers.append(local)
        sentences = ["A person is riding a bicycle", "Database connection timed out",
                     "The cat is sitting on the sofa", "A person is cooking pasta"]
        result = local.tool("add_episode", {"group_id": "pi_local", "episode_id": "local-fixture",
            "content": "Synthetic English semantic fixture", "source": "synthetic",
            "reference_time": "2026-10-01T00:00:00Z", "extraction": {
                "entities": [{"name": "Fixture source"}, {"name": "Fixture target"}],
                "facts": [{"source": "Fixture source", "target": "Fixture target", "relation": "FIXTURE",
                           "fact": sentence} for sentence in sentences]}})
        local_queries = [({"group_id": "pi_local", "query": "Someone rides a bike", "limit": 1},
                          result["fact_ids"][0])] * 10
        report["local_minilm_english_smoke"] = idle_and_search(local, "search_facts", local_queries, "facts")
        report["local_minilm_english_smoke"]["scope"] = "four facts, one paraphrase repeated; not multilingual or corpus quality certification"
        assert report["local_minilm_english_smoke"]["exact_uuid_recall_at_10"] == 1
        assert not report["local_minilm_english_smoke"]["errors"]
        local.stop()

        stamp = '"2026-10-01T00:00:00.123456789Z"'
        for first in range(0, COUNT, 50):
            rows = [f'{{id:{json.dumps(uid(i+100000))},nid:{json.dumps(uid(i+200000))},name:"Synthetic Person {i}",fact:"marker{i:06d} synthetic employment"}}'
                    for i in range(first, first+50)]
            query = f'CYPHER rows=[{",".join(rows)}] UNWIND $rows AS r MERGE(t:Entity {{uuid:"{uid(999999)}"}}) SET t.name="Synthetic Hub",t.summary="Fixture",t.group_id="{GROUP}",t.created_at={stamp} CREATE(n:Entity:Person {{uuid:r.nid,name:r.name,summary:"Fixture",group_id:"{GROUP}",created_at:{stamp}}}) CREATE(n)-[e:RELATES_TO {{uuid:r.id,name:"WORKS_AT",fact:r.fact,group_id:"{GROUP}",created_at:{stamp},valid_at:{stamp},episodes:[]}}]->(t)'
            redis.query(GROUP, query)
        # Only the first 64 and the final fact have vectors, with a known far-outlier.
        for i in [*range(64), COUNT-1]:
            vector = [0., 1.] + [0.]*3070 if i == COUNT-1 else [1.] + [0.]*3071
            redis.query(GROUP, f'MATCH ()-[e:RELATES_TO {{uuid:"{uid(i+100000)}"}}]->() SET e.fact_embedding=vecf32({json.dumps(vector)})')
            redis.query(GROUP, f'MATCH(n:Entity {{uuid:"{uid(i+200000)}"}}) SET n.name_embedding=vecf32({json.dumps(vector)})')
        redis.query(GROUP, "CREATE INDEX FOR ()-[e:RELATES_TO]-() ON (e.uuid,e.group_id,e.name,e.created_at,e.expired_at,e.valid_at,e.invalid_at)")
        readonly = Server(root, "compat-read", {"COMPAT_DB_URL": "redis://127.0.0.1:16379"}, compat=True)
        servers.append(readonly)
        queries = [({"group_ids": [GROUP], "query": f"marker{i:06d}", "max_facts": 10}, uid(i+100000)) for i in order]
        report["compat_keyword"] = idle_and_search(readonly, "search_memory_facts", queries, "facts")
        assert report["compat_keyword"]["exact_uuid_recall_at_10"] == 1
        assert not report["compat_keyword"]["errors"]
        assert readonly.tool("search_nodes", {"group_ids": [GROUP], "query": "Person 1199", "entity_types": ["Company"]})["nodes"] == []
        assert readonly.tool("search_memory_facts", {"group_ids": [], "query": "marker"})["facts"] == []
        report["type_empty_group_filters"] = "passed"
        readonly.stop()

        mock = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Mock)
        threading.Thread(target=mock.serve_forever, daemon=True).start()
        base = f"http://127.0.0.1:{mock.server_port}"
        semantic = Server(root, "native-semantic", {"EMBEDDING_PROVIDER": "external",
            "EMBEDDING_MODEL": "synthetic", "EMBEDDING_DIM": "2", "MODEL_API_BASE": base,
            "SEARCH_CANDIDATES": "50"})
        servers.append(semantic)
        def semantic_episode(eid, stamp, facts):
            return semantic.tool("add_episode", {"group_id": "pi_semantic", "episode_id": eid,
                "content": "Synthetic semantic fixture " + eid, "source": "synthetic",
                "reference_time": stamp, "extraction": {"entities": [{"name": "Semantic source"}, {"name": "Semantic hub"}],
                    "facts": [{"source": "Semantic source", "target": "Semantic hub", "relation": "LINK", "fact": fact} for fact in facts]}})
        target = semantic_episode("old", "2026-10-01T00:00:00Z", ["A cyclist on a road"])["fact_ids"][0]
        semantic_episode("recent", "2026-10-02T00:00:00Z", [f"Unrelated decoy {i}" for i in range(201)])
        args = {"group_id": "pi_semantic", "query": "semantic outlier", "limit": 10,
                "as_of": "2026-10-02T01:00:00Z"}
        began = time.perf_counter()
        hits = semantic.tool("search_facts", args)
        assert hits[0]["id"] == target
        report["standalone_semantic_outside_recent_window"] = {"facts": 202, "expected_id": target,
            "returned_ids": [f["id"] for f in hits], "first_rank": 1,
            "latency_ms": (time.perf_counter()-began)*1000, **metrics(semantic.process.pid)}
        redis.query(NATIVE_GRAPH, "MATCH ()-[r:FACT]->() WHERE r.group_id='pi_semantic' SET r.embedding=null,r.embedding_version=null")
        began = time.perf_counter()
        hits = semantic.tool("search_facts", args)
        assert hits[0]["id"] == target
        report["standalone_json_only_semantic_fallback"] = {"facts": 202, "expected_id": target,
            "returned_ids": [f["id"] for f in hits], "first_rank": 1,
            "latency_ms": (time.perf_counter()-began)*1000, **metrics(semantic.process.pid)}
        semantic.stop()
        synthetic = Server(root, "compat-worker", {"COMPAT_DB_URL": "redis://127.0.0.1:16379",
            "COMPAT_WRITE_MODE": "synthetic", "COMPAT_JOURNAL_DIR": str(root / "journal"),
            "COMPAT_LLM_BASE": base, "COMPAT_GEMINI_BASE": base, "COMPAT_MAX_TOKENS": "4096"}, compat=True)
        servers.append(synthetic)
        result = synthetic.tool("search_memory_facts", {"group_ids": [GROUP], "query": "semantic outlier", "max_facts": 10})
        report["semantic_outside_candidate_window"] = {"expected_uuid": uid(COUNT-1+100000),
            "returned_uuids": [f["uuid"] for f in result["facts"]], "recall_at_10": int(any(f["uuid"] == uid(COUNT-1+100000) for f in result["facts"])),
            "gate": "passed: DB ranks the entire supported vector set before the result bound"}
        assert report["semantic_outside_candidate_window"]["recall_at_10"] == 1
        result = synthetic.tool("search_nodes", {"group_ids": [GROUP], "query": "semantic outlier", "max_nodes": 10, "entity_types": ["Person"]})
        assert result["nodes"][0]["uuid"] == uid(COUNT-1+200000)
        report["semantic_node_outside_candidate_window"] = {"expected_uuid": uid(COUNT-1+200000),
            "returned_uuids": [n["uuid"] for n in result["nodes"]], "first_rank": 1}
        assert synthetic.tool("search_nodes", {"group_ids": [GROUP], "query": "semantic outlier", "entity_types": ["Company"]})["nodes"] == []
        job_group = "project_pi_worker_20261002"
        job = {"group_id": job_group, "uuid": uid(900001), "name": "restart fixture", "episode_body": "worker fixture"}
        synthetic.tool("add_memory", job)
        deadline = time.monotonic()+20
        while time.monotonic() < deadline:
            exact = synthetic.tool("get_episode", {"group_id": job_group, "uuid": job["uuid"]})
            if exact["found"]:
                break
            time.sleep(.05)
        assert exact["found"]
        calls = dict(Mock.counts)
        synthetic.stop(kill=True)
        synthetic.start()
        synthetic.tool("add_memory", job)
        assert synthetic.tool("get_episode", {"group_id": job_group, "uuid": job["uuid"]}) == exact
        assert Mock.counts == calls
        report["committed_sigkill_restart"] = "passed; exact metadata unchanged, zero repeat model calls"
        uncertain = {**job, "uuid": uid(900002), "name": "ambiguous dispatch", "episode_body": "dispatch ambiguity"}
        synthetic.tool("add_memory", uncertain)
        assert Mock.entered.wait(15)
        synthetic.stop(kill=True)
        Mock.block.set()
        calls = dict(Mock.counts)
        synthetic.start()
        time.sleep(1)
        assert not synthetic.tool("get_episode", {"group_id": job_group, "uuid": uncertain["uuid"]})["found"]
        assert Mock.counts == calls
        report["worker_metrics"] = metrics(synthetic.process.pid)
        # Inspect persisted receipts after stopping this isolated worker.
        synthetic.stop()
        import sqlite3
        jobs_file = root / "journal/jobs.sqlite"
        assert jobs_file.is_file()
        with sqlite3.connect(jobs_file.as_uri()+"?mode=ro", uri=True) as db:
            rows = db.execute("SELECT * FROM jobs").fetchall()
            columns = [r[1] for r in db.execute("PRAGMA table_info(jobs)")]
            state = next(dict(zip(columns, row)) for row in rows if uncertain["uuid"] in row)
        assert state["state"] == "failed"
        report["dispatch_ambiguity_sigkill_restart"] = "passed; failed receipt, no episode, no automatic provider replay"
        report["mock_calls"] = dict(Mock.counts)
        synthetic.stop()
        redis.close()
        synthetic.start()
        subprocess.run(["docker", "stop", CONTAINER], check=True, stdout=subprocess.DEVNULL)
        assert synthetic.tool("get_status", {})["status"] == "error"
        subprocess.run(["docker", "start", CONTAINER], check=True, stdout=subprocess.DEVNULL)
        for _ in range(100):
            try:
                redis = Redis()
                if redis.call("PING") == "PONG":
                    break
            except (OSError, RuntimeError):
                time.sleep(.1)
        deadline = time.monotonic()+30
        while synthetic.tool("get_status", {})["status"] != "ok":
            assert time.monotonic() < deadline, "server did not reconnect"
            time.sleep(.1)
        assert synthetic.tool("get_episode", {"group_id": job_group, "uuid": job["uuid"]}) == exact
        synthetic.stop()
        report["db_disconnect_reconnect"] = "passed; error status while isolated DB stopped, recovery without server restart"
        native.start()
        assert native.tool("get_episode", {"group_id": "pi_bench", "episode_id": "batch0"})["id"] == "batch0"
        native.stop()
        synthetic.start()
        assert synthetic.tool("get_episode", {"group_id": job_group, "uuid": job["uuid"]}) == exact
        assert Mock.counts == calls
        synthetic.stop()
        report["aof_db_restart"] = "passed; standalone and compatibility exact records survive, no provider replay"
        report["db_stats"] = json.loads(subprocess.check_output(["docker", "stats", "--no-stream", "--format", "{{json .}}", CONTAINER]))
        memory_info = dict(line.split(":", 1) for line in redis.call("INFO", "memory").splitlines()
                           if ":" in line and not line.startswith("#"))
        report["redis_memory_bytes"] = {k: int(memory_info[k]) for k in
            ("used_memory", "used_memory_rss", "used_memory_peak")}
        report["transport"] = "Rust HTTP over loopback; DB over labelled docker-exec stdlib relay; latency includes relay overhead"
        report["limits"] = {"db_cpu_quota": 1, "db_memory_MiB": 512,
                            "rust_async_workers": 2, "rust_blocking_threads_max": 4,
                            "server_process_cpu_or_memory_cgroup_limit": None}
        report["host_after"] = host_memory()
        report["status"] = "completed; semantic outlier gates passed for compatibility facts/nodes and standalone native/JSON vectors"
    finally:
        if report["status"] == "running":
            report["status"] = "failed before completion; inspect console and server logs"
        for server in servers:
            server.stop()
            server.log.close()
        if mock:
            mock.shutdown()
        redis.close()
        (root / "evidence/benchmark.json").write_text(json.dumps(report, indent=2)+"\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
