#!/usr/bin/env python3
"""Run observed source bodies with synthetic DTOs/driver/LLM decisions, stdlib only.
No package imports, operational startup hooks, credentials, model or database I/O.
This is method-level differential evidence, not end-to-end Python Graphiti parity.
"""
import ast
import asyncio
import copy
import hashlib
import json
import logging
import re
import sys
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path
from types import SimpleNamespace as NS

ROOT = Path(__file__).resolve().parent.parent / 'tests/fixtures/python-0282'
MANIFEST = json.loads((ROOT / 'manifest.json').read_text())

class StripAnnotations(ast.NodeTransformer):
    def visit_FunctionDef(self, node):
        node.returns = None
        node.decorator_list = []
        for arg in node.args.posonlyargs + node.args.args + node.args.kwonlyargs:
            arg.annotation = None
        return self.generic_visit(node)
    visit_AsyncFunctionDef = visit_FunctionDef
    def visit_AnnAssign(self, node):
        return ast.copy_location(ast.Assign(targets=[node.target], value=node.value or ast.Constant(None)), node)


def load(source, name, env):
    item = next(i for i in MANIFEST['functions'] if i['source'] == source and i['name'] == name)
    data = (ROOT / item['file']).read_bytes()
    if hashlib.sha256(data).hexdigest() != item['excerpt_sha256']:
        raise RuntimeError('baseline excerpt hash mismatch')
    tree = StripAnnotations().visit(ast.parse(data))
    exec(compile(ast.fix_missing_locations(tree), item['file'], 'exec'), env)
    return env.get(name)


def dt(s):
    return datetime.fromisoformat(s.replace('Z', '+00:00')) if s else None


def edge(value):
    obj = NS(**copy.deepcopy(value))
    for key in ('valid_at', 'invalid_at', 'expired_at', 'created_at'):
        setattr(obj, key, dt(getattr(obj, key, None)))
    return obj


def public_edge(obj):
    out = vars(obj).copy()
    for key, value in out.items():
        if isinstance(value, datetime):
            out[key] = value.isoformat()
    return out


