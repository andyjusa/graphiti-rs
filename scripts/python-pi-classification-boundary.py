#!/usr/bin/env python3
"""Replay five classification boundary episodes against the actual pinned SDK.

Only a fresh owned synthetic DB, env -i/python -S, and the mounted supplied
Rust/mock report are permitted. Private profile definitions never leave the Pi.
"""
import ast
import asyncio
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
from graphiti_core.utils.maintenance import node_operations


def tag(text, name):
    return text.split('<' + name + '>', 1)[1].split('</' + name + '>', 1)[0].strip()


def classifications(values):
    return [{**value, 'entity_type': sorted(value['entity_type'])} for value in values]


class BoundaryLLM(baseline.MockLLM):
    def __init__(self, steps, schemas):
        super().__init__(steps, schemas)
        self.extraction_prompts = 0
        self.dedup_prompts = 0
        self.default_descriptions = 0
        self.whitespace_descriptions = 0
        self.blank_names_supplied = 0

    async def _generate_response(self, messages, response_model=None, max_tokens=None, model_size=None):
        expected = self.steps[self.cursor]['messages'][1]['content']
        actual = messages[1].content
        if response_model.__name__ == 'ExtractedEntities':
            assert ast.literal_eval(tag(actual, 'ENTITY TYPES')) == ast.literal_eval(tag(expected, 'ENTITY TYPES'))
            self.extraction_prompts += 1
            self.blank_names_supplied += sum(not value['name'].strip() for value in self.steps[self.cursor]['mock_response']['extracted_entities'])
        elif response_model.__name__ == 'NodeResolutions':
            values = classifications(json.loads(tag(actual, 'ENTITIES')))
            assert values == classifications(json.loads(tag(expected, 'ENTITIES'))), 'dedup descriptions or classifications differ'
            self.dedup_prompts += 1
            self.default_descriptions += sum(value['entity_type_description'] == 'Default Entity Type' for value in values)
            self.whitespace_descriptions += sum(value['entity_type_description'] == ' ' for value in values)
        return await super()._generate_response(messages, response_model, max_tokens, model_size)


async def main():
    assert baseline.importlib.metadata.version('graphiti-core') == '0.28.2'
    # None.__doc__ differs on newer Python interpreters; the deployed 3.11 value is authoritative.
    assert sys.version_info[:2] == (3, 11) and None.__doc__ is None
    source = Path(sys.modules['graphiti_core.graphiti'].__file__)
    source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
    assert source_hash == '8487af6bda1b4efa914f4a827502d4f241a4e3c8ce7973df3c410e4b2ef8153f'
    node_hash = hashlib.sha256(Path(node_operations.__file__).read_bytes()).hexdigest()
    assert node_hash == '49d8caad8e89aac5f9530dded21d816e840ccb53284a2b2d5e42073f90f56c34'
    patch = Path('/tmp/falkor_edge_query_compat.py')
    patch_hash = hashlib.sha256(patch.read_bytes()).hexdigest()
    assert patch_hash == '7abd25af31520e3951625c7a3739033bf9469b0d466c592243d9eaa8b3153939'
    runpy.run_path(str(patch))['install']()
    rust = json.loads(Path('/tmp/boundary-rust.json').read_text())
    assert rust['actual_provider_calls'] == 0 and len(rust['profiles']) == 3
    schemas = {json.loads(step['messages'][1]['content'].rsplit('input:\n', 1)[1])['title']:
               json.loads(step['messages'][1]['content'].rsplit('input:\n', 1)[1]) for step in rust['steps']}
    llm = BoundaryLLM(rust['steps'], schemas)
    counts = []
    identity_checks = 0
    for index, profile in enumerate(rust['profiles']):
        types = {value['name']: type(value['name'], (BaseModel,), {'__doc__': value['description']}) for value in profile['entity_types']}
        assert all(not value.model_fields for value in types.values())
        if profile['profile'] == 'blank-descriptions':
            assert types['EmptyKind'].__doc__ == '' and types['WhitespaceKind'].__doc__ == ' '
        driver = baseline.FalkorDriver(host='127.0.0.1', port=6379, database=f'project_python_boundary_{index}_20261002')
        graphiti = baseline.Graphiti(graph_driver=driver, llm_client=llm, embedder=baseline.MockEmbedder(), cross_encoder=baseline.NoRanking(), max_coroutines=2)
        try:
            rows, _, _ = await driver.execute_query('MATCH(n) RETURN count(n) AS count')
            assert rows[0]['count'] == 0, 'fresh owned graph required'
            await graphiti.build_indices_and_constraints()
            for _ in range(100):
                rows, _, _ = await driver.execute_query('CALL db.indexes() YIELD status RETURN status')
                if all(row['status'] == 'OPERATIONAL' for row in rows):
                    break
                await asyncio.sleep(.05)
            else:
                raise AssertionError('indexes not operational')
            identities = {}
            labels = {}
            for expected in profile['snapshots']:
                ep = expected['episode']
                now = datetime.fromisoformat(ep['created_at'].replace('Z', '+00:00'))
                for name, module in list(sys.modules.items()):
                    if name.startswith('graphiti_core') and hasattr(module, 'utc_now'):
                        module.utc_now = lambda: now
                result = await graphiti.add_episode(name=ep['name'], episode_body=ep['content'], source=baseline.EpisodeType(ep['source']), source_description=ep['source_description'], group_id=driver._database, uuid=ep['uuid'], reference_time=now, entity_types=types or None)
                current = await baseline.snapshot(driver, result.episode)
                assert baseline.canonical(current) == baseline.canonical(expected), 'canonical boundary graph differs'
                for node in current['nodes']:
                    if node['name'] in identities:
                        assert identities[node['name']] == node['uuid'] and labels[node['name']] == sorted(node['labels'])
                        identity_checks += 1
                    else:
                        identities[node['name']] = node['uuid']
                        labels[node['name']] = sorted(node['labels'])
                counts.append(len(current['nodes']))
        finally:
            await driver.close()
    assert counts == [6, 6, 12, 4, 4]
    assert llm.cursor == len(rust['steps'])
    assert llm.extraction_prompts == 5 and llm.blank_names_supplied == 10
    assert llm.default_descriptions > 0 and llm.whitespace_descriptions == 1
    print(json.dumps({'scope':'Actual deployed Python classification fallbacks and optional classification descriptions; synthetic message/JSON/text only', 'package':'graphiti-core 0.28.2','python':sys.version.split()[0],'none_doc_is_null':True,'profiles':3,'episodes':5,'cumulative_entities':counts,'mock_llm_requests':llm.cursor,'actual_provider_calls':0,'extraction_prompts_match':llm.extraction_prompts,'dedup_prompts_match':llm.dedup_prompts,'default_descriptions_match':llm.default_descriptions,'whitespace_descriptions_match':llm.whitespace_descriptions,'blank_names_supplied_and_filtered':llm.blank_names_supplied,'existing_identity_and_labels_checks':identity_checks,'canonical_graphs_match':True,'schemas_match_real_pydantic':True,'graphiti_source_sha256':source_hash,'node_operations_sha256':node_hash,'edge_patch_sha256':patch_hash,'limits':'Supplied responses and basis vectors; no relation/provider quality, original-data writes, transport or operational cutover evidence'},indent=2))


if __name__ == '__main__':
    asyncio.run(main())
