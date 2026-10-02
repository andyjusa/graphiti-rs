#!/usr/bin/env python3
"""Private ten-type profile replay in a fresh owned Pi synthetic database only.

Run env -i/python -S with GRAPHITI_PI_SYNTHETIC=graphiti-rs-compat-pi.
Input profile, Rust snapshots and inherited helper must be mounted read-only in
/tmp. Output contains counts/hashes only; exceptions stay in private logs.
"""
import asyncio
import ast
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import runpy
import sys
from datetime import datetime

assert os.environ.get('GRAPHITI_PI_SYNTHETIC') == 'graphiti-rs-compat-pi'
spec = importlib.util.spec_from_file_location('baseline', '/tmp/python-pi-pipeline.py')
baseline = importlib.util.module_from_spec(spec)
spec.loader.exec_module(baseline)
from pydantic import BaseModel

GROUP = 'project_python_profile_20261002'


def tag(text, name):
    return text.split('<' + name + '>', 1)[1].split('</' + name + '>', 1)[0].strip()


def classifications(values):
    # Python's set-derived label order is deliberately unspecified.
    return [{**v, 'entity_type': sorted(v['entity_type'])} for v in values]


class ProfileLLM(baseline.MockLLM):
    def __init__(self, steps, schemas):
        super().__init__(steps, schemas)
        self.type_prompts = 0
        self.dedup_prompts = 0

    async def _generate_response(self, messages, response_model=None, max_tokens=None, model_size=None):
        expected = self.steps[self.cursor]['messages'][1]['content']
        actual = messages[1].content
        if response_model.__name__ == 'ExtractedEntities':
            assert ast.literal_eval(tag(actual, 'ENTITY TYPES')) == ast.literal_eval(tag(expected, 'ENTITY TYPES'))
            self.type_prompts += 1
        elif response_model.__name__ == 'NodeResolutions':
            assert classifications(json.loads(tag(actual, 'ENTITIES'))) == classifications(json.loads(tag(expected, 'ENTITIES')))
            self.dedup_prompts += 1
        return await super()._generate_response(messages, response_model, max_tokens, model_size)


async def main():
    assert baseline.importlib.metadata.version('graphiti-core') == '0.28.2'
    source = Path(sys.modules['graphiti_core.graphiti'].__file__)
    source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
    assert source_hash == '8487af6bda1b4efa914f4a827502d4f241a4e3c8ce7973df3c410e4b2ef8153f'
    patch = Path('/tmp/falkor_edge_query_compat.py')
    patch_hash = hashlib.sha256(patch.read_bytes()).hexdigest()
    assert patch_hash == '7abd25af31520e3951625c7a3739033bf9469b0d466c592243d9eaa8b3153939'
    runpy.run_path(str(patch))['install']()
    profile_bytes = Path('/tmp/entity-types.json').read_bytes()
    profile = json.loads(profile_bytes)
    assert len(profile) == 10
    # Exact classification-only factory used by the observed MCP service.
    types = {t['name']: type(t['name'], (BaseModel,), {'__doc__': t['description']}) for t in profile}
    assert all(not t.model_fields for t in types.values()), 'classification-only profile required'
    rust = json.loads(Path('/tmp/profile-rust.json').read_text())
    assert rust['actual_provider_calls'] == 0
    schemas = {json.loads(s['messages'][1]['content'].rsplit('input:\n', 1)[1])['title']:
               json.loads(s['messages'][1]['content'].rsplit('input:\n', 1)[1]) for s in rust['steps']}
    llm = ProfileLLM(rust['steps'], schemas)
    driver = baseline.FalkorDriver(host='127.0.0.1', port=6379, database=GROUP)
    graphiti = baseline.Graphiti(graph_driver=driver, llm_client=llm, embedder=baseline.MockEmbedder(),
                                 cross_encoder=baseline.NoRanking(), max_coroutines=2)
    try:
        rows, _, _ = await driver.execute_query('MATCH(n) RETURN count(n) AS count')
        assert rows[0]['count'] == 0, 'fresh owned synthetic graph required'
        await graphiti.build_indices_and_constraints()
        for _ in range(100):
            rows, _, _ = await driver.execute_query('CALL db.indexes() YIELD status RETURN status')
            if all(r['status'] == 'OPERATIONAL' for r in rows):
                break
            await asyncio.sleep(.05)
        else:
            raise AssertionError('indexes not operational')
        matched = 0
        identities = {}
        counts = []
        for expected in rust['snapshots']:
            ep = expected['episode']
            now = datetime.fromisoformat(ep['created_at'].replace('Z', '+00:00'))
            for name, module in list(sys.modules.items()):
                if name.startswith('graphiti_core') and hasattr(module, 'utc_now'):
                    module.utc_now = lambda: now
            result = await graphiti.add_episode(name=ep['name'], episode_body=ep['content'],
                source=baseline.EpisodeType(ep['source']), source_description=ep['source_description'],
                group_id=GROUP, uuid=ep['uuid'], reference_time=now, entity_types=types)
            current = await baseline.snapshot(driver, result.episode)
            assert baseline.canonical(current) == baseline.canonical(expected), f'canonical mismatch at episode {matched}'
            for node in current['nodes']:
                old = identities.setdefault(node['name'], node['uuid'])
                assert old == node['uuid'], 'reclassification replaced identity'
            matched += 1
            counts.append(len(current['nodes']))
        assert llm.cursor == len(rust['steps']) == 11
        assert llm.type_prompts == 3 and llm.dedup_prompts == 3
        print(json.dumps({'scope':'Actual deployed ten classification-only models; synthetic message/JSON/text episodes and mock providers',
            'package':'graphiti-core 0.28.2', 'profile_sha256':hashlib.sha256(profile_bytes).hexdigest(),
            'entity_type_count':len(types), 'episodes':matched, 'cumulative_entities':counts,
            'model_calls':llm.cursor, 'actual_provider_calls':0, 'canonical_graphs_match':True,
            'schemas_match_real_pydantic':True, 'classification_prompts_match':llm.type_prompts,
            'dedup_classifications_match':llm.dedup_prompts, 'existing_identity_preserved':True,
            'existing_labels_preserved_on_reclassification':True,
            'graphiti_source_sha256':source_hash, 'edge_patch_sha256':patch_hash,
            'limits':'Supplied model responses/basis embeddings. No relation extraction, real-provider quality, operational write, transport or cutover evidence.'}, indent=2))
    finally:
        await driver.close()


if __name__ == '__main__':
    asyncio.run(main())
