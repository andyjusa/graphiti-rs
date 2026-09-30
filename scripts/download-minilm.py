#!/usr/bin/env python3
"""Explicitly fetch a pinned BERT embedding model; never run automatically."""
import pathlib
import sys
import urllib.request
REPO = "sentence-transformers/all-MiniLM-L6-v2"
REVISION = "1110a243fdf4706b3f48f1d95db1a4f5529b4d41"
path = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "models/minilm")
path.mkdir(parents=True, exist_ok=True)
for name in ("config.json", "tokenizer.json", "model.safetensors"):
    print(f"Downloading {name} at {REVISION}")
    urllib.request.urlretrieve(f"https://huggingface.co/{REPO}/resolve/{REVISION}/{name}", path / name)
(path / "revision.txt").write_text(REVISION + "\n")
print("Model preparation complete. Runtime does not download models.")
