#!/usr/bin/env python3
"""Additional private read-only search edge cases; no provider calls or stdout data."""
import ast
import asyncio
import collections
import hashlib
import json
import os
from pathlib import Path

assert os.environ.get('GRAPHITI_PRIVATE_READ_REHEARSAL') == 'original-restore-read-only'
reference_path = Path('/tmp/original-search-probe.py')
tree = ast.parse(reference_path.read_text())
# Load the guarded reference setup without rerunning its completed 105 cases.
assert isinstance(tree.body[-1], ast.Expr)
assert ast.unparse(tree.body[-1]) == 'asyncio.run(main())'
tree.body.pop()
ns = {'__name__': 'private_reference_setup'}
exec(compile(tree, str(reference_path), 'exec'), ns)
original_fixture = json.loads(Path('/tmp/original-search-fixture.json').read_text())
vector = original_fixture['cases'][0]['vector']
ns['client'].vector = vector

async def main():
    cases = []
    async def add(tool, args, category):
        wrapper = ns['namespace']['_search_nodes_group_aware' if tool == 'search_nodes' else '_search_memory_facts_group_aware']
        expected = await wrapper(**args)
        cases.append(dict(tool=tool, args=args, vector=vector, expected=expected, category=category))
    for group in ns['groups']:
        driver = ns['drivers'][group]
        for tool, label, query in [
            ('search_nodes','Entity','MATCH(n:Entity) WHERE n.group_id=$group RETURN count(n) AS count'),
            ('search_memory_facts','RELATES_TO','MATCH()-[e:RELATES_TO]->() WHERE e.group_id=$group RETURN count(e) AS count'),
        ]:
            rows, _, _ = await driver.execute_query(query, group=group)
            if rows[0]['count'] == 0:
                await add(tool, dict(query='SyntheticAbsentToken', group_ids=[group]), 'empty_entities' if label=='Entity' else 'empty_facts')
    for tool in ['search_nodes','search_memory_facts']:
        for query in ['', ' \t\n', 'the a and', '?!', 'SyntheticAbsentToken']:
            await add(tool, dict(query=query, group_ids=['user_primary']), 'blank_query' if not query.strip() else 'stopword_query' if query in ['the a and','?!'] else 'semantic_without_keyword')
        for groups in [[], ['user_primary','user_primary'], ['unapproved_group'], ['user_primary','unapproved_group'], [''], ['user_primary','']]:
            await add(tool, dict(query='SyntheticAbsentToken', group_ids=groups), 'group_guard')
        limit_name = 'max_nodes' if tool=='search_nodes' else 'max_facts'
        for limit in [0, -1]:
            await add(tool, dict(query='SyntheticAbsentToken', group_ids=['user_primary'], **{limit_name:limit}), 'invalid_limit')
    for labels in [[], [''], ['1Person'], ['two words'], ['Person|Company'], ['Person:Company'], ['가온'], ['Person`'], ['Person'], ['_AbsentType']]:
        await add('search_nodes', dict(query='SyntheticAbsentToken', group_ids=['user_primary'], entity_types=labels), 'type_guard')
    # Empty group lists are handled before the SDK filter constructor.
    for query in ['SyntheticAbsentToken','']:
        await add('search_nodes', dict(query=query, group_ids=[], entity_types=['two words']), 'empty_group_filter')
        await add('search_nodes', dict(query=query, group_ids=['user_primary'], entity_types=[]), 'empty_type_filter')
    await add('search_memory_facts', dict(query='SyntheticAbsentToken', group_ids=['user_primary','general_chat'], center_node_uuid='00000000-0000-4000-8000-000000000001'), 'center_group_guard')
    path = Path('/tmp/original-search-negative-fixture.json')
    path.write_text(json.dumps(dict(scope='private provider-free original-memory search replay', cases=cases), ensure_ascii=False)); path.chmod(0o600)
    result = dict(scope='private original-memory search negative/empty cases', cases=len(cases),
        categories=dict(collections.Counter(c['category'] for c in cases)),
        tools=dict(collections.Counter(c['tool'] for c in cases)),
        reference_refusals=sum('error' in c['expected'] for c in cases),
        sdk_readonly_queries=ns['readonly_queries'], provider_calls=0,
        reference_setup_sha256=hashlib.sha256(reference_path.read_bytes()).hexdigest(),
        node_wrapper_sha256=hashlib.sha256(ns['source_path'].read_bytes()).hexdigest(),
        fixture_sha256=hashlib.sha256(path.read_bytes()).hexdigest())
    for driver in ns['drivers'].values(): await driver.close()
    print(json.dumps(result), flush=True)
asyncio.run(main())
