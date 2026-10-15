#!/usr/bin/env python3
"""Assemble native OCI artifacts without changing image or attestation blobs.

The resulting index must pass image-gate scan and verify before publication.
This operation does not create or reuse a scan receipt.
"""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil

spec = importlib.util.spec_from_file_location('image_gate', Path(__file__).with_name('image-gate.py'))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


def merge(inputs: list[Path], output: Path, sha: str) -> None:
    if len(inputs) != 2:
        raise ValueError('exactly two native image layouts are required')
    manifests = []
    platforms = set()
    for layout in inputs:
        root, images = gate.inventory(layout)
        if len(images) != 1 or platforms.intersection(images):
            raise ValueError('each native artifact must supply one distinct platform')
        platforms.update(images)
        for platform, digest in images.items():
            gate.verify_image_identity(layout, digest, platform, sha)

        def leaves(descriptor):
            document = json.loads((layout / 'blobs/sha256' / descriptor['digest'][7:]).read_text())
            if 'manifests' in document:
                for child in document['manifests']:
                    leaves(child)
            else:
                manifests.append(descriptor)

        leaves(json.loads((layout / 'index.json').read_text())['manifests'][0])
    if platforms != gate.DEFAULT_PLATFORMS:
        raise ValueError('merged image requires linux/amd64 and linux/arm64')
    output.mkdir(parents=True, exist_ok=False)
    blobs = output / 'blobs/sha256'
    blobs.mkdir(parents=True)
    for layout in inputs:
        for source in (layout / 'blobs/sha256').iterdir():
            if (source.is_symlink() or not source.is_file()
                    or not gate.DIGEST.fullmatch('sha256:' + source.name)
                    or gate.digest_file(source) != 'sha256:' + source.name):
                raise ValueError('invalid source blob')
            destination = blobs / source.name
            if not destination.exists():
                shutil.copyfile(source, destination)
    media = 'application/vnd.oci.image.index.v1+json'
    data = json.dumps({'schemaVersion': 2, 'mediaType': media, 'manifests': manifests},
                      sort_keys=True, separators=(',', ':')).encode()
    digest = hashlib.sha256(data).hexdigest()
    (blobs / digest).write_bytes(data)
    (output / 'index.json').write_text(json.dumps({'schemaVersion': 2, 'manifests': [
        {'mediaType': media, 'digest': 'sha256:' + digest, 'size': len(data)}]}) + '\n')
    (output / 'oci-layout').write_text('{"imageLayoutVersion":"1.0.0"}\n')
    gate.inventory(output)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', type=Path, action='append', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--source-sha', required=True)
    args = parser.parse_args()
    merge(args.input, args.output, args.source_sha)
