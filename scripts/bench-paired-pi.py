#!/usr/bin/env python3
"""Same-Pi, isolated read/search comparison. Never runs on macOS.

Uses existing pinned images, synthetic data and loopback embedding mocks only.
The host coordinator creates uniquely labelled containers and removes only its
own resources. No production entrypoint, volume, key or external API is used.
"""
import argparse
import datetime
import hashlib
import http.client
import http.server
import json
import os
from pathlib import Path
import platform
import random
import re
import signal
import socket
import statistics
import subprocess
import threading
import time
import uuid

MARKER = "graphiti-paired-pi-20261015"
PYTHON_IMAGE = "sha256:ed8f5d566429152635aaf6795ffd4e5bfd11f704ea03d6cc79fa25c41384d854"
RUST_IMAGE = "sha256:8b5ba4567e8047d95edcca0a197cc2645a713d75cff3330fcb77571e1d576604"
GROUP = "project_paired_bench_20261015"
COUNT = 1200
STAMP = "2026-10-15T00:00:00Z"
TOKEN = "isolated-synthetic-benchmark-token"


def encode(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def cypher(value):
    if isinstance(value, dict):
        assert all(re.fullmatch(r"[a-zA-Z_][a-zA-Z0-9_]*", key) for key in value)
        return "{" + ",".join(key + ":" + cypher(item) for key, item in value.items()) + "}"
    if isinstance(value, list):
        return "[" + ",".join(cypher(item) for item in value) + "]"
    return json.dumps(value)


def identity(value):
    return f"00000000-0000-4000-8000-{value:012x}"


def vector(index):
    values = [0.0] * 3072
    values[index] = 1.0
    return values


class Mock(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    calls = 0
    lock = threading.Lock()

    def log_message(self, *_):
        pass

    def respond(self, value):
        data = encode(value).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        assert self.path == "/counts"
        self.respond({"embeddings": self.calls})

    def do_POST(self):
        assert self.path == "/models/gemini-embedding-2:batchEmbedContents", self.path
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        embeddings = []
        for request in body["requests"]:
            text = request["content"]["parts"][0]["text"]
            match = re.fullmatch(r"(?:marker|semantic)(\d{6})", text)
            assert match and int(match[1]) < COUNT, text
            embeddings.append({"values": vector(int(match[1]))})
        with self.lock:
            type(self).calls += 1
        self.respond({"embeddings": embeddings})


class Redis:
    def __init__(self, host):
        self.sock = socket.create_connection((host, 16379), timeout=15)
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
        raise RuntimeError(f"invalid Redis response: {line[:100]!r}")

    def call(self, *args):
        parts = [str(value).encode() for value in args]
        self.sock.sendall(b"*%d\r\n" % len(parts) + b"".join(
            b"$%d\r\n" % len(part) + part + b"\r\n" for part in parts))
        return self.read()

    def query(self, text):
        return self.call("GRAPH.QUERY", GROUP, text, "TIMEOUT", 5000)


def host_memory():
    values = {line.split(":")[0]: int(line.split()[1])
              for line in Path("/proc/meminfo").read_text().splitlines()}
    return {key + "_KiB": values[key] for key in ("MemAvailable", "SwapTotal", "SwapFree")}


def metrics(pid):
    status = Path(f"/proc/{pid}/status").read_text()
    result = {line.split(":")[0] + "_KiB": int(line.split()[1])
              for line in status.splitlines() if line.startswith(("VmRSS:", "VmHWM:"))}
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    result["cpu_seconds"] = (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")
    return result


def canonical(value):
    if isinstance(value, dict):
        return {key: canonical(item) for key, item in value.items()}
    if isinstance(value, list):
        return [canonical(item) for item in value]
    if isinstance(value, str) and re.match(r"^2026-\d\d-\d\dT", value):
        return datetime.datetime.fromisoformat(value.replace("Z", "+00:00")).isoformat()
    return value


class Client:
    def __init__(self, host):
        self.conn = http.client.HTTPConnection(host, 18080, timeout=10)
        self.serial = 0

    def rpc(self, method, params):
        self.serial += 1
        headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream",
                   "Authorization": f"Bearer {TOKEN}", "Host": "localhost:18080",
                   "MCP-Protocol-Version": "2025-11-25", "Mcp-Method": method}
        if method == "tools/call":
            headers["Mcp-Name"] = params["name"]
        self.conn.request("POST", "/mcp", encode({"jsonrpc": "2.0", "id": self.serial,
                          "method": method, "params": params}), headers)
        response = self.conn.getresponse()
        data = response.read()
        assert response.status == 200, (response.status, data[:500])
        if response.getheader("Content-Type", "").startswith("text/event-stream"):
            data = next(line[6:] for line in data.splitlines() if line.startswith(b"data: "))
        result = json.loads(data)
        assert "error" not in result, result
        return result["result"]

    def tool(self, name, args):
        result = self.rpc("tools/call", {"name": name, "arguments": args})
        assert not result.get("isError"), result
        value = result.get("structuredContent")
        if value is None:
            value = json.loads(result["content"][0]["text"])
        while isinstance(value, dict) and set(value) == {"result"}:
            value = value["result"]
        assert "error" not in value, value
        return canonical(value)

    def close(self):
        self.conn.close()


class Experiment:
    def __init__(self, root):
        self.root = root
        self.tag = "grs-paired-" + uuid.uuid4().hex[:8]
        self.network = self.tag + "-net"
        self.containers = []
        self.report = {"status": "preparing", "date_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "marker": MARKER, "tag": self.tag, "host": {"model": Path("/proc/device-tree/model").read_text().strip("\x00"),
            "architecture": platform.machine(), "kernel": platform.release(), "memory_before": host_memory()},
            "python_image": PYTHON_IMAGE, "rust_image": RUST_IMAGE,
            "rust_source_commit": "8df990467bab7f2c53b0ab3fbbafa9ef7e7f9c4b",
            "scope": "matched ten-tool MCP read/hybrid-search profile; no ingestion or real model inference",
            "corpus": {"facts": COUNT, "entities": COUNT + 1, "episodes": 40, "dimensions": 3072,
                       "vector_rule": "orthogonal unit basis i per fact/person, hub has no vector"},
            "limits": {"server_cpus": 1, "server_memory_MiB": 768, "db_cpus": 1, "db_memory_MiB": 512,
                       "mock_cpus": .25, "mock_memory_MiB": 128, "swap_in_containers": False},
            "design": {"rounds": 3, "orders": [["python", "rust"], ["rust", "python"], ["python", "rust"]],
                       "samples_per_workload_per_round": 100, "warmup_per_workload": 10, "concurrency": 1,
                       "latency": "same Pi host HTTP client to private container IP; same namespace/port, no relay",
                       "memory": "server process /proc VmRSS and VmHWM; DB and mock measured separately"},
            "actual_provider_calls": 0, "production_mutations": False, "runs": []}
        (root / "benchmark-passwd").write_text(
            f"root:x:0:0:root:/root:/bin/sh\nbenchmark:x:{os.getuid()}:{os.getgid()}:Synthetic benchmark:/tmp/benchmark:/bin/sh\n")
        token_file = root / "backend-token"
        token_file.write_text(TOKEN)
        token_file.chmod(0o600)
        self.started = time.monotonic()
        self.save()

    def save(self):
        temporary = self.root / "report.tmp"
        temporary.write_text(json.dumps(self.report, indent=2) + "\n")
        temporary.replace(self.root / "report.json")

    def progress(self, message):
        assert time.monotonic() - self.started < 1100, "overall benchmark deadline"
        assert host_memory()["MemAvailable_KiB"] > 512 * 1024, "host headroom below 512 MiB"
        print(message, flush=True)
        self.save()

    def docker(self, *args, timeout=30):
        result = subprocess.run(["docker", *args], text=True, capture_output=True, timeout=timeout)
        if result.returncode:
            raise RuntimeError(f"docker {args[:2]}: {result.stderr[:2000]}")
        return (result.stdout + result.stderr if args[0] == "logs" else result.stdout).strip()

    def inspect(self, name):
        return json.loads(self.docker("inspect", name))[0]

    def container(self, suffix, image, argv, memory, cpus, network=None, mounts=()):
        name = self.tag + "-" + suffix
        args = ["run", "--detach", "--name", name, "--label", f"task={self.tag}",
            "--network", network or self.network, "--read-only", "--cap-drop", "ALL",
            "--security-opt", "no-new-privileges:true", "--pids-limit", "128", "--cpus", str(cpus),
            "--memory", f"{memory}m", "--memory-swap", f"{memory}m", "--user", f"{os.getuid()}:{os.getgid()}",
            "--tmpfs", "/tmp:rw,nosuid,nodev,size=32m,mode=1777"]
        args.extend(["--mount", f"type=bind,src={self.root / 'benchmark-passwd'},dst=/etc/passwd,readonly"])
        args.extend(mounts)
        args.extend(["--entrypoint", argv[0], image, *argv[1:]])
        self.docker(*args)
        self.containers.append(name)
        return name

    def remove(self, name):
        info = self.inspect(name)
        assert info["Config"]["Labels"]["task"] == self.tag
        log = self.docker("logs", name)
        (self.root / (name + ".log")).write_text(log)
        for line in log.splitlines():
            if line.startswith('{"embedding_dimension"') or '"kind": "paired_python_mcp_metadata"' in line:
                self.report["python_runtime"] = json.loads(line)
        self.docker("rm", "--force", name)
        self.containers.remove(name)

    def prepare(self):
        self.docker("network", "create", "--internal", "--label", f"task={self.tag}", self.network)
        uid, gid = os.getuid(), os.getgid()
        self.db = self.container("db", PYTHON_IMAGE, ["redis-server", "--loadmodule", "/var/lib/falkordb/bin/falkordb.so",
            "--port", "16379", "--bind", "0.0.0.0", "--protected-mode", "no", "--dir", "/data",
            "--save", "", "--appendonly", "no", "--maxmemory", "384mb"], 512, 1,
            mounts=["--tmpfs", f"/data:rw,nosuid,nodev,size=256m,uid={uid},gid={gid},mode=0700"])
        self.ip = self.inspect(self.db)["NetworkSettings"]["Networks"][self.network]["IPAddress"]
        for _ in range(60):
            try:
                self.redis = Redis(self.ip)
                assert self.redis.call("PING") == "PONG"
                break
            except (OSError, AssertionError):
                time.sleep(.5)
        else:
            raise RuntimeError("disposable DB not ready within 30 seconds")
        self.report["db_modules"] = self.redis.call("MODULE", "LIST")
        self.progress("DB ready; seeding exclusively synthetic graph")
        ep = [{"id": identity(300000 + i), "edges": [identity(100000 + j) for j in range(i * 30, (i + 1) * 30)]}
              for i in range(40)]
        self.redis.query(f'CYPHER rows={cypher(ep)} UNWIND $rows AS r CREATE(e:Episodic {{uuid:r.id,name:"Synthetic benchmark episode",content:"Synthetic benchmark content",source:"text",source_description:"paired Pi benchmark",group_id:"{GROUP}",created_at:"{STAMP}",valid_at:"{STAMP}",entity_edges:r.edges}})')
        self.redis.query(f'CREATE (:Entity {{uuid:"{identity(999999)}",name:"Synthetic Hub",summary:"Fixture",labels:["Entity"],group_id:"{GROUP}",created_at:"{STAMP}"}})')
        for first in range(0, COUNT, 10):
            rows = [{"id": identity(i + 100000), "nid": identity(i + 200000), "name": f"Person marker{i:06d}",
                     "fact": f"marker{i:06d} synthetic employment", "ep": identity(300000 + i // 30), "vector": vector(i)}
                    for i in range(first, first + 10)]
            self.redis.query(f'CYPHER rows={cypher(rows)} UNWIND $rows AS r MATCH(t:Entity {{uuid:"{identity(999999)}"}}) CREATE(n:Entity:Person {{uuid:r.nid,name:r.name,summary:"Fixture",labels:["Entity","Person"],group_id:"{GROUP}",created_at:"{STAMP}"}}) CREATE(n)-[e:RELATES_TO {{uuid:r.id,name:"WORKS_AT",fact:r.fact,group_id:"{GROUP}",created_at:"{STAMP}",valid_at:"{STAMP}",episodes:[r.ep]}}]->(t) SET n.name_embedding=vecf32(r.vector),e.fact_embedding=vecf32(r.vector)')
            if first % 200 == 0:
                self.progress(f"seeded {first + 10}/{COUNT} facts")
        for query in ["CALL db.idx.fulltext.createNodeIndex('Entity','name','summary','group_id')",
                      "CREATE FULLTEXT INDEX FOR ()-[e:RELATES_TO]-() ON (e.name,e.fact,e.group_id)",
                      "CREATE INDEX FOR (n:Entity) ON (n.uuid, n.group_id)",
                      "CREATE INDEX FOR (n:Episodic) ON (n.uuid, n.group_id)"]:
            self.redis.query(query)
        for _ in range(120):
            result = self.redis.query("CALL db.indexes() YIELD status RETURN status")
            if result[1] and all(row[0] == "OPERATIONAL" for row in result[1]):
                break
            time.sleep(.5)
        else:
            raise RuntimeError("fixture indexes not operational within 60 seconds")
        self.report["corpus"]["identity_sha256"] = hashlib.sha256(encode(self.redis.query(
            "MATCH(n) RETURN n.uuid,n.name ORDER BY n.uuid")).encode()).hexdigest()
        self.mount = ["--mount", f"type=bind,src={self.root},dst=/bench,readonly"]
        self.mock = self.container("mock", PYTHON_IMAGE,
            ["/usr/bin/env", "-i", "PATH=/usr/local/bin:/usr/bin:/bin", "BENCH_MARKER=" + MARKER,
             "/app/mcp/.venv/bin/python", "-S", "/bench/scripts/bench-paired-pi.py", "--mock"],
            128, .25, network="container:" + self.db, mounts=self.mount)
        for _ in range(60):
            try:
                self.mock_count()
                break
            except OSError:
                time.sleep(.2)
        self.progress("fixture/indexes/mock ready; beginning paired server runs")

    def mock_count(self):
        connection = http.client.HTTPConnection(self.ip, 18081, timeout=3)
        connection.request("GET", "/counts")
        result = json.loads(connection.getresponse().read())["embeddings"]
        connection.close()
        return result

    def start_server(self, kind, trial):
        common = ["/usr/bin/env", "-i", "PATH=/app/mcp/.venv/bin:/usr/local/bin:/usr/bin:/bin", "LANG=C.UTF-8"]
        mounts = list(self.mount)
        if kind == "python":
            argv = common + [f"BENCH_MARKER={MARKER}", f"BENCH_GROUP={GROUP}",
                "BENCH_DB_URL=redis://127.0.0.1:16379", "BENCH_MOCK_BASE=http://127.0.0.1:18081",
                "POSTHOG_DISABLED=true", "ANONYMIZED_TELEMETRY=false", "SEMAPHORE_LIMIT=2",
                "OMP_NUM_THREADS=1", "OPENBLAS_NUM_THREADS=1", "MKL_NUM_THREADS=1",
                "/app/mcp/.venv/bin/python", "-S", "/bench/scripts/bench-python-mcp.py"]
            image = PYTHON_IMAGE
        else:
            journal = self.root / f"journal-{self.tag}-{trial}"
            journal.mkdir(mode=0o700)
            mounts.extend(["--mount", f"type=bind,src={journal},dst=/journal"])
            argv = common + ["GRAPHITI_BIND=0.0.0.0:18080", "COMPAT_TOKEN_FILE=/bench/backend-token",
                "HTTP_ALLOWED_HOSTS=localhost:18080", "COMPAT_DB_URL=redis://127.0.0.1:16379",
                "COMPAT_WRITE_MODE=synthetic", "COMPAT_JOURNAL_DIR=/journal", "COMPAT_MAX_TOKENS=512",
                "COMPAT_SEARCH_CANDIDATES=200", "COMPAT_LLM_BASE=http://127.0.0.1:18081",
                "COMPAT_GEMINI_BASE=http://127.0.0.1:18081", "COMPAT_GEMINI_TRANSPORT=native",
                "RUST_LOG=warn", "/usr/local/bin/graphiti-rs", "--storage-format", "graphiti-0282", "--transport", "http"]
            image = RUST_IMAGE
        began = time.monotonic()
        name = self.container(f"{kind}-{trial}", image, argv, 768, 1,
                              network="container:" + self.db, mounts=mounts)
        last_error = None
        for _ in range(120):
            info = self.inspect(name)
            if not info["State"]["Running"]:
                raise RuntimeError(f"server exited: {self.docker('logs', name)}")
            client = Client(self.ip)
            try:
                client.rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                    "clientInfo": {"name": "paired-pi-benchmark", "version": "1"}})
                tools = client.rpc("tools/list", {})["tools"]
                assert len(tools) == 10, len(tools)
                if kind == "rust":
                    self.report["rust_binary_sha256"] = self.docker("exec", name, "sha256sum", "/usr/local/bin/graphiti-rs").split()[0]
                return name, info["State"]["Pid"], client, time.monotonic() - began
            except (OSError, AssertionError, ValueError) as error:
                client.close()
                last_error = error
                time.sleep(.25)
        raise RuntimeError(f"MCP readiness failed: {last_error}")

    def run(self, kind, trial, reference):
        self.progress(f"starting round {trial + 1} {kind}")
        name, pid, client, startup = self.start_server(kind, trial)
        record = {"kind": kind, "round": trial + 1, "startup_to_ready_seconds": startup, "workloads": {}}
        self.report["runs"].append(record)
        initial = metrics(pid)
        time.sleep(2)
        idle = metrics(pid)
        record["idle"] = {**idle, "cpu_seconds_over_2s": idle["cpu_seconds"] - initial["cpu_seconds"]}
        order = random.Random(20261015).sample(range(COUNT), 100)
        definitions = [
            ("episode_read", "get_episode", lambda i: {"uuid": identity(300000 + i % 40), "group_id": GROUP}),
            ("hybrid_fact_search", "search_memory_facts", lambda i: {"query": f"marker{i:06d}", "group_ids": [GROUP], "max_facts": 10}),
            ("semantic_node_search", "search_nodes", lambda i: {"query": f"semantic{i:06d}", "group_ids": [GROUP], "max_nodes": 10})]
        for workload, tool, arguments in definitions:
            self.progress(f"round {trial + 1} {kind}: {workload}")
            for i in order[:10]:
                client.tool(tool, arguments(i))
            cpu = metrics(pid)["cpu_seconds"]
            db_cpu = metrics(self.inspect(self.db)["State"]["Pid"])["cpu_seconds"]
            mock_before = self.mock_count()
            samples, observations = [], []
            for index, i in enumerate(order):
                began = time.perf_counter()
                result = client.tool(tool, arguments(i))
                samples.append((time.perf_counter() - began) * 1000)
                if workload == "episode_read":
                    assert result["found"] and result["episode"]["uuid"] == identity(300000 + i % 40), result
                else:
                    key, expected = ("facts", identity(100000 + i)) if workload == "hybrid_fact_search" else ("nodes", identity(200000 + i))
                    assert [item["uuid"] for item in result[key]] == [expected], result
                observations.append(result)
                if index % 25 == 0:
                    self.progress(f"round {trial + 1} {kind}: {workload} {index}/100")
            expected_calls = 0 if workload == "episode_read" else 100
            actual_calls = self.mock_count() - mock_before
            assert actual_calls == expected_calls, (kind, workload, actual_calls)
            observed_hash = hashlib.sha256(encode(observations).encode()).hexdigest()
            if workload in reference:
                assert reference[workload] == observed_hash, (kind, workload, "response parity failure")
            else:
                reference[workload] = observed_hash
            final = metrics(pid)
            ordered = sorted(samples)
            record["workloads"][workload] = {"samples": len(samples), "warmup": 10,
                "p50_ms": statistics.median(samples), "p95_ms": ordered[94], "p99_ms": ordered[98],
                "latencies_ms": samples, "server_cpu_seconds": final["cpu_seconds"] - cpu,
                "db_cpu_seconds": metrics(self.inspect(self.db)["State"]["Pid"])["cpu_seconds"] - db_cpu,
                "server_after": final, "db_after": metrics(self.inspect(self.db)["State"]["Pid"]),
                "mock_embedding_calls": actual_calls, "response_sha256": observed_hash,
                "successes": 100, "errors": 0}
            self.save()
        record["final"] = metrics(pid)
        record["mock_after"] = metrics(self.inspect(self.mock)["State"]["Pid"])
        record["container_oom_killed"] = self.inspect(name)["State"]["OOMKilled"]
        client.close()
        self.remove(name)
        time.sleep(1)

    def cleanup(self):
        errors = []
        for name in list(reversed(self.containers)):
            try:
                self.remove(name)
            except Exception as error:
                errors.append(str(error))
        try:
            info = json.loads(self.docker("network", "inspect", self.network))[0]
            assert info["Labels"]["task"] == self.tag
            self.docker("network", "rm", self.network)
        except Exception as error:
            errors.append(str(error))
        self.report["cleanup_errors"] = errors
        self.report["host"]["memory_after"] = host_memory()
        self.save()


def main():
    assert platform.system() == "Linux" and platform.machine() == "aarch64", "Pi ARM64 only; never benchmark on Mac"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path)
    parser.add_argument("--mock", action="store_true")
    args = parser.parse_args()
    if args.mock:
        assert os.environ.get("BENCH_MARKER") == MARKER
        http.server.ThreadingHTTPServer(("0.0.0.0", 18081), Mock).serve_forever()
        return
    root = args.root.resolve()
    assert root.parent == Path("/home/pi4-3") and root.name.startswith("graphiti-paired-bench-20261015-")
    assert (root / "OWNED").read_text().strip() == MARKER
    assert root.stat().st_uid == os.getuid() and root.stat().st_mode & 0o077 == 0
    experiment = Experiment(root)
    def interrupted(number, _):
        raise SystemExit(f"interrupted by signal {number}")
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGHUP, interrupted)
    try:
        experiment.prepare()
        reference = {}
        for trial, order in enumerate(experiment.report["design"]["orders"]):
            for kind in order:
                experiment.run(kind, trial, reference)
        experiment.report["response_parity"] = reference
        experiment.report["status"] = "completed"
        experiment.progress("all paired requests passed with identical canonical responses")
    except BaseException as error:
        experiment.report["status"] = "failed"
        experiment.report["error"] = str(error)
        raise
    finally:
        experiment.cleanup()


if __name__ == "__main__":
    main()
