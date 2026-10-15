"""Execute only the pinned pure upstream temporal helper with stdlib inputs."""
import ast
import datetime
import hashlib
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1] / "tests/fixtures/upstream-main"
PATTERNS = {"_FUTURE_ASSERTION_RE", "_DEADLINE_RE", "_ONGOING_STATE_END_RE"}


def utc(value):
    if value is None:
        return None
    if value.tzinfo is None:
        return value.replace(tzinfo=datetime.timezone.utc)
    return value.astimezone(datetime.timezone.utc)


def dt(value):
    return None if value is None else datetime.datetime.fromisoformat(value.replace("Z", "+00:00"))


def main():
    manifest = json.loads((ROOT / "manifest.json").read_text())
    source = (ROOT / "temporal_edge_utils.py").read_bytes()
    assert hashlib.sha256(source).hexdigest() == manifest["files"]["temporal_edge_utils.py"]["sha256"]
    selected = []
    for node in ast.parse(source).body:
        if isinstance(node, ast.Assign) and any(
            isinstance(target, ast.Name) and target.id in PATTERNS for target in node.targets
        ):
            selected.append(node)
        elif isinstance(node, ast.FunctionDef) and node.name == "normalize_future_temporal_bounds":
            selected.append(node)
    assert len(selected) == 4, "pinned temporal helper structure changed"
    namespace = {"re": re, "datetime": datetime.datetime, "ensure_utc": utc}
    exec(compile(ast.Module(body=selected, type_ignores=[]), "pinned-upstream-temporal", "exec"), namespace)
    results = []
    for case in json.load(sys.stdin):
        result = namespace["normalize_future_temporal_bounds"](
            case["fact"], dt(case["reference"]), dt(case["valid"]), dt(case["invalid"])
        )
        results.append([None if value is None else value.isoformat() for value in result])
    json.dump(results, sys.stdout)


if __name__ == "__main__":
    main()
