#!/usr/bin/env python3
import argparse
import hashlib
import json
from pathlib import Path
import re
import tarfile

ROOT = Path(__file__).resolve().parents[2]


def update(downloads):
    source = json.loads((downloads / "source.json").read_text())
    tag, sha = source["tag"], source["source_sha"]
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", tag) or not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("invalid release identity")
    version = tag[1:]
    sums = dict(line.split(maxsplit=1)[::-1] for line in (downloads / "SHA256SUMS").read_text().splitlines())
    packages = [
        ("collied-bin", f"collied-{version}-linux-x86_64", "collied-aur-adopt", "linux-x86_64"),
        ("collie-tray", f"CollieTray-{version}-omarchy", "collie-tray-activate", "omarchy"),
    ]
    updates = []
    protocol = None
    for name, stem, helper, platform in packages:
        asset = downloads / f"{stem}.tar.gz"
        digest = hashlib.sha256(asset.read_bytes()).hexdigest()
        if sums.get(asset.name) != digest:
            raise ValueError(f"checksum mismatch: {asset.name}")
        with tarfile.open(asset) as archive:
            metadata = json.load(archive.extractfile(f"{stem}/release.json"))
            if (metadata["version"], metadata["source_sha"], metadata["platform"]) != (version, sha, platform):
                raise ValueError(f"release identity mismatch: {asset.name}")
            if platform == "linux-x86_64" and tuple(map(int, metadata["glibc_required"].split("."))) > (2, 39):
                raise ValueError("review the package glibc dependency before updating")
            current = (metadata["protocol_min"], metadata["protocol_max"])
            if protocol is not None and protocol != current:
                raise ValueError("daemon and widget protocol ranges differ")
            protocol = current
            if name == "collie-tray":
                manifest = json.load(archive.extractfile(f"{stem}/CollieTray/manifest.json"))
                if manifest["version"] != version or manifest["id"] != "rbstp.collie":
                    raise ValueError("widget manifest differs from release")
        recipe = ROOT / "packaging/aur" / name / "PKGBUILD"
        text = re.sub(r"^pkgver=.*$", f"pkgver={version}", recipe.read_text(), flags=re.M)
        text = re.sub(r"^pkgrel=.*$", "pkgrel=1", text, flags=re.M)
        helper_hash = hashlib.sha256((recipe.parent / helper).read_bytes()).hexdigest()
        text = re.sub(r"sha256sums=\([^)]*\)", f"sha256sums=('{digest}'\n            '{helper_hash}')", text)
        updates.append((recipe, text))
    for recipe, text in updates:
        recipe.write_text(text)
    print(f"Updated AUR recipes for {tag} at {sha}; regenerate .SRCINFO with makepkg --printsrcinfo.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("downloads", type=Path, help="verified published release downloads")
    update(parser.parse_args().downloads)
