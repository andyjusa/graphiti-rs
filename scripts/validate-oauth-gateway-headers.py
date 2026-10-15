#!/usr/bin/env python3
"""Test the two-header patch in a temporary copy of verified gateway source.

Requires an exclusively owned disposable DB. Does not edit the input source,
load operational credentials, log in to GitHub, invoke providers or deploy.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--baseline', action='store_true')
    args = parser.parse_args()
    assert os.environ.get('COMPAT_TEST_DB_URL') == 'redis://127.0.0.1:16379', 'explicit disposable DB required'
    repo = Path(__file__).resolve().parents[1]
    assets = repo / 'integration/oauth-gateway'
    manifest = json.loads((assets / 'baseline.json').read_text())
    source = args.source.resolve()
    for name, expected in manifest['baseline_sha256'].items():
        assert hashlib.sha256((source / name).read_bytes()).hexdigest() == expected, name
    os.umask(0o077)
    with tempfile.TemporaryDirectory(prefix='graphiti-oauth-header-tests-') as directory:
        root = Path(directory)
        for name in ['Cargo.toml', 'Cargo.lock', 'src/main.rs', 'src/lib.rs']:
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source / name, target)
        shutil.copytree(source / 'tests', root / 'tests')
        if not args.baseline:
            patch = assets / 'standard-headers.patch'
            subprocess.run(['git', 'apply', '--check', str(patch)], cwd=root, check=True)
            subprocess.run(['git', 'apply', str(patch)], cwd=root, check=True)
        cargo = root / 'Cargo.toml'
        text = cargo.read_text()
        marker = '[dev-dependencies]\n'
        assert text.count(marker) == 1
        deps = 'graphiti-rs = { path = ' + json.dumps(str(repo), ensure_ascii=False) + ', features = ["graphiti-compat"] }\ntokio-util = "0.7"\n'
        cargo.write_text(text.replace(marker, marker + deps))
        main_rs = root / 'src/main.rs'
        fixture = repo / 'tests/fixtures/oauth-gateway-headers.rs'
        main_rs.write_text(main_rs.read_text() + '\n#[cfg(test)]\n#[path = ' + json.dumps(str(fixture), ensure_ascii=False) + ']\nmod transport_header_tests;\n')
        report = root / 'comparison.json'
        env = {'PATH': os.environ['PATH'], 'LANG': 'C.UTF-8',
               'COMPAT_TEST_DB_URL': 'redis://127.0.0.1:16379',
               'CARGO_TARGET_DIR': str(repo / 'target/oauth-header-validation'),
               'OAUTH_HEADER_REPORT': str(report), 'OAUTH_HEADER_EXPECT_FORWARDING': '0' if args.baseline else '1'}
        command = ['cargo', 'test', '--offline']
        if args.baseline:
            command += ['--bin', 'graphiti-oauth-gateway', 'transport_header_tests::modern_headers_and_legacy_requests_preserve_authentication_and_transport_validation', '--', '--ignored', '--exact']
        else:
            command += ['--', '--include-ignored', '--test-threads=1']
        subprocess.run(command, cwd=root, env=env, check=True)
        for name, expected in manifest['baseline_sha256'].items():
            assert hashlib.sha256((source / name).read_bytes()).hexdigest() == expected, name
        args.report.resolve().write_text(report.read_text())
    print(json.dumps({'report': str(args.report.resolve()), 'baseline': args.baseline, 'source_unmodified': True}))


if __name__ == '__main__':
    main()
