#!/usr/bin/env python3
"""Seed the same synthetic legacy fact attributes, then use the fixed SDK evaluator."""
import asyncio,importlib.util,json
from pathlib import Path
spec=importlib.util.spec_from_file_location('relation_evaluator','/tmp/python-pi-interacting-relations.py')
evaluator=importlib.util.module_from_spec(spec);spec.loader.exec_module(evaluator)
assert json.loads(Path('/tmp/relation-rust.json').read_text())['metadata_seeded'] is True
snapshot=evaluator.baseline.snapshot
async def metadata_snapshot(driver,episode):
    if episode.name=='interacting-0':
        for fact in ['Gaon works at Saebit.','Nari works at Saebit.']:
            rows,_,_=await driver.execute_query("MATCH()-[r:RELATES_TO]->() WHERE r.fact=$fact SET r.fixture_note='Synthetic legacy metadata',r.fixture_weight=7 RETURN r.uuid AS uuid",fact=fact)
            assert len(rows)==1
    result=await snapshot(driver,episode)
    facts={f['fact']:f for f in result['facts']}
    if episode.name!='interacting-0':
        assert facts['Gaon works at Saebit.']['attributes']=={}, 'SDK model branch must clear generic attributes'
        assert facts['Nari works at Saebit.']['attributes']=={'fixture_note':'Synthetic legacy metadata','fixture_weight':7}, 'exact reuse and later invalidation must retain metadata'
    return result
evaluator.baseline.snapshot=metadata_snapshot
asyncio.run(evaluator.main())
