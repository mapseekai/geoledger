"""Provision disposable test accounts through the private authentication API.

Never use this helper against a production service. Secrets are written only to
mode-0600 files owned by the caller, and are never printed.
"""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import secrets
import urllib.request


def auth_call(base: str, operation: str, body: dict, session: str | None = None):
    headers = {"Content-Type": "application/json"}
    if session:
        headers["x-geoledger-session"] = session
    request = urllib.request.Request(base + "/auth/v1/" + operation,
                                     data=json.dumps(body).encode(), headers=headers)
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def private_file(path: Path, content: str) -> None:
    with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as output:
        output.write(content)


def provision(base: str, data_dir: Path, output: Path) -> dict:
    files = list(data_dir.glob("initial-admin-*.json"))
    if len(files) != 1:
        raise ValueError("expected one disposable initial administrator file")
    initial = json.loads(files[0].read_text())
    login = auth_call(base, "login_password", initial)
    password = secrets.token_urlsafe(32)
    changed = auth_call(base, "change_password", {"current_password": initial["password"], "new_password": password}, login["session"])
    key = auth_call(base, "create_api_key", {"name": "disposable integration", "current_password": password}, changed["session"])
    member = auth_call(base, "create_user", {"username": "sdk-" + secrets.token_hex(8), "password": secrets.token_urlsafe(32), "current_password": password}, changed["session"])
    private_file(Path(str(output) + ".member-id"), member["id"] + "\n")
    private_file(output, key["api_key"] + "\n")
    private_file(Path(str(output) + ".password"), password + "\n")
    return {"session": changed["session"], "password": password, "user": changed["user"], "api_key": key["api_key"]}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True)
    parser.add_argument("--data-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    provision(args.base, args.data_dir, args.output)
