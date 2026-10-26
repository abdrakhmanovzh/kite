#!/usr/bin/env python3
"""Remove this project's Windows-only Cargo cache entries."""
import json
import pathlib
import re
import shutil
import subprocess

project = pathlib.Path(__file__).resolve().parent.parent
metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--offline", "--locked", "--format-version", "1",
    "--filter-platform", "aarch64-apple-darwin",
], cwd=project))
resolved = {node["id"] for node in metadata["resolve"]["nodes"]}
needed = {(p["name"], p["version"]) for p in metadata["packages"] if p["id"] in resolved}
names = set()
for block in (project / "Cargo.lock").read_text().split("[[package]]")[1:]:
    name = re.search(r'^name = "([^"]+)"', block, re.M).group(1)
    version = re.search(r'^version = "([^"]+)"', block, re.M).group(1)
    if (name.startswith(("windows", "winapi", "winreg")) or name == "uds_windows") and (name, version) not in needed:
        names.add(name + "-" + version)

registry = pathlib.Path.home() / ".cargo/registry"
paths = [p for kind in ("src", "cache") for directory in (registry / kind).iterdir()
         if directory.is_dir() for p in directory.iterdir()
         if p.name.removesuffix(".crate") in names]
size = sum(f.stat().st_size for p in paths
           for f in (p.rglob("*") if p.is_dir() else [p]) if f.is_file())
for path in paths:
    if path.is_dir():
        shutil.rmtree(path)
    else:
        path.unlink()
print(f"Removed {len(paths)} Windows cache entries ({size / 1024**2:.1f} MiB).")
