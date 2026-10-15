#!/usr/bin/env python3
"""Private native-vector replay using the observed SDK and deployed wrappers.

Run only inside the owned read-only original restoration. Writes a private
fixture under /tmp, never original text/vectors to stdout. No provider clients.
"""
import ast
import asyncio
import collections
import copy
import hashlib
import json
import logging
import os
import sys
from pathlib import Path
from types import SimpleNamespace
from typing import Any

assert os.environ.get('GRAPHITI_PRIVATE_READ_REHEARSAL') == 'original-restore-read-only'
sys.path[:0] = ['/tmp/read-source', '/app/mcp/.venv/lib/python3.11/site-packages', '/app/mcp/src']
logging.disable(logging.CRITICAL)
from redis import Redis
from redis.asyncio import Redis as AsyncRedis
from falkordb.asyncio.graph import AsyncGraph
from graphiti_core.driver.falkordb_driver import FalkorDriver
from graphiti_core.helpers import validate_group_id
from graphiti_core.search.search import search as sdk_search
from graphiti_core.search.search_filters import SearchFilters
from graphiti_core.search.search_config_recipes import EDGE_HYBRID_SEARCH_RRF, EDGE_HYBRID_SEARCH_NODE_DISTANCE
from models.response_types import NodeResult, NodeSearchResponse, FactSearchResponse, ErrorResponse
from utils.formatting import format_fact_result
from group_read_routing import *
from falkor_edge_query_compat import install

readonly_queries = 0
redis_execute = AsyncRedis.execute_command
async def force_ro(self, *args, **kwargs):
    global readonly_queries
    command = args[0].decode() if isinstance(args[0], bytes) else str(args[0])
    command = command.upper()
    if command == 'GRAPH.QUERY':
        args = ('GRAPH.RO_QUERY', *args[1:]); command = 'GRAPH.RO_QUERY'
    assert command in {'GRAPH.RO_QUERY', 'GRAPH.LIST', 'PING', 'CLIENT', 'INFO', 'COMMAND'}, 'non-read command refused'
    if command == 'GRAPH.RO_QUERY': readonly_queries += 1
    return await redis_execute(self, *args, **kwargs)
AsyncRedis.execute_command = force_ro
install()
# All driver construction occurs before a running event loop: no index task.
graph_names = Redis(host='127.0.0.1', port=6379).execute_command('GRAPH.LIST')
all_groups = sorted(g.decode() if isinstance(g, bytes) else g for g in graph_names)
groups = [g for g in all_groups if is_owner_group_authorized(g)]
drivers = {g: FalkorDriver(host='127.0.0.1', port=6379, database=g) for g in groups}
assert all(type(d._get_graph(g)) is AsyncGraph and isinstance(d.client.connection, AsyncRedis)
           for g, d in drivers.items())

class Forbidden:
    async def create(self, *args, **kwargs): raise AssertionError('provider forbidden')
    async def rank(self, *args, **kwargs): raise AssertionError('provider forbidden')
class PinnedDrivers:
    _database = None
    def clone(self, database): return drivers[database]
class Client:
    driver = PinnedDrivers()
    vector = None
    async def search_(self, query, config, group_ids, search_filter, driver):
        clients = SimpleNamespace(driver=driver, embedder=Forbidden(), cross_encoder=Forbidden())
        return await sdk_search(clients, query, group_ids, config, search_filter,
                                query_vector=self.vector, driver=driver)
    async def search(self, query, group_ids, num_results, center_node_uuid, driver):
        config = copy.deepcopy(EDGE_HYBRID_SEARCH_NODE_DISTANCE if center_node_uuid else EDGE_HYBRID_SEARCH_RRF)
        config.limit = num_results
        clients = SimpleNamespace(driver=driver, embedder=Forbidden(), cross_encoder=Forbidden())
        result = await sdk_search(clients, query, group_ids, config, SearchFilters(),
            center_node_uuid=center_node_uuid, query_vector=self.vector, driver=driver)
        return result.edges
client = Client()
class Service:
    async def get_client(self): return client
upstream = SimpleNamespace(graphiti_service=Service(), NodeResult=NodeResult,
    NodeSearchResponse=NodeSearchResponse, FactSearchResponse=FactSearchResponse,
    ErrorResponse=ErrorResponse, format_fact_result=format_fact_result,
    logger=logging.getLogger('private-search'),
    config=SimpleNamespace(graphiti=SimpleNamespace(group_id='user_primary')))
