#!/usr/bin/env python3
"""Keep .env.example in sync with the configuration the code actually reads.

Every GL_* flag of geoledger-server and gl (clap `env = "..."`) must be listed,
and every listed GL_* variable must still be read somewhere in the repository.
"""
import pathlib
import re
import sys

root = pathlib.Path(__file__).resolve().parent.parent
example = root / ".env.example"
flag_sources = [root / "crates/server/src/main.rs", root / "crates/cli/src/main.rs"]
search_roots = ["crates", "sdk", "scripts", "web/src", "deploy", "compose.yaml", "Dockerfile"]
skip_dirs = {"node_modules", "target", "dist", "_internal", "__pycache__", ".next"}

listed = set(re.findall(r"^#?\s*(GL_[A-Z0-9_]+)=", example.read_text(), re.M))
flags = set()
for source in flag_sources:
    flags |= set(re.findall(r'env\s*=\s*"(GL_[A-Z0-9_]+)"', source.read_text()))

used = set()
for name in search_roots:
    base = root / name
    paths = [base] if base.is_file() else base.rglob("*")
    for path in paths:
        if not path.is_file() or skip_dirs.intersection(path.relative_to(root).parts):
            continue
        if path.suffix not in {".rs", ".py", ".sh", ".ts", ".tsx", ".go", ".yaml", ".yml", ".cjs", ""}:
            continue
        try:
            used |= set(re.findall(r"\bGL_[A-Z0-9_]+\b", path.read_text()))
        except UnicodeDecodeError:
            continue

errors = [f"{name} is a server/CLI flag but missing from .env.example" for name in sorted(flags - listed)]
errors += [f"{name} is in .env.example but no code or deployment file reads it" for name in sorted(listed - used)]
for error in errors:
    print(error, file=sys.stderr)
if errors:
    sys.exit(1)
print(f"Validated {len(listed)} variables in .env.example against {len(flags)} flags.")
