#!/usr/bin/env python3
"""Compile selected observed Graphiti prompt ASTs into inert Rust-readable data.
No Python package or startup module is imported. Source hashes are pinned.
"""
import argparse
import ast
import hashlib
import json
from pathlib import Path

FUNCTIONS = {
    'extract_nodes': ['extract_message', 'extract_json', 'extract_text', 'extract_summaries_batch'],
    'extract_edges': ['edge'],
    'dedupe_nodes': ['nodes'],
    'dedupe_edges': ['resolve_edge'],
}
MODELS = {
    'extract_message': 'ExtractedEntities', 'extract_json': 'ExtractedEntities',
    'extract_text': 'ExtractedEntities', 'extract_summaries_batch': 'SummarizedEntities',
    'edge': 'ExtractedEdges', 'nodes': 'NodeResolutions', 'resolve_edge': 'EdgeDuplicate',
}

def compile_all(root):
    sources = {}
    snippets = ast.parse((root / 'snippets.py').read_text())
    constants = {'MAX_SUMMARY_CHARS': 500}
    for node in snippets.body:
        if isinstance(node, ast.Assign) and isinstance(node.value, ast.Constant):
            constants[node.targets[0].id] = node.value.value
    classes = {}
    trees = {}
    for module in FUNCTIONS:
        path = root / (module + '.py')
        data = path.read_bytes()
        sources[path.name] = hashlib.sha256(data).hexdigest()
        trees[module] = ast.parse(data)
        classes.update({n.name: n for n in trees[module].body if isinstance(n, ast.ClassDef)})
    sources['snippets.py'] = hashlib.sha256((root / 'snippets.py').read_bytes()).hexdigest()

    def annotation(node):
        if isinstance(node, ast.Name):
            if node.id in classes:
                return {'$ref': '#/$defs/' + node.id}
            return {'type': {'str': 'string', 'int': 'integer', 'bool': 'boolean'}[node.id]}
        if isinstance(node, ast.Constant) and node.value is None:
            return {'type': 'null'}
        if isinstance(node, ast.BinOp) and isinstance(node.op, ast.BitOr):
            return {'anyOf': [annotation(node.left), annotation(node.right)]}
        if isinstance(node, ast.Subscript) and isinstance(node.value, ast.Name) and node.value.id == 'list':
            return {'type': 'array', 'items': annotation(node.slice)}
        raise ValueError(ast.dump(node))

    def schema(name):
        properties, required = {}, []
        for n in classes[name].body:
            if not isinstance(n, ast.AnnAssign):
                continue
            prop = annotation(n.annotation)
            prop['title'] = n.target.id.replace('_', ' ').title()
            default = n.value
            if isinstance(default, ast.Call):
                for kw in default.keywords:
                    if kw.arg == 'description':
                        prop['description'] = ast.literal_eval(kw.value)
                default = default.args[0] if default.args else None
            if default is None or isinstance(default, ast.Constant) and default.value is Ellipsis:
                required.append(n.target.id)
            else:
                prop['default'] = ast.literal_eval(default)
            properties[n.target.id] = prop
        result = {'title': name, 'type': 'object', 'properties': properties}
        if required:
            result['required'] = required
        return result

    def chunks(expr, local):
        if isinstance(expr, ast.Name):
            if expr.id in local:
                return chunks(local[expr.id], local)
            return [{'literal': str(constants[expr.id])}]
        if isinstance(expr, ast.Constant):
            return [{'literal': str(expr.value)}]
        if isinstance(expr, ast.JoinedStr):
            return [c for part in expr.values for c in chunks(part, local)]
        if isinstance(expr, ast.FormattedValue):
            assert expr.conversion == -1 and expr.format_spec is None
            return chunks(expr.value, local)
        if isinstance(expr, ast.Subscript) and isinstance(expr.value, ast.Name) and expr.value.id == 'context':
            return [{'context': ast.literal_eval(expr.slice), 'style': 'python', 'offset': 0}]
        if isinstance(expr, ast.Call) and isinstance(expr.func, ast.Name):
            arg = expr.args[0]
            if isinstance(arg, ast.ListComp):
                assert len(arg.generators) == 1 and not arg.generators[0].ifs
                assert isinstance(arg.elt, ast.Name) and isinstance(arg.generators[0].target, ast.Name)
                assert arg.elt.id == arg.generators[0].target.id
                arg = arg.generators[0].iter
            result = chunks(arg, local)
            assert len(result) == 1 and 'context' in result[0]
            if expr.func.id == 'to_prompt_json':
                result[0]['style'] = 'json'
            elif expr.func.id == 'len':
                result[0]['style'] = 'length'
            else:
                raise ValueError(expr.func.id)
            return result
        if isinstance(expr, ast.BinOp) and isinstance(expr.op, ast.Sub):
            result = chunks(expr.left, local)
            assert len(result) == 1 and result[0]['style'] == 'length'
            result[0]['offset'] -= ast.literal_eval(expr.right)
            return result
        raise ValueError(ast.dump(expr))

    result = {}
    for module, names in FUNCTIONS.items():
        for func in trees[module].body:
            if not isinstance(func, ast.FunctionDef) or func.name not in names:
                continue
            # The observed edge prompt's optional type section is deliberately empty
            # for this deployed MCP profile. Runtime rejects nonempty edge_types.
            for conditional in (n for n in func.body if isinstance(n, ast.If)):
                assert module == 'extract_edges' and func.name == 'edge'
                assert ast.unparse(conditional.test) == "context.get('edge_types')"
            local = {n.targets[0].id: n.value for n in func.body if isinstance(n, ast.Assign)}
            returned = next(n.value for n in func.body if isinstance(n, ast.Return))
            messages = []
            for call in returned.elts:
                assert isinstance(call, ast.Call) and call.func.id == 'Message'
                args = {kw.arg: kw.value for kw in call.keywords}
                messages.append({'role': ast.literal_eval(args['role']), 'chunks': chunks(args['content'], local)})
            model_schema = schema(MODELS[func.name])
            definitions = {}
            def collect_refs(value):
                if isinstance(value, dict):
                    if '$ref' in value:
                        name = value['$ref'].split('/')[-1]
                        if name not in definitions:
                            definitions[name] = schema(name)
                            collect_refs(definitions[name])
                    for child in value.values(): collect_refs(child)
                elif isinstance(value, list):
                    for child in value: collect_refs(child)
            collect_refs(model_schema)
            if definitions: model_schema['$defs'] = definitions
            result[module + '.' + func.name] = {'messages': messages, 'schema': model_schema}
    return {'package': 'graphiti-core 0.28.2 observed source', 'license': 'Apache-2.0; Copyright 2024 Zep Software, Inc.', 'profile': 'deployed MCP: classification-only entities, empty edge_types', 'source_sha256': sources, 'prompts': result}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-root', required=True, type=Path)
    parser.add_argument('--output', default='src/compat/prompts/graphiti-0282.json', type=Path)
    args = parser.parse_args()
    compiled = compile_all(args.source_root)
    # The recorded source files, not an unpinned installed SDK, are authoritative.
    manifest = Path('tests/fixtures/python-0282/prompt-source-hashes.json')
    if manifest.exists():
        assert json.loads(manifest.read_text()) == compiled['source_sha256'], 'prompt source hash mismatch'
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(compiled, ensure_ascii=False, indent=2) + '\n')

if __name__ == '__main__':
    main()