source_path = Path('/tmp/read-source/hermes_main.py')
source = source_path.read_text()
namespace = dict(globals())
tree = ast.parse(source)
for name in ['_search_nodes_group_aware', '_search_memory_facts_group_aware']:
    wrapper = next(n for n in tree.body if isinstance(n, ast.AsyncFunctionDef) and n.name == name)
    exec(compile(ast.Module(body=[wrapper], type_ignores=[]), 'observed-search-wrapper', 'exec'), namespace)

async def main():
    cases = []
    selected = {}
    async def add(tool, args, vector, category):
        assert len(vector) == 3072
        client.vector = vector
        wrapper = namespace['_search_nodes_group_aware' if tool == 'search_nodes' else '_search_memory_facts_group_aware']
        expected = await wrapper(**args)
        cases.append(dict(tool=tool, args=args, vector=vector, expected=expected, category=category))
    for group in groups:
        driver = drivers[group]
        nodes, _, _ = await driver.execute_query(
            'MATCH(n:Entity) WHERE n.group_id=$group AND n.name_embedding IS NOT NULL RETURN n.name AS name,n.name_embedding AS vector,labels(n) AS labels,n.uuid AS uuid ORDER BY n.uuid LIMIT 1', group=group)
        facts, _, _ = await driver.execute_query(
            'MATCH()-[e:RELATES_TO]->() WITH e,startNode(e) AS n,endNode(e) AS m WHERE e.group_id=$group AND n.group_id=$group AND m.group_id=$group AND e.fact_embedding IS NOT NULL RETURN e.fact AS fact,e.fact_embedding AS vector,n.uuid AS center ORDER BY e.uuid LIMIT 1', group=group)
        if nodes:
            node = nodes[0]; selected[group] = dict(node=node)
            await add('search_nodes', dict(query=node['name'][:512], group_ids=[group], max_nodes=10), node['vector'], 'canonical_node')
        if facts:
            fact = facts[0]; selected.setdefault(group, {})['fact'] = fact
            await add('search_memory_facts', dict(query=' '.join(fact['fact'].split()[:8])[:512], group_ids=[group], max_facts=10), fact['vector'], 'canonical_fact')
    # Bounded extensions of the same captured-vector cohort, not new embeddings.
    representatives = [g for g in ['user_primary', next((g for g in groups if g.startswith('project_') and 'fact' in selected.get(g, {})), '')]
                       if g in selected]
    for group in representatives:
        node = selected[group].get('node'); fact = selected[group].get('fact')
        if node:
            for limit in [1, 25, 100]:
                await add('search_nodes', dict(query=node['name'][:512], group_ids=[group], max_nodes=limit), node['vector'], 'node_window')
            for labels in [[x for x in node['labels'] if x != 'Entity'][:1], ['SyntheticAbsentType']]:
                await add('search_nodes', dict(query=node['name'][:512], group_ids=[group], max_nodes=10, entity_types=labels), node['vector'], 'type_filter')
        if fact:
            for limit in [1, 25, 100]:
                await add('search_memory_facts', dict(query=' '.join(fact['fact'].split()[:8])[:512], group_ids=[group], max_facts=limit), fact['vector'], 'fact_window')
            await add('search_memory_facts', dict(query=' '.join(fact['fact'].split()[:8])[:512], group_ids=[group], max_facts=10, center_node_uuid=fact['center']), fact['vector'], 'center_distance')
    if len(representatives) == 2:
        node = selected[representatives[0]].get('node'); fact = selected[representatives[0]].get('fact')
        if node:
            await add('search_nodes', dict(query=node['name'][:512], group_ids=representatives, max_nodes=25), node['vector'], 'multi_group')
        if fact:
            await add('search_memory_facts', dict(query=' '.join(fact['fact'].split()[:8])[:512], group_ids=representatives, max_facts=25), fact['vector'], 'multi_group')
    assert readonly_queries > 0
    fixture = dict(scope='private provider-free original-memory search replay', cases=cases)
    path = Path('/tmp/original-search-fixture.json')
    path.write_text(json.dumps(fixture, ensure_ascii=False)); path.chmod(0o600)
    categories = collections.Counter(c['category'] for c in cases)
    counts = collections.Counter(c['tool'] for c in cases)
    result = dict(scope=fixture['scope'], policy_groups=len(groups), outside_policy_groups=len(all_groups)-len(groups),
        cases=len(cases), categories=dict(categories), tools=dict(counts),
        reference_refusals=sum('error' in c['expected'] for c in cases), sdk_readonly_queries=readonly_queries,
        provider_calls=0, node_wrapper_sha256=hashlib.sha256(source_path.read_bytes()).hexdigest(),
        fixture_sha256=hashlib.sha256(path.read_bytes()).hexdigest())
    for driver in drivers.values(): await driver.close()
    print(json.dumps(result), flush=True)
asyncio.run(main())
