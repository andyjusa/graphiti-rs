#!/usr/bin/env python3
"""Compare observed/patched gateway binaries on an owned isolated Pi network.

No GitHub login, external provider, original memory or operational route is used.
Only count/status/guard evidence is written; synthetic credentials stay private.
"""
import http.client
import json
import os
from pathlib import Path
import subprocess
import time

assert os.environ.get('GRAPHITI_PI_SYNTHETIC') == 'graphiti-rs-oauth-headers'
ROOT = Path(__file__).resolve().parent
manifest = json.loads((ROOT / 'resources.json').read_text())
LABEL = 'oauth-headers-pi-20261002'


def inspect(kind, identity):
    return json.loads(subprocess.check_output(['docker', kind, 'inspect', identity]))[0]


network = inspect('network', manifest['network'])
assert network['Internal'] and network['Labels']['graphiti-rs-task'] == LABEL
addresses = {}
for role, cid in manifest['containers'].items():
    info = inspect('container', cid)
    assert info['Id'] == cid and info['Config']['Labels']['graphiti-rs-task'] == LABEL
    assert info['Config']['Labels']['role'] == role
    assert info['HostConfig']['ReadonlyRootfs'] and not info['HostConfig']['PortBindings']
    assert set(info['NetworkSettings']['Networks']) == {network['Name']}
    assert all(Path(m['Source']).is_relative_to(ROOT) for m in info['Mounts'] if m['Type'] == 'bind')
    addresses[role] = info['NetworkSettings']['Networks'][network['Name']]['IPAddress']
token = (ROOT / 'secrets/oauth-test-token').read_text().strip()
expired = (ROOT / 'secrets/oauth-expired-token').read_text().strip()
contract = json.loads((ROOT / 'tool-contract.json').read_text())['tools']['tools']


def request(version, method, name=None):
    params = {}
    if version == '2026-07-28':
        params['_meta'] = {'io.modelcontextprotocol/protocolVersion': version,
                           'io.modelcontextprotocol/clientCapabilities': {}}
    if method == 'initialize':
        params = {'protocolVersion': version, 'capabilities': {},
                  'clientInfo': {'name': 'synthetic-pi-gateway', 'version': '1'}}
    headers = [('Host', 'localhost:8000'), ('Authorization', 'Bearer ' + token),
               ('Accept', 'application/json, text/event-stream'), ('Content-Type', 'application/json'),
               ('MCP-Protocol-Version', version)]
    if version == '2026-07-28':
        headers.append(('Mcp-Method', method))
    if name is not None:
        params.update(name=name, arguments={})
        if version == '2026-07-28':
            headers.append(('Mcp-Name', name))
    return headers, {'jsonrpc': '2.0', 'id': 1, 'method': method, 'params': params}


def call(role, headers, body):
    data = json.dumps(body).encode()
    conn = http.client.HTTPConnection(addresses[role], 8010, timeout=10)
    try:
        conn.putrequest('POST', '/chatgpt/mcp', skip_host=True, skip_accept_encoding=True)
        for key, value in headers:
            conn.putheader(key, value)
        conn.putheader('Content-Length', str(len(data)))
        conn.endheaders(data)
        response = conn.getresponse()
        raw = response.read()
        if response.status == 401:
            assert 'oauth-protected-resource/chatgpt/mcp' in response.getheader('www-authenticate', '')
        return response.status, json.loads(raw) if raw else {}
    finally:
        conn.close()


def main():
    for role in ['baseline', 'patched']:
        for _ in range(100):
            try:
                status, _ = call(role, *request('2025-11-25', 'tools/call', 'get_status'))
                if status == 200:
                    break
            except (OSError, ValueError):
                pass
            time.sleep(.1)
        else:
            raise RuntimeError('owned gateway/backend not ready')
    reports = []
    for role in ['baseline', 'patched']:
        forwarded = role == 'patched'
        events = []
        for method, name in [('server/discover', None), ('tools/list', None), ('tools/call', 'get_status')]:
            status, payload = call(role, *request('2026-07-28', method, name))
            if forwarded:
                assert status == 200 and 'result' in payload, (role, method, status)
                if method == 'tools/list':
                    assert payload['result']['tools'] == contract
                if method == 'tools/call':
                    assert payload['result']['structuredContent']['status'] == 'ok'
            else:
                assert status == 400 and payload['error']['code'] == -32020
            events.append({'case': 'modern_' + method, 'status': status,
                           'rpc_error': payload.get('error', {}).get('code')})
        for method, name in [('initialize', None), ('tools/list', None), ('tools/call', 'get_status')]:
            status, payload = call(role, *request('2025-11-25', method, name))
            assert status == 200 and 'result' in payload
            if method == 'tools/list':
                assert payload['result']['tools'] == contract
            events.append({'case': 'legacy_' + method, 'status': status})
        for header, value in [('Mcp-Method', 'tools/list'), ('Mcp-Method', None),
                              ('Mcp-Name', 'clear_graph'), ('Mcp-Name', None)]:
            headers, body = request('2026-07-28', 'tools/call', 'get_status')
            headers = [(k, v) for k, v in headers if k != header]
            if value is not None:
                headers.append((header, value))
            status, payload = call(role, headers, body)
            assert status == 400 and payload['error']['code'] == -32020
            events.append({'case': 'reject_' + header + ('_missing' if value is None else '_mismatch'),
                           'status': status, 'rpc_error': -32020})
        for header, value, kind in [('Mcp-Method', 'tools/list', 'different'),
                                    ('Mcp-Method', 'tools/call', 'identical'),
                                    ('Mcp-Name', 'clear_graph', 'different'),
                                    ('Mcp-Name', 'get_status', 'identical')]:
            headers, body = request('2026-07-28', 'tools/call', 'get_status')
            headers.append((header, value))
            status, payload = call(role, headers, body)
            assert status == 400 and payload['error']['code'] == -32020
            events.append({'case': 'reject_' + header + '_duplicate_' + kind,
                           'status': status, 'rpc_error': -32020})
        for label, auth in [('missing', None), ('invalid', 'Bearer synthetic-invalid'),
                            ('expired', 'Bearer ' + expired)]:
            headers, body = request('2026-07-28', 'tools/list')
            headers = [(k, v) for k, v in headers if k != 'Authorization']
            if auth is not None:
                headers.append(('Authorization', auth))
            status, _ = call(role, headers, body)
            assert status == 401
            events.append({'case': 'reject_token_' + label, 'status': status})
        headers, body = request('2026-07-28', 'tools/list')
        headers.append(('Origin', 'https://evil.test'))
        status, _ = call(role, headers, body)
        assert status == 401
        events.append({'case': 'reject_origin', 'status': status})
        assert len(events) == 18
        reports.append({'variant': role, 'headers_forwarded': forwarded, 'events': events})
    result = {'scope': 'Observed OAuth image vs patched ARM64 gateway on actual Pi4; fresh empty disposable database, actual Rust MCP backend and seeded synthetic tokens',
              'variants': reports, 'cases': 36, 'ten_tool_schemas_match': True,
              'actual_provider_calls': 0, 'operational_writes': 0,
              'fresh_github_login_tested': False, 'operational_routes_changed': False}
    (ROOT / 'evidence/pi-summary.json').write_text(json.dumps(result, indent=2))
    print(json.dumps({'cases': 36, 'all_expected_results_match': True,
                      'actual_provider_calls': 0, 'operational_writes': 0}))


if __name__ == '__main__':
    main()
