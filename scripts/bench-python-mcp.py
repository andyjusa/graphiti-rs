#!/usr/bin/env python3
"""TEST ONLY: serve the preserved Python MCP wrapper for paired Pi read benchmarks.

Run only in the owned, isolated arm64 Pi container, under env -i and python -S.
The orchestrator seeds its disposable database and indexes before launch. This
entrypoint never calls upstream main/initialize_server, loads .env files, builds
indexes, ingests data, or constructs provider SDK clients. Keep its container in
the owned DB network namespace; port 18080 is not an authenticated public API.
"""

import asyncio
import hashlib
import importlib.metadata
import json
import logging
import math
import os
from pathlib import Path
import platform
import runpy
import sys
from types import SimpleNamespace


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


require(sys.flags.no_site, "requires python -S")
require("sitecustomize" not in sys.modules, "sitecustomize must not be loaded")
require(sys.platform == "linux" and platform.machine() in {"aarch64", "arm64"},
        "requires the isolated arm64 Pi container; do not run on the Mac")
require(os.environ.get("BENCH_MARKER") == "graphiti-paired-pi-20261015",
        "requires the paired benchmark marker")

# Retain only synthetic benchmark inputs, before any third-party imports. No
# operational API key, proxy, config location or Python startup env survives.
bench_env = {key: os.environ[key] for key in
             ("BENCH_MARKER", "BENCH_DB_URL", "BENCH_GROUP", "BENCH_MOCK_BASE")
             if key in os.environ}
os.environ.clear()
os.environ.update(bench_env)
os.environ.update({
    "GRAPHITI_TELEMETRY_ENABLED": "false",
    "POSTHOG_DISABLED": "true",
    "DO_NOT_TRACK": "1",
    "OTEL_SDK_DISABLED": "true",
    "EMBEDDING_DIM": "3072",
    "SEMAPHORE_LIMIT": "2",
    "OMP_NUM_THREADS": "1",
    "OPENBLAS_NUM_THREADS": "1",
    "MKL_NUM_THREADS": "1",
    "PYTHONDONTWRITEBYTECODE": "1",
})
sys.dont_write_bytecode = True
Path("/tmp/benchmark").mkdir(mode=0o700, exist_ok=True)

GROUP = os.environ.get("BENCH_GROUP", "project_paired_bench_20261015")
DB_URL = os.environ.get("BENCH_DB_URL", "redis://127.0.0.1:16379")
MOCK_BASE = os.environ.get("BENCH_MOCK_BASE", "http://127.0.0.1:18081").rstrip("/")
require(GROUP == "project_paired_bench_20261015", "unowned benchmark group refused")
require(DB_URL == "redis://127.0.0.1:16379", "requires the owned loopback DB relay")
require(MOCK_BASE == "http://127.0.0.1:18081", "requires the owned loopback mock")

APP = Path("/app/mcp")
SITE = APP / ".venv/lib/python3.11/site-packages"
PATCHES = APP / "patches"
sys.path[:0] = [str(PATCHES), str(APP / "src"), str(APP), str(SITE)]

# Both import styles must resolve to the disabled function. In particular,
# graphiti_mcp_server imports load_dotenv directly at module scope.
import dotenv
import dotenv.main


def no_dotenv(*args, **kwargs):
    return False


dotenv.load_dotenv = no_dotenv
dotenv.main.load_dotenv = no_dotenv

import httpx
from graphiti_core import Graphiti
from graphiti_core.cross_encoder.client import CrossEncoderClient
from graphiti_core.driver.falkordb_driver import FalkorDriver
from graphiti_core.embedder.client import EmbedderClient
from graphiti_core.llm_client.client import LLMClient
from graphiti_core.llm_client.config import LLMConfig

require(importlib.metadata.version("graphiti-core") == "0.28.2",
        "requires preserved graphiti-core 0.28.2")


class MockLLM(LLMClient):
    def __init__(self):
        super().__init__(LLMConfig(model="paired-benchmark-forbidden-llm"))

    async def _generate_response(self, messages, response_model=None,
                                 max_tokens=None, model_size=None):
        raise RuntimeError("LLM calls are forbidden in the read benchmark")


