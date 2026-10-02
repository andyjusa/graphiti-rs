#!/usr/bin/env python3
"""Compare actual observed Python add_episode with synthetic Rust snapshots.

Run only under env -i / python -S in the labelled isolated Pi database image.
Supplies model responses and embeddings; never loads operational entrypoints,
credentials, original memories, paid providers or sitecustomize.
"""
import asyncio
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import runpy
import sys
from datetime import datetime, timezone

assert os.environ.get('GRAPHITI_PI_SYNTHETIC') == 'graphiti-rs-compat-pi'
sys.path.insert(0, '/app/mcp/.venv/lib/python3.11/site-packages')

from graphiti_core import Graphiti
from graphiti_core.cross_encoder.client import CrossEncoderClient
from graphiti_core.driver.falkordb_driver import FalkorDriver
from graphiti_core.embedder.client import EmbedderClient
from graphiti_core.llm_client.client import LLMClient
from graphiti_core.llm_client.config import LLMConfig
from graphiti_core.nodes import EpisodeType
from pydantic import BaseModel

GROUP = 'project_python_pipeline_20261002'


class MockLLM(LLMClient):
    def __init__(self, steps, schemas):
        super().__init__(LLMConfig(model='synthetic'))
        self.steps = steps
        self.schemas = schemas
        self.cursor = 0
        self.observed = []

    async def _generate_response(self, messages, response_model=None, max_tokens=None, model_size=None):
        step = self.steps[self.cursor]
        schema = json.loads(step['messages'][1]['content'].rsplit('input:\n', 1)[1])
        assert response_model.__name__ == schema['title'], (self.cursor, response_model.__name__, schema['title'])
        # Compare generated schemas with real Pydantic, not the AST compiler.
        assert response_model.model_json_schema() == self.schemas[schema['title']], ('schema', schema['title'])
        self.observed.append({'stage': response_model.__name__, 'messages': [m.model_dump() for m in messages]})
        self.cursor += 1
        return step['mock_response']


class MockEmbedder(EmbedderClient):
    async def create(self, input_data):
        return [1.] + [0.] * 3071

    async def create_batch(self, input_data_list):
        return [[1.] + [0.] * 3071 for _ in input_data_list]


class NoRanking(CrossEncoderClient):
    async def rank(self, query, passages):
        raise AssertionError('unexpected cross encoder call in RRF ingestion')


class Person(BaseModel):
    """A person"""


class Organization(BaseModel):
    """A company"""


def instant(value):
    if not value:
        return None
    return datetime.fromisoformat(value.replace('Z', '+00:00')).astimezone(timezone.utc).isoformat()


def canonical(snapshot):
    names = {n['uuid']: n['name'] for n in snapshot['nodes']}
    facts = {f['uuid']: f['fact'] for f in snapshot['facts']}
    ep = snapshot['episode']
    return {
        'episode': {**{k: ep[k] for k in ['uuid', 'name', 'content', 'source', 'source_description']},
                    'created_at': instant(ep['created_at']), 'valid_at': instant(ep['valid_at']),
                    'entity_edges': sorted(facts[id] for id in ep['entity_edges'])},
        'nodes': sorted([{'name': n['name'], 'summary': n['summary'], 'labels': sorted(n['labels']),
                          'attributes': n['attributes'], 'created_at': instant(n['created_at'])}
                         for n in snapshot['nodes']], key=lambda n: n['name']),
        'facts': sorted([{'source': names[f['source_node_uuid']], 'target': names[f['target_node_uuid']],
                          'name': f['name'], 'fact': f['fact'], 'episodes': sorted(f['episodes']),
                          'created_at': instant(f['created_at']), 'valid_at': instant(f['valid_at']),
                          'invalid_at': instant(f['invalid_at']), 'expired': f['expired_at'] is not None,
                          'attributes': f['attributes']}
                         for f in snapshot['facts']], key=lambda f: (f['fact'], f['source'], f['target'])),
    }


async def snapshot(driver, episode):
    from graphiti_core.nodes import EntityNode
    from graphiti_core.edges import EntityEdge
    rows, _, _ = await driver.execute_query('MATCH(n:Entity) RETURN n.uuid AS uuid')
    nodes = [await EntityNode.get_by_uuid(driver, row['uuid']) for row in rows]
    rows, _, _ = await driver.execute_query('MATCH()-[e:RELATES_TO]->() RETURN e.uuid AS uuid')
    edges = [await EntityEdge.get_by_uuid(driver, row['uuid']) for row in rows]
    return {'episode': json.loads(episode.model_dump_json()),
            'nodes': [json.loads(n.model_dump_json()) for n in nodes],
            'facts': [json.loads(e.model_dump_json()) for e in edges]}


