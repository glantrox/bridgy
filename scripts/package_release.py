#!/usr/bin/env python3
"""Build release archives/checksums; requires only Python's standard library."""
import argparse
import hashlib
import shutil
import tarfile
import zipfile
from pathlib import Path

UNIX_TARGETS = (
    "aarch64-apple-darwin", "x86_64-apple-darwin",
    "aarch64-unknown-linux-musl", "x86_64-unknown-linux-musl",
)
WINDOWS_TARGET = "x86_64-pc-windows-msvc"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def package(binary, target, output):
    output.mkdir(parents=True, exist_ok=True)
    stem = f"bridgy-{target}"
    if target == WINDOWS_TARGET:
        archive = output / f"{stem}.zip"
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as bundle:
            bundle.write(binary, "bridgy.exe")
        files = [archive]
    else:
        raw = output / stem
        shutil.copyfile(binary, raw)
        raw.chmod(0o755)
        archive = output / f"{stem}.tar.gz"
        with tarfile.open(archive, "w:gz") as bundle:
            info = bundle.gettarinfo(str(raw), arcname="bridgy")
            info.mode = 0o755
            with raw.open("rb") as stream:
                bundle.addfile(info, stream)
        files = [raw, archive]
    for path in files:
        path.with_name(path.name + ".sha256").write_text(f"{digest(path)}  {path.name}\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--target", choices=(*UNIX_TARGETS, WINDOWS_TARGET), required=True)
    parser.add_argument("--output", type=Path, default=Path("dist"))
    args = parser.parse_args()
    package(args.binary, args.target, args.output)
