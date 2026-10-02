#!/usr/bin/env python3
"""Real observed SDK fulltext baseline on the labelled synthetic Pi fixture.
Run env -i/python -S inside the owned image, never the app entrypoint.
"""
import asyncio
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import runpy
import sys

assert os.environ.get('GRAPHITI_PI_SYNTHETIC') == 'graphiti-rs-compat-pi'
sys.path.insert(0, '/app/mcp/.venv/lib/python3.11/site-packages')
from graphiti_core.driver.falkordb_driver import FalkorDriver
from graphiti_core.search.search_filters import SearchFilters
from graphiti_core.search.search_utils import edge_fulltext_search, node_fulltext_search

GROUP = 'project_pi_fulltext_20261002'
QUERIES = ['cat', 'the cat', 'cats', 'CAT?', 'cat:missingterm', '"cat"']

async def main():
    assert importlib.metadata.version('graphiti-core') == '0.28.2'
    patch = Path('/tmp/falkor_edge_query_compat.py')
    assert hashlib.sha256(patch.read_bytes()).hexdigest() == '7abd25af31520e3951625c7a3739033bf9469b0d466c592243d9eaa8b3153939'
    runpy.run_path(str(patch))['install']()
    driver = FalkorDriver(host='127.0.0.1', port=6379, database=GROUP)
    try:
        rows, _, _ = await driver.execute_query('MATCH ()-[e:RELATES_TO]->() RETURN count(e) AS count')
        assert rows[0]['count'] == 1200
        results=[]
        for query in QUERIES:
            facts = await edge_fulltext_search(driver, query, SearchFilters(), [GROUP], 10)
            nodes = await node_fulltext_search(driver, query, SearchFilters(node_labels=['Person']), [GROUP], 10)
            results.append({'query':query,'fact_ids':[f.uuid for f in facts],'node_ids':[n.uuid for n in nodes]})
        print(json.dumps({'package':'graphiti-core 0.28.2','scope':'real fulltext query/search bodies with observed edge patch; no LLM/embeddings/ingestion parity','cases':results,'actual_provider_calls':0},indent=2))
    finally:
        await driver.close()

if __name__=='__main__': asyncio.run(main())
