#!/usr/bin/env python3
"""Builds still.pack (Noto colour emoji, 72 px PNG) in crates/tidewatch/assets/emoji. Needs git,
pngquant, oxipng and Pillow.

    scripts/make-emoji.py [--work DIR]

Downloads go to DIR (default target/emoji-src) and are reused on the next run.
"""

import argparse
import math
import re
import struct
import subprocess
import sys
import urllib.request
from multiprocessing import Pool
from pathlib import Path

from PIL import Image, ImageChops, ImageStat

NOTO_TAG = "v2.051"
NOTO_GIT = "https://github.com/googlefonts/noto-emoji"
NOTO_CDN = "https://fonts.gstatic.com/s/e/notoemoji/latest"
EMOJI_TEST = "https://unicode.org/Public/emoji/latest/emoji-test.txt"

STILL_PX = 72
STILL_COLOURS = 96
MIN_PSNR = 40.0
FE0F = 0xFE0F

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "crates" / "tidewatch" / "assets" / "emoji"


def key_of(codepoints):
    return "_".join(f"{cp:x}" for cp in codepoints if cp != FE0F)


def fetch(url, dest):
    if not dest.exists():
        dest.parent.mkdir(parents=True, exist_ok=True)
        with urllib.request.urlopen(url, timeout=120) as reply:
            dest.write_bytes(reply.read())
    return dest


def pack(entries, dest):
    entries = sorted(entries.items())
    index, offset = b"", 0
    for key, blob in entries:
        index += struct.pack("<B", len(key)) + key.encode() + struct.pack("<II", offset, len(blob))
        offset += len(blob)
    head = b"TWEP1" + struct.pack("<I", len(entries))
    dest.write_bytes(head + index + b"".join(blob for _, blob in entries))


def psnr(a, b):
    ground = Image.new("RGBA", a.size, (24, 26, 32, 255))
    a = Image.alpha_composite(ground, a.convert("RGBA")).convert("RGB")
    b = Image.alpha_composite(ground, b.convert("RGBA")).convert("RGB")
    squares = ImageStat.Stat(ImageChops.difference(a, b)).sum2
    mse = sum(squares) / (a.width * a.height * 3)
    return 99.0 if mse == 0 else 10 * math.log10(255 * 255 / mse)


def fully_qualified(test_file):
    found = []
    for line in test_file.read_text().splitlines():
        match = re.match(r"^([0-9A-F ]+?)\s*;\s*fully-qualified", line)
        if match:
            found.append([int(cp, 16) for cp in match.group(1).split()])
    return found


def shrink_png(blob_path_pairs):
    src, dest = blob_path_pairs
    image = Image.open(src).convert("RGBA")
    if image.size != (STILL_PX, STILL_PX):
        image = image.resize((STILL_PX, STILL_PX), Image.LANCZOS)
    image.save(dest)


def compress_pngs(files):
    names = [str(f) for f in files]
    for start in range(0, len(names), 200):
        batch = names[start : start + 200]
        subprocess.run(
            ["pngquant", "--force", "--speed", "1", "--strip", "--ext", ".png", str(STILL_COLOURS), *batch],
            check=False,
        )
        subprocess.run(["oxipng", "-q", "-o", "6", "--strip", "safe", *batch], check=True)


def build_stills(work):
    clone = work / "noto-emoji"
    if not clone.exists():
        subprocess.run(
            ["git", "clone", "--depth", "1", "--branch", NOTO_TAG, "--filter=blob:none", "--sparse", NOTO_GIT, str(clone)],
            check=True,
        )
        subprocess.run(["git", "-C", str(clone), "sparse-checkout", "set", "png/72"], check=True)
    source = clone / "png" / "72"
    originals = {}
    for path in source.glob("emoji_u*.png"):
        originals[key_of(int(cp, 16) for cp in path.stem[len("emoji_u") :].split("_"))] = path
    staged = work / "stills"
    staged.mkdir(exist_ok=True)
    jobs, wanted = [], {}
    for key, path in originals.items():
        wanted[key] = staged / f"{key}.png"
        jobs.append((path, wanted[key]))
    test = fetch(EMOJI_TEST, work / "emoji-test.txt")
    for cps in fully_qualified(test):
        key = key_of(cps)
        if key in wanted:
            continue
        code = "_".join(f"{cp:04x}" if cp < 0x10000 else f"{cp:x}" for cp in cps)
        try:
            cdn = fetch(f"{NOTO_CDN}/{code}/512.png", work / "static512" / f"{key}.png")
        except OSError:
            print(f"no still picture for {key}", file=sys.stderr)
            continue
        wanted[key] = staged / f"{key}.png"
        jobs.append((cdn, wanted[key]))
    with Pool() as pool:
        pool.map(shrink_png, jobs)
    before = sum(p.stat().st_size for p in wanted.values())
    compress_pngs(list(wanted.values()))
    after = sum(p.stat().st_size for p in wanted.values())
    worst = min(
        (psnr(Image.open(src).convert("RGBA").resize((STILL_PX, STILL_PX)), Image.open(wanted[key])), key)
        for key, src in list(originals.items())[:: max(1, len(originals) // 60)]
    )
    print(f"stills: {len(wanted)} pictures, {before / 1e6:.1f} MB -> {after / 1e6:.1f} MB, worst PSNR of a sample {worst[0]:.1f} dB ({worst[1]})")
    if worst[0] < MIN_PSNR:
        sys.exit(f"PSNR below {MIN_PSNR} dB: raise STILL_COLOURS")
    pack({key: path.read_bytes() for key, path in wanted.items()}, OUT / "still.pack")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--work", type=Path, default=ROOT / "target" / "emoji-src")
    args = parser.parse_args()
    args.work.mkdir(parents=True, exist_ok=True)
    OUT.mkdir(parents=True, exist_ok=True)
    build_stills(args.work)
    print(f"still.pack: {(OUT / 'still.pack').stat().st_size / 1e6:.2f} MB")


if __name__ == "__main__":
    main()
