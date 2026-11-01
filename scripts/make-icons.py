#!/usr/bin/env python3
"""Builds crates/tidewatch-core/assets/lucide/icons.pack from the lucide-static npm package and
copies its LICENSE next to it. Needs only Python 3.

    scripts/make-icons.py [--work DIR]

The tarball goes to DIR (default target/icon-src) and is reused on the next run.
"""

import argparse
import json
import struct
import sys
import tarfile
import urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path

LUCIDE_VERSION = "1.59.0"
TARBALL = f"https://registry.npmjs.org/lucide-static/-/lucide-static-{LUCIDE_VERSION}.tgz"
MAGIC = b"TWLU1"
MAX_PACK_BYTES = 1_000_000

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "crates" / "tidewatch-core" / "assets" / "lucide"


def fetch(url, dest):
    if not dest.exists():
        dest.parent.mkdir(parents=True, exist_ok=True)
        with urllib.request.urlopen(url, timeout=120) as reply:
            dest.write_bytes(reply.read())
    return dest


def body_of(nodes):
    parts = []
    for tag, attrs in nodes:
        inner = " ".join(f'{key}="{value}"' for key, value in attrs.items() if key != "key")
        parts.append(f"<{tag} {inner}/>")
    return "".join(parts)


def body_of_svg(text):
    root = ET.fromstring(text.split("-->", 1)[-1])
    return body_of([(child.tag.split("}")[-1], dict(child.attrib)) for child in root])


def pack(entries):
    """Identical bodies are stored once: an alias shares its icon's offset and length."""
    entries = sorted(entries.items())
    index, blobs, seen, offset = b"", [], {}, 0
    for name, blob in entries:
        if blob not in seen:
            seen[blob] = offset
            blobs.append(blob)
            offset += len(blob)
        key = name.encode()
        index += struct.pack("<B", len(key)) + key + struct.pack("<II", seen[blob], len(blob))
    return MAGIC + struct.pack("<I", len(entries)) + index + b"".join(blobs)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--work", type=Path, default=ROOT / "target" / "icon-src")
    args = parser.parse_args()
    tarball = fetch(TARBALL, args.work / f"lucide-static-{LUCIDE_VERSION}.tgz")
    with tarfile.open(tarball) as tar:
        nodes = json.load(tar.extractfile("package/icon-nodes.json"))
        license_text = tar.extractfile("package/LICENSE").read()
        entries = {name: body_of(parts).encode() for name, parts in nodes.items()}
        for member in tar.getmembers():
            path = Path(member.name)
            if path.parent.name == "icons" and path.suffix == ".svg" and path.stem not in entries:
                text = tar.extractfile(member).read().decode()
                entries[path.stem] = body_of_svg(text).encode()
    blob = pack(entries)
    if len(blob) > MAX_PACK_BYTES:
        sys.exit(f"{len(blob)} bytes is over the {MAX_PACK_BYTES} byte budget")
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "icons.pack").write_bytes(blob)
    (OUT / "LICENSE").write_bytes(license_text)
    print(f"lucide-static {LUCIDE_VERSION}: {len(entries)} icons, {len(blob) / 1e3:.0f} kB")


if __name__ == "__main__":
    main()
