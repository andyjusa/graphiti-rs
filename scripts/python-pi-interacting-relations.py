#!/usr/bin/env python3
"""Actual pinned SDK replay of an owned synthetic interacting-relation cohort.

Rust and Python allocate fresh UUIDs independently. Match candidate fact sets
and rebase supplied decision indices to those same facts; do not replace the
SDK's resolution or persistence behavior. No operational data/providers.
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


def tag(text, name):
    return text.split('<' + name + '>', 1)[1].split('</' + name + '>', 1)[0].strip()


def canonical(snapshot):
    result = baseline.canonical(snapshot)
    by_fact = {fact['fact']: fact for fact in snapshot['facts']}
    for fact in result['facts']:
        fact['expired_at'] = baseline.instant(by_fact[fact['fact']]['expired_at'])
    return result


class RelationLLM(baseline.MockLLM):
    def __init__(self, steps, schemas):
        super().__init__(steps, schemas)
        self.edge_contexts = 0
        self.rebased_decisions = 0

    async def _generate_response(self, messages, response_model=None, max_tokens=None, model_size=None):
        index = self.cursor
        step = self.steps[index]
        expected = step['messages'][1]['content']
        actual = messages[1].content
        if response_model.__name__ != 'EdgeDuplicate':
            return await super()._generate_response(messages, response_model, max_tokens, model_size)
        assert tag(actual, 'NEW FACT') == tag(expected, 'NEW FACT')
        old_expected = ast.literal_eval(tag(expected, 'EXISTING FACTS'))
        inv_expected = ast.literal_eval(tag(expected, 'FACT INVALIDATION CANDIDATES'))
        old_actual = ast.literal_eval(tag(actual, 'EXISTING FACTS'))
        inv_actual = ast.literal_eval(tag(actual, 'FACT INVALIDATION CANDIDATES'))
        for before, after in [(old_expected, old_actual), (inv_expected, inv_actual)]:
            assert sorted(value['fact'] for value in before) == sorted(value['fact'] for value in after), 'candidate fact set differs'
        decision = await super()._generate_response(messages, response_model, max_tokens, model_size)
        def rebase(values, before, after):
            by_idx = {value['idx']: value['fact'] for value in before}
            by_fact = {value['fact']: value['idx'] for value in after}
            return [by_fact[by_idx[value]] if value in by_idx else value for value in values]
        result = {'duplicate_facts': rebase(decision['duplicate_facts'], old_expected, old_actual),
                  'contradicted_facts': rebase(decision['contradicted_facts'], old_expected + inv_expected, old_actual + inv_actual)}
        self.edge_contexts += 1
        self.rebased_decisions += result != decision
        return result


async def main():
    assert baseline.importlib.metadata.version('graphiti-core') == '0.28.2' and sys.version_info[:2] == (3, 11)
    source_hash = hashlib.sha256(Path(sys.modules['graphiti_core.graphiti'].__file__).read_bytes()).hexdigest()
    assert source_hash == '8487af6bda1b4efa914f4a827502d4f241a4e3c8ce7973df3c410e4b2ef8153f'
    patch = Path('/tmp/falkor_edge_query_compat.py')
    patch_hash = hashlib.sha256(patch.read_bytes()).hexdigest()
    assert patch_hash == '7abd25af31520e3951625c7a3739033bf9469b0d466c592243d9eaa8b3153939'
    runpy.run_path(str(patch))['install']()
    rust = json.loads(Path('/tmp/relation-rust.json').read_text())
    assert rust['actual_provider_calls'] == 0 and len(rust['snapshots']) == 4
    schemas = {json.loads(step['messages'][1]['content'].rsplit('input:\n', 1)[1])['title']:
               json.loads(step['messages'][1]['content'].rsplit('input:\n', 1)[1]) for step in rust['steps']}
    llm = RelationLLM(rust['steps'], schemas)
    group = 'project_python_interacting_20261003'
    driver = baseline.FalkorDriver(host='127.0.0.1', port=6379, database=group)
    graphiti = baseline.Graphiti(graph_driver=driver, llm_client=llm, embedder=baseline.MockEmbedder(), cross_encoder=baseline.NoRanking(), max_coroutines=2)
    try:
        rows, _, _ = await driver.execute_query('MATCH(n) RETURN count(n) AS count')
        assert rows[0]['count'] == 0, 'fresh owned synthetic graph required'
        await graphiti.build_indices_and_constraints()
        for _ in range(100):
            rows, _, _ = await driver.execute_query('CALL db.indexes() YIELD status RETURN status')
            if all(row['status'] == 'OPERATIONAL' for row in rows):
                break
            await asyncio.sleep(.05)
        else:
            raise AssertionError('indexes not operational')
        counts = []
        refs = []
        identities = {}
        vector_checks = 0
        for index, expected in enumerate(rust['snapshots']):
            ep = expected['episode']
            now = datetime.fromisoformat(ep['created_at'].replace('Z', '+00:00'))
            for name, module in list(sys.modules.items()):
                if name.startswith('graphiti_core') and hasattr(module, 'utc_now'):
                    module.utc_now = lambda: now
            result = await graphiti.add_episode(name=ep['name'], episode_body=ep['content'], source=baseline.EpisodeType(ep['source']), source_description=ep['source_description'], group_id=group, uuid=ep['uuid'], reference_time=now, entity_types={'Person':baseline.Person,'Organization':baseline.Organization})
            current = await baseline.snapshot(driver, result.episode)
            if canonical(current) != canonical(expected):
                print(json.dumps({'failed_episode':index,'episode_refs_actual':len(current['episode']['entity_edges']),'episode_refs_expected':len(expected['episode']['entity_edges']),'canonical_actual':canonical(current),'canonical_expected':canonical(expected)},indent=2))
                raise AssertionError('interacting relation canonical graph differs')
            actual_facts = {fact['uuid']:fact['fact'] for fact in current['facts']}
            expected_facts = {fact['uuid']:fact['fact'] for fact in expected['facts']}
            assert [actual_facts[value] for value in current['episode']['entity_edges']] == [expected_facts[value] for value in expected['episode']['entity_edges']], 'ordered episode references differ'
            for node in current['nodes']:
                assert identities.setdefault(node['name'],node['uuid']) == node['uuid']
            for query in ['MATCH(n:Entity) RETURN n.name_embedding AS embedding','MATCH()-[e:RELATES_TO]->() RETURN e.fact_embedding AS embedding']:
                rows, _, _ = await driver.execute_query(query)
                for row in rows:
                    assert row['embedding'] == [1.] + [0.] * 3071
                    vector_checks += 1
            counts.append(len(current['facts']))
            refs.append(len(current['episode']['entity_edges']))
        assert llm.cursor == len(rust['steps'])
        print(json.dumps({'scope':'Actual deployed SDK interacting reuse/invalidation and temporal history; supplied semantic decisions with fact-index rebasing','package':'graphiti-core 0.28.2','python':sys.version.split()[0],'episodes':len(counts),'entities':6,'cumulative_facts':counts,'episode_reference_counts':refs,'mock_llm_requests':llm.cursor,'edge_candidate_sets_match':llm.edge_contexts,'rebased_decisions':llm.rebased_decisions,'canonical_graphs_match':True,'schemas_match_real_pydantic':True,'expired_at_instants_match':True,'basis_vector_records_match':vector_checks,'actual_provider_calls':0,'graphiti_source_sha256':source_hash,'edge_patch_sha256':patch_hash,'limits':'Synthetic generic relations, mocked basis vectors and supplied decisions. Ranking ties may produce different candidate order in independently allocated fresh graphs; candidate sets must match. No real-provider quality, original-data writes or operational cutover.'},indent=2))
    finally:
        await driver.close()


if __name__ == '__main__':
    asyncio.run(main())
