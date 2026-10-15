# Pinned upstream temporal oracle

`temporal_edge_utils.py` is an unmodified source file from
[getzep/graphiti](https://github.com/getzep/graphiti), commit
`aca369364226a666ca541b061ff2a1a54b80f2ff`, licensed under Apache-2.0.
The repository root LICENSE contains the license text. `manifest.json` records
its source path and SHA-256 hash.

`scripts/python-upstream-temporal.py` verifies that hash and executes only the
three compiled pattern assignments and pure normalization function from its AST.
It does not import Graphiti or run the edge mutation helper. This fixture is only
an independent test oracle; there is no Python server runtime dependency.
