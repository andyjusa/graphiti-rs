#!/usr/bin/env python3
"""Execute pinned original prompt functions and dedup helpers; stdlib only.

This independent oracle does not invoke the Rust prompt compiler, SDK startup,
providers or databases. All input is synthetic JSON on stdin.
"""
import ast
import hashlib
import json
import runpy
import sys
from pathlib import Path
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parent.parent / 'tests/fixtures/python-0282'
PROMPT_HASHES = json.loads((ROOT / 'prompt-source-hashes.json').read_text())


def checked(path, expected):
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != expected:
        raise RuntimeError('observed source hash mismatch: ' + path.name)
    return data


def prompt(stage, context):
    module, name = stage.split('.')
    path = ROOT / 'prompts' / (module + '.py')
    tree = ast.parse(checked(path, PROMPT_HASHES[path.name]))
    function = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == name)
    snippets = ast.parse(checked(ROOT / 'prompts/snippets.py', PROMPT_HASHES['snippets.py']))
    instructions = next(ast.literal_eval(n.value) for n in snippets.body
                        if isinstance(n, ast.Assign) and n.targets[0].id == 'summary_instructions')
    env = {'Message': lambda **values: values,
           'to_prompt_json': lambda value: json.dumps(value, ensure_ascii=False),
           'MAX_SUMMARY_CHARS': 500, 'summary_instructions': instructions}
    # Postpone annotations without changing the selected function body.
    future = ast.ImportFrom(module='__future__', names=[ast.alias(name='annotations')], level=0)
    selected = ast.fix_missing_locations(ast.Module(body=[future, function], type_ignores=[]))
    exec(compile(selected, str(path), 'exec'), env)
    return env[name](context)


def main():
    hashes = json.loads((ROOT / 'dedup-source-hash.json').read_text())
    path = ROOT / 'dedup_helpers.py'
    checked(path, hashes[path.name])
    dedup = runpy.run_path(str(path))
    helpers = {}
    for name, item in json.loads((ROOT / 'pipeline-helper-hashes.json').read_text()).items():
        env = {'re': __import__('re')}
        exec(checked(ROOT / item['file'], item['excerpt_sha256']), env)
        helpers[name] = env[name]
    results = []
    for case in json.load(sys.stdin):
        if case['kind'] == 'prompt':
            results.append(prompt(case['stage'], case['context']))
        elif case['kind'] == 'name':
            fuzzy = dedup['_normalize_name_for_fuzzy'](case['name'])
            shingles = dedup['_shingles'](fuzzy)
            results.append({'fuzzy': fuzzy, 'high_entropy': dedup['_has_high_entropy'](fuzzy),
                            'shingles': sorted(shingles), 'signature': dedup['_minhash_signature'](shingles)})
        elif case['kind'] == 'resolve':
            candidates = [SimpleNamespace(name=name, uuid=str(i)) for i, name in enumerate(case['candidates'])]
            node = SimpleNamespace(name=case['name'], uuid='new')
            indexes = dedup['_build_candidate_indexes'](candidates)
            state = dedup['DedupResolutionState'](resolved_nodes=[None], uuid_map={}, unresolved_indices=[])
            dedup['_resolve_with_similarity']([node], indexes, state)
            match = state.resolved_nodes[0]
            results.append(int(match.uuid) if match else None)
        elif case['kind'] == 'clean':
            results.append(helpers['_clean_input'](None, case['text']))
        elif case['kind'] == 'summary':
            results.append(helpers['truncate_at_sentence'](case['text'], 500))
        else:
            raise ValueError('unknown baseline operation')
    json.dump(results, sys.stdout, ensure_ascii=False)


if __name__ == '__main__':
    main()
