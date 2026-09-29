#!/usr/bin/env python3
"""Check repository documentation links and current GeoLedger terminology."""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import re
import subprocess
import sys
import time
from urllib.error import URLError
from urllib.parse import unquote, urlsplit
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
LINK = re.compile(r"\[[^\]\n]+\]\(([^\s)]+)(?:\s+\"[^\"]*\")?\)")
URL = re.compile(r"https?://[^\s<>\)\]\"'`]+")
STALE = re.compile(r"spatial[-_.]version|SpatialVersion|\bSV_|/Users/|/tmp/|/path/to/|\.reference/|rename-to-geoledger")
NEGATIVE = re.compile(r"不支持|尚未|尚无|暂不|不兼容|未实现")


def anchors(path: Path) -> set[str]:
    result: set[str] = set()
    counts: dict[str, int] = {}
    fenced = False
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.lstrip().startswith("```"):
            fenced = not fenced
        if fenced or not re.match(r"^#{1,6}\s", line):
            continue
        heading = re.sub(r"^#{1,6}\s+", "", line).strip().rstrip("#").strip()
        slug = re.sub(r"[^\w\- ]", "", heading.lower()).replace(" ", "-")
        number = counts.get(slug, 0)
        result.add(f"{slug}-{number}" if number else slug)
        counts[slug] = number + 1
    return result


def check_url(url: str) -> str | None:
    for attempt in range(2):
        try:
            request = Request(url, headers={"User-Agent": "GeoLedger-documentation-check/1.0"})
            with urlopen(request, timeout=20) as response:
                if response.status != 200:
                    return f"{url}: HTTP {response.status}"
            return None
        except (URLError, TimeoutError, OSError) as error:
            if attempt:
                return f"{url}: {error}"
            time.sleep(0.5)
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--external", action="store_true", help="Also verify public HTTP links")
    args = parser.parse_args()
    indexed = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=ROOT
    ).decode().split("\0")
    tracked = {name for name in indexed if name and (ROOT / name).is_file()}
    documents = [ROOT / "README.md", ROOT / "AGENTS.md", *sorted((ROOT / "docs").glob("*.md"))]
    errors: list[str] = []
    external: set[str] = set()
    local_count = 0
    for document in documents:
        text = document.read_text(encoding="utf-8")
        label = document.relative_to(ROOT)
        for pattern in [STALE, NEGATIVE]:
            for match in pattern.finditer(text):
                line = text.count("\n", 0, match.start()) + 1
                errors.append(f"{label}:{line}: review terminology: {match.group()}")
        for target in LINK.findall(text):
            parsed = urlsplit(target)
            if parsed.scheme in {"http", "https"}:
                external.add(target)
                continue
            if parsed.scheme:
                errors.append(f"{label}: unexpected link scheme: {target}")
                continue
            destination = (document.parent / unquote(parsed.path)).resolve() if parsed.path else document
            try:
                relative = destination.relative_to(ROOT).as_posix()
            except ValueError:
                errors.append(f"{label}: link leaves repository: {target}")
                continue
            available = relative in tracked or (destination.is_dir() and any(p.startswith(relative + "/") for p in tracked))
            if not available:
                errors.append(f"{label}: missing repository target: {target}")
                continue
            if parsed.fragment and destination.suffix == ".md" and unquote(parsed.fragment) not in anchors(destination):
                errors.append(f"{label}: missing heading: {target}")
            local_count += 1
        for url in URL.findall(text):
            parsed = urlsplit(url)
            if parsed.hostname not in {"127.0.0.1", "localhost", "::1"}:
                external.add(url.rstrip(".,;"))
    if args.external:
        with ThreadPoolExecutor(max_workers=4) as pool:
            errors.extend(error for error in pool.map(check_url, sorted(external)) if error)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"Validated {local_count} repository links in {len(documents)} documents.")
    if args.external:
        print(f"Validated {len(external)} public HTTP links.")
    else:
        print(f"Found {len(external)} public HTTP links; --external verifies reachability.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