async def run(case):
    env = {'re': re, 'defaultdict': defaultdict, 'logger': logging.getLogger('baseline'), 'time': lambda: 0,
           'ensure_utc': lambda value: value.astimezone(timezone.utc) if value else None,
           'utc_now': lambda: dt(case.get('now', '2026-10-01T10:00:00.123456Z'))}
    load('core/utils/maintenance/dedup_helpers.py', '_normalize_string_exact', env)
    if case['kind'] == 'fulltext':
        item=MANIFEST['stopwords']; data=(ROOT/item['file']).read_bytes()
        if hashlib.sha256(data).hexdigest()!=item['excerpt_sha256']: raise RuntimeError('stopword hash mismatch')
        env.update({'STOPWORDS':json.loads(data),'validate_group_ids':lambda groups: None})
        sanitize=load('core/driver/falkordb_driver.py','sanitize',env)
        build=load('core/driver/falkordb_driver.py','build_fulltext_query',env)
        return build(NS(sanitize=lambda text:sanitize(None,text)),case['query'],[case['group']])
    if case['kind'] == 'normalize':
        return [env['_normalize_string_exact'](x) for x in case['texts']]
    if case['kind'] == 'rrf':
        f = load('core/search/search_utils.py', 'rrf', env)
        return f(case['lists'])[0]
    if case['kind'] == 'resolve':
        load('core/utils/maintenance/edge_operations.py', 'resolve_edge_contradictions', env)
        f = load('core/utils/maintenance/edge_operations.py', 'resolve_extracted_edge', env)
        class LLM:
            async def generate_response(self, *args, **kwargs):
                return case['decision']
        env.update({'EdgeDuplicate': lambda **kw: NS(**kw), 'ModelSize': NS(small='small'),
                    'prompt_library': NS(dedupe_edges=NS(resolve_edge=lambda x: x))})
        result = await f(LLM(), edge(case['extracted']), [edge(x) for x in case['related']],
                         [edge(x) for x in case['existing']], NS(uuid=case['episode'], valid_at=dt(case['now'])))
        return [public_edge(result[0]), [public_edge(x) for x in result[1]], [public_edge(x) for x in result[2]]]
    if case['kind'] == 'edge_read':
        return ' '.join(load('patches/falkor_edge_query_compat.py','safe_edge_read',env)(case['query']).split())
    if case['kind'] == 'center' and case.get('algorithm') in ('fallback', 'deployed-falkor'):
        f=load('core/search/search_utils.py','node_distance_reranker',env)
        class Driver:
            search_interface=None
            provider='falkor'
            async def execute_query(self, query, **params):
                if case.get('algorithm') == 'deployed-falkor':
                    return [{'score':1,'uuid':u} for u in case['neighbors']], ['score','uuid'], None
                return [[1,u] for u in case['neighbors']], ['score','uuid'], None
        env['GraphProvider']=NS(FALKORDB='falkor',KUZU='kuzu')
        return (await f(Driver(),case['ids'],case['center']))[0]
    if case['kind'] == 'center':
        f = load('core/driver/falkordb/operations/search_ops.py', 'node_distance_reranker', env)
        class Executor:
            async def execute_query(self, query, **params):
                if 'RETURN 1 AS score' in query:
                    return ([{'score':1, 'uuid':u} for u in case['neighbors']], None, None)
                return ([{'uuid':u} for u in params['uuids']], None, None)
        env.update({'entity_node_from_record': lambda r: NS(uuid=r['uuid']),
                    'get_entity_node_return_query': lambda p: 'n.uuid AS uuid', 'GraphProvider': NS(FALKORDB='falkor')})
        return [n.uuid for n in await f(None, Executor(), case['ids'], case['center'])]
    if case['kind'] == 'groups':
        env['GroupScopeViolation'] = RuntimeError
        f = load('patches/group_read_routing.py', 'normalize_group_ids', env)
        return f(case['groups'], 'user_primary', lambda g: None)
    if case['kind'] in ('delete_episode', 'delete_edge', 'clear'):
        # Observe exact emitted operations; graph effects are verified separately on FalkorDB.
        queries = []
        class Driver:
            provider = 'falkor'
            graph_operations_interface = None
            async def execute_query(self, query, **params):
                queries.append({'query':' '.join(query.split()), 'params':params})
                return [], None, None
        env['GraphProvider'] = NS(NEO4J='neo4j', KUZU='kuzu')
        if case['kind'] == 'clear':
            f = load('core/driver/falkordb/operations/graph_ops.py', 'clear_data', env)
            await f(None, Driver(), ['user_primary'])
        else:
            source = 'core/nodes.py' if case['kind'] == 'delete_episode' else 'core/edges.py'
            f = load(source, 'delete', env)
            await f(NS(uuid=case['uuid']), Driver())
        return queries
    if case['kind'] == 'recipes':
        item=next(i for i in MANIFEST['functions'] if i['name']=='search_recipes')
        data=(ROOT/item['file']).read_bytes()
        assert hashlib.sha256(data).hexdigest()==item['excerpt_sha256']
        result={}
        for node in ast.parse(data).body:
            config=node.value.keywords[0].value
            result[node.targets[0].id]={k.arg: [e.attr for e in k.value.elts] if isinstance(k.value,ast.List) else k.value.attr for k in config.keywords}
        load('core/search/search_utils.py','DEFAULT_MIN_SCORE',env)
        return {'recipes':result,'semantic_min_score':env['DEFAULT_MIN_SCORE']}
    if case['kind'] == 'decode':
        for name in ('decode_first_json','flatten_schema_value_envelope','normalize_structured_response'):
            load('patches/openrouter_json.py',name,env)
        item=next(i for i in MANIFEST['functions'] if i['name']=='adapter_decode_block')
        data=(ROOT/item['file']).read_bytes()
        assert hashlib.sha256(data).hexdigest()==item['excerpt_sha256']
        body=ast.parse(data).body
        wrapper=ast.FunctionDef(name='decode',args=ast.arguments(posonlyargs=[],args=[],kwonlyargs=[],kw_defaults=[],defaults=[]),body=body,decorator_list=[])
        env.update({'content':case['content'],'schema':{'properties':{'entities':{},'facts':{}}}})
        exec(compile(ast.fix_missing_locations(ast.Module(body=[wrapper],type_ignores=[])),item['file'],'exec'),env)
        try: return env['decode']()
        except (ValueError,IndexError): return {'error':True}
    if case['kind'] == 'stable_uuid':
        # Execute only the actual if/try/create block in an async wrapper.
        item = next(i for i in MANIFEST['functions'] if i['name']=='stable_episode_block')
        data = (ROOT/item['file']).read_bytes()
        assert hashlib.sha256(data).hexdigest()==item['excerpt_sha256']
        class Missing(Exception): pass
        class Episode:
            def __new__(cls, **kw): return NS(**kw)
            @staticmethod
            async def get_by_uuid(driver, uuid):
                if case['exists']: return NS(content='old content', name='old name', uuid=uuid)
                raise Missing()
        body = ast.parse(data).body
        wrapper = ast.AsyncFunctionDef(name='choose', args=ast.arguments(posonlyargs=[], args=[],kwonlyargs=[],kw_defaults=[],defaults=[]), body=body+[ast.Return(ast.Name('episode',ast.Load()))], decorator_list=[])
        env.update({'EpisodicNode':Episode,'NodeNotFoundError':Missing,'self':NS(driver=None),'uuid':case['uuid'],
                    'name':'new name','group_id':'user_primary','source':'text','episode_body':'new content',
                    'source_description':'synthetic','now':dt(case['now']),'reference_time':dt(case['now'])})
        exec(compile(ast.fix_missing_locations(ast.Module(body=[wrapper],type_ignores=[])),item['file'],'exec'),env)
        e=await env['choose']()
        return {'uuid':e.uuid,'name':e.name,'content':e.content}
    raise ValueError('unknown case')


if __name__ == '__main__':
    cases=json.load(sys.stdin)
    print(json.dumps([asyncio.run(run(c)) for c in cases], ensure_ascii=False, allow_nan=False))
