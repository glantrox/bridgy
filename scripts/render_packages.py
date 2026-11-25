#!/usr/bin/env python3
"""Render package definitions using the actual release repository and archive hashes."""
import argparse
import re
from pathlib import Path

from package_release import UNIX_TARGETS, WINDOWS_TARGET, digest

ROOT = Path(__file__).resolve().parents[1]


def render(repository, tag, artifacts, output, package_id=None):
    if not re.fullmatch(r"[A-Za-z0-9_-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("repository must be owner/repository")
    if not re.fullmatch(r"v\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?", tag):
        raise ValueError("tag must be vMAJOR.MINOR.PATCH (optional prerelease suffix)")
    publisher = repository.split("/")[0]
    package_id = package_id or f"{publisher}.Bridgy"
    if not re.fullmatch(r"[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+", package_id):
        raise ValueError("invalid WinGet package identifier")
    values = {
        "REPOSITORY": repository, "VERSION": tag[1:],
        "BASE_URL": f"https://github.com/{repository}/releases/download/{tag}",
        "PACKAGE_ID": package_id, "PUBLISHER": publisher,
    }
    # Compute hashes from the artifacts, never from placeholder or supplied checksums.
    for target in (*UNIX_TARGETS, WINDOWS_TARGET):
        extension = "zip" if target == WINDOWS_TARGET else "tar.gz"
        values[f"SHA_{target}"] = digest(artifacts / f"bridgy-{target}.{extension}")
    outputs = [(ROOT / "packaging/homebrew/bridgy.rb.in", output / "bridgy.rb")]
    for template in sorted((ROOT / "packaging/winget").glob("*.in")):
        name = template.name.removesuffix(".in").replace("bridgy", package_id, 1)
        outputs.append((template, output / "winget" / name))
    for template, destination in outputs:
        text = template.read_text()
        text = re.sub(r"@([A-Za-z0-9_-]+)@", lambda match: values[match[1]], text)
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(text)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--artifacts", type=Path, default=Path("dist"))
    parser.add_argument("--output", type=Path, default=Path("dist/packages"))
    parser.add_argument("--package-id")
    args = parser.parse_args()
    render(args.repository, args.tag, args.artifacts, args.output, args.package_id)