class MockHTTPEmbedder(EmbedderClient):
    """Use the same Gemini-shaped mock request as the Rust transport."""

    def __init__(self):
        self.http = None

    async def create(self, input_data):
        if isinstance(input_data, str):
            text = input_data
        elif isinstance(input_data, list) and len(input_data) == 1 and isinstance(input_data[0], str):
            # The actual 0.28.2 search path supplies [query].
            text = input_data[0]
        else:
            raise ValueError("read benchmark expects one string per embedding")
        return (await self.create_batch([text]))[0]

    async def create_batch(self, input_data_list):
        require(isinstance(input_data_list, list) and
                all(isinstance(text, str) for text in input_data_list),
                "embedding batch must contain strings")
        if not input_data_list:
            return []
        require(self.http is not None, "mock HTTP client is not initialized")
        response = await self.http.post(
            MOCK_BASE + "/models/gemini-embedding-2:batchEmbedContents",
            json={"requests": [
                {"model": "models/gemini-embedding-2",
                 "content": {"parts": [{"text": text}]},
                 "outputDimensionality": 3072}
                for text in input_data_list
            ]},
        )
        response.raise_for_status()
        payload = response.json()
        embeddings = payload.get("embeddings") if isinstance(payload, dict) else None
        require(isinstance(embeddings, list) and len(embeddings) == len(input_data_list),
                "mock returned an unexpected embedding count")
        vectors = []
        for item in embeddings:
            values = item.get("values") if isinstance(item, dict) else None
            require(isinstance(values, list) and len(values) == 3072 and
                    all(type(value) in (int, float) and math.isfinite(value) for value in values),
                    "mock returned an invalid 3072-dimensional vector")
            vectors.append(values)
        return vectors


class NoRanking(CrossEncoderClient):
    async def rank(self, query, passages):
        raise RuntimeError("cross-encoder calls are forbidden in the RRF read benchmark")


# Import the actual deployed wrapper, including all ten actual tool contracts.
# Its __main__ block is not run; no private YAML config or startup path is used.
import hermes_main
import group_read_routing

upstream = hermes_main.upstream
group_read_routing.ensure_readable_group(GROUP)
edge_patch = PATCHES / "falkor_edge_query_compat.py"
runpy.run_path(str(edge_patch))["install"]()

# Construct OUTSIDE a running event loop: FalkorDriver.__init__ otherwise
# schedules build_indices_and_constraints. The real scoped-driver adapter
# returns this same driver for GROUP, so read routing creates no other driver.
driver = FalkorDriver(host="127.0.0.1", port=16379, database=GROUP)
embedder = MockHTTPEmbedder()
client = Graphiti(graph_driver=driver, llm_client=MockLLM(), embedder=embedder,
                  cross_encoder=NoRanking(), max_coroutines=2)
require(group_read_routing.driver_for_group(driver, GROUP) is driver,
        "read routing unexpectedly cloned the benchmark driver")

service = upstream.GraphitiService.__new__(upstream.GraphitiService)
service.config = SimpleNamespace(database=SimpleNamespace(provider="falkordb"))
service.semaphore_limit = 2
service.semaphore = asyncio.Semaphore(2)
service.client = client
service.entity_types = None
upstream.graphiti_service = service
upstream.graphiti_client = client
upstream.config = SimpleNamespace(graphiti=SimpleNamespace(group_id=GROUP))
upstream.semaphore = service.semaphore

upstream.mcp.settings.host = "0.0.0.0"
upstream.mcp.settings.port = 18080
upstream.mcp.settings.stateless_http = True
upstream.mcp.settings.json_response = True
upstream.mcp.settings.log_level = "WARNING"
logging.getLogger().setLevel(logging.WARNING)
for logger_name in ("uvicorn", "uvicorn.error", "uvicorn.access", "httpx", "httpcore"):
    logging.getLogger(logger_name).setLevel(logging.WARNING)


def source_metadata():
    paths = {
        "benchmark_entrypoint": Path(__file__),
        "graphiti_core": Path(sys.modules["graphiti_core.graphiti"].__file__),
        "mcp_server": Path(upstream.__file__),
        "hermes_wrapper": Path(hermes_main.__file__),
        "edge_patch": edge_patch,
        "compat_patch": PATCHES / "graphiti_0282_compat.py",
        "group_read_routing": Path(group_read_routing.__file__),
    }
    return {name: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
            for name, path in paths.items()}


async def main():
    tools = await upstream.mcp.list_tools()
    require(len(tools) == 10, "preserved wrapper must expose exactly ten tools")
    require(await service.get_client() is client, "service would initialize another client")
    async with httpx.AsyncClient(trust_env=False, timeout=3.0) as http:
        embedder.http = http
        print(json.dumps({
            "kind": "paired_python_mcp_metadata",
            "test_only": True,
            "versions": {package: importlib.metadata.version(package)
                         for package in ("graphiti-core", "mcp", "httpx", "falkordb", "redis")},
            "sources": source_metadata(),
            "tools": [tool.name for tool in tools],
            "transport": {"path": upstream.mcp.settings.streamable_http_path,
                          "stateless_http": True, "json_response": True},
            "startup_initialization": False,
            "embedding_dimension": 3072,
            "max_coroutines": 2,
        }, sort_keys=True), flush=True)
        try:
            await upstream.mcp.run_streamable_http_async()
        finally:
            embedder.http = None
            await driver.close()


if __name__ == "__main__":
    asyncio.run(main())