async def main():
    assert importlib.metadata.version('graphiti-core') == '0.28.2'
    module = sys.modules['graphiti_core.graphiti']
    assert hashlib.sha256(Path(module.__file__).read_bytes()).hexdigest() == '8487af6bda1b4efa914f4a827502d4f241a4e3c8ce7973df3c410e4b2ef8153f'
    rust = json.loads(Path('/tmp/staged-rust.json').read_text())
    assert rust['actual_provider_calls'] == 0
    schemas = {}
    for step in rust['steps']:
        schema = json.loads(step['messages'][1]['content'].rsplit('input:\n', 1)[1])
        schemas[schema['title']] = schema
    patch = Path('/tmp/falkor_edge_query_compat.py')
    assert hashlib.sha256(patch.read_bytes()).hexdigest() == '7abd25af31520e3951625c7a3739033bf9469b0d466c592243d9eaa8b3153939'
    runpy.run_path(str(patch))['install']()
    llm = MockLLM(rust['steps'], schemas)
    driver = FalkorDriver(host='127.0.0.1', port=6379, database=GROUP)
    graphiti = Graphiti(graph_driver=driver, llm_client=llm, embedder=MockEmbedder(),
                        cross_encoder=NoRanking(), max_coroutines=2)
    try:
        count, _, _ = await driver.execute_query('MATCH(n) RETURN count(n) AS count')
        assert count[0]['count'] == 0, 'requires fresh owned synthetic graph'
        await graphiti.build_indices_and_constraints()
        for _ in range(100):
            rows, _, _ = await driver.execute_query('CALL db.indexes() YIELD status RETURN status')
            if all(row['status'] == 'OPERATIONAL' for row in rows):
                break
            await asyncio.sleep(.05)
        else:
            raise AssertionError('indexes did not become operational')
        actual = []
        old_ids = []
        for expected in rust['snapshots']:
            ep = expected['episode']
            now = datetime.fromisoformat(ep['created_at'].replace('Z', '+00:00'))
            # Freeze creation time in the real loaded SDK modules for comparison.
            for name, module in list(sys.modules.items()):
                if name.startswith('graphiti_core') and hasattr(module, 'utc_now'):
                    module.utc_now = lambda: now
            result = await graphiti.add_episode(name=ep['name'], episode_body=ep['content'],
                source=EpisodeType(ep['source']), source_description=ep['source_description'],
                group_id=GROUP, uuid=ep['uuid'], reference_time=now,
                entity_types={'Person': Person, 'Organization': Organization})
            current = await snapshot(driver, result.episode)
            assert canonical(current) == canonical(expected), json.dumps({'actual':canonical(current),'expected':canonical(expected)},ensure_ascii=False)
            old = next(f for f in current['facts'] if f['fact'] == 'Gaon works at Saebit.')
            old_ids.append(old['uuid'])
            actual.append(canonical(current))
        assert len(set(old_ids)) == 1, 'duplicate changed existing identity'
        assert llm.cursor == len(rust['steps'])
        module = sys.modules['graphiti_core.graphiti']
        report = {'scope':'actual observed patched Python Graphiti.add_episode vs Rust; seven synthetic message/JSON/text episodes and supplied model responses',
                  'package':'graphiti-core 0.28.2','actual_provider_calls':0,
                  'graphiti_source_sha256':hashlib.sha256(Path(module.__file__).read_bytes()).hexdigest(),
                  'edge_patch_sha256':hashlib.sha256(patch.read_bytes()).hexdigest(),
                  'episodes':len(actual),'model_calls':llm.cursor,'canonical_graphs_match':True,
                  'schemas_match_real_pydantic':True,'snapshots':actual,
                  'limits':'Generic deployed MCP relations/classification-only entity types; fake model/embedding values. Transport, gateway, actual providers, multilingual quality and original-memory restoration are separate gates.'}
        print(json.dumps(report,ensure_ascii=False,indent=2))
    finally:
        await driver.close()


if __name__ == '__main__':
    asyncio.run(main())
