#!/usr/bin/env python3
"""Validate declared test locations; AST rules check syntax, not test intent."""
import json
import re
import shutil
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

IGNORED = {".git", "node_modules", "target", "dist", ".next", "__pycache__", ".venv"}
AST_VERSION = "0.51.0"
AST_RULES = (
    "test-layout-framework-ts", "test-layout-framework-tsx",
    "test-layout-go-declaration", "test-layout-rust-inline",
    "test-layout-rust-test-scope", "test-layout-rust-test-gate",
)


def inventory(root, roots):
    """Enumerate source paths without treating comments or strings as paths."""
    files = set()
    for prefix in roots:
        folder = root / prefix
        if not folder.is_dir():
            raise ValueError(f"missing configured language root: {prefix}")
        for path in folder.rglob("*"):
            relative = path.relative_to(root)
            if not set(relative.parts) & IGNORED and path.is_file():
                files.add(relative.as_posix())
    return files


def check_tree(root, config):
    errors = []
    pairs = defaultdict(list)
    generated = config["generatedDirectories"]
    files = inventory(root, config["goRoots"] + list(config["tsRoots"]) + config["rustRoots"])
    files = {p for p in files if not any(p.startswith(g + "/") for g in generated)}
    suites = config["integrationFiles"]
    for name, reason in suites.items():
        if name not in files:
            errors.append(f"{name}: registered integration/fixture file is missing")
        if not isinstance(reason, str) or not reason.strip():
            errors.append(f"{name}: integration/fixture registration requires a reason")
    for name in sorted(files):
        path = Path(name)
        if path.suffix not in {".go", ".ts", ".tsx", ".cjs", ".mjs", ".rs"} or name.endswith(".d.ts"):
            continue
        if (root / name).is_symlink():
            errors.append(f"{name}: source/test symlinks need an explicit reviewed policy")
            continue
        go = next((g for g in config["goRoots"] if name.startswith(g + "/")), None)
        ts = next((g for g in config["tsRoots"] if name.startswith(g + "/")), None)
        if go or ts:
            package = go or ts
            relative = path.relative_to(package)
            if "tests" in relative.parts or "__tests__" in relative.parts:
                errors.append(f"{name}: Go/TS integration uses package test/; units belong beside their source")
                continue
            if relative.parts[0] == "test":
                if name not in suites:
                    errors.append(f"{name}: test/ requires an explicit integration or fixture reason in test-layout.json")
                continue
            if name in suites:
                errors.append(f"{name}: registered integration/fixture must be inside {package}/test/")
            if go and name.endswith("_test.go"):
                source = name[:-8] + ".go"
                if source not in files or source.endswith("_test.go"):
                    errors.append(f"{name}: no corresponding source {source}; merge scenarios into the source's unique _test.go")
                else:
                    pairs[source].append(name)
            if ts:
                match = re.fullmatch(r"(.+)\.(test|spec)\.(ts|tsx|cjs|mjs)", name)
                if match:
                    stem, suffix, extension = match.groups()
                    source = stem + "." + extension
                    if extension not in {"ts", "tsx"} or source not in files:
                        errors.append(f"{name}: unit test must match the source path and .ts/.tsx extension")
                    else:
                        pairs[source].append(name)
                    if suffix != config["tsRoots"][ts]:
                        errors.append(f"{name}: package consistently uses .{config['tsRoots'][ts]}, not .{suffix}")
        if path.suffix == ".rs":
            ancestors = path.parents
            crate = next((a for a in ancestors if (root / a / "Cargo.toml").is_file()), None)
            if crate is None:
                errors.append(f"{name}: Rust source has no owning Cargo.toml")
                continue
            relative = path.relative_to(crate)
            if "tests" in relative.parts and relative.parts[0] != "tests":
                errors.append(f"{name}: Rust integration belongs in crate tests/; units stay in their source")
            if relative.parts[0] == "src" and re.search(r"(?:_tests?|\.tests?)\.rs$|(?:^|/)tests\.rs$", name):
                errors.append(f"{name}: separate Rust unit-test file; inline into the owner's unique #[cfg(test)] mod tests")
    for source, tests in sorted(pairs.items()):
        if len(tests) != 1:
            errors.append(f"{source}: multiple unit-test files: {', '.join(tests)}")
    return errors, len(pairs)


def run_json(command, root, allow_no_match=False):
    result = subprocess.run(command, cwd=root, capture_output=True, text=True)
    if result.returncode not in (0, 1):
        raise ValueError(result.stderr.strip() or f"AST command failed: {result.returncode}")
    try:
        data = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise ValueError("AST command did not return a JSON diagnostic array") from error
    if not isinstance(data, list):
        raise ValueError("AST command did not return a diagnostic array")
    # Code 1 is normal only when diagnostics actually exist; parse/config errors
    # must never masquerade as an empty successful scan.
    if result.returncode and not data and not allow_no_match:
        raise ValueError(result.stderr.strip() or "AST command failed without diagnostics")
    return data


def check_ast(root, binary, config):
    version = subprocess.run([binary, "--version"], capture_output=True, text=True, check=True).stdout.strip()
    if version != f"ast-grep {AST_VERSION}":
        raise ValueError(f"expected ast-grep {AST_VERSION}, found {version}")
    rows = []
    for rule in AST_RULES:
        rule_file = root / ".ast-grep" / "rules" / (rule + ".yml")
        if not rule_file.is_file():
            raise ValueError(f"missing required AST rule: {rule_file}")
        rows += run_json([binary, "scan", "-r", str(rule_file), "--error", "--json"], root)
    errors = [f"{r['file']}:{r['range']['start']['line'] + 1}: {r['ruleId']}: {r['message']}" for r in rows]
    rust = sorted(p for p in inventory(root, config["rustRoots"]) if p.endswith(".rs") and "/src/" in p)
    if rust:
        rows = run_json([binary, "run", "--lang", "Rust", "--pattern", "mod tests { $$$BODY }", "--json", *rust], root, allow_no_match=True)
        modules = defaultdict(list)
        for row in rows:
            modules[row["file"]].append(row["range"]["start"]["line"] + 1)
        for name, lines in sorted(modules.items()):
            if len(lines) > 1:
                errors.append(f"{name}: multiple inline mod tests at lines {lines}; merge under one container")
    return errors


def main():
    root = Path(__file__).resolve().parent.parent
    binary = shutil.which("ast-grep")
    if binary is None:
        print(f"test layout: ast-grep {AST_VERSION} is required; install: npm install --global @ast-grep/cli@{AST_VERSION}", file=sys.stderr)
        return 2
    try:
        config = json.loads((root / "test-layout.json").read_text())
        errors, pairs = check_tree(root, config)
        errors += check_ast(root, binary, config)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"test layout: {error}", file=sys.stderr)
        return 2
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"Test layout passed: {pairs} source/test pairs; registered integration files and Rust inline containers checked.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
