#!/usr/bin/env python3
import argparse
import base64
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile

from package import digest


SOURCE = "rbstp/collie"
TAP = "rbstp/homebrew-tap"
FILES = ["Formula/collied.rb", "Casks/colliebar.rb"]
FIELDS = re.compile(r'^  (url|version|sha256) "([^"\n]+)"$', re.MULTILINE)


def api(repo, path, data=None, method=None):
    env = os.environ.copy()
    if repo == TAP:
        env["GH_TOKEN"] = os.environ["HOMEBREW_TAP_TOKEN"]
    command = ["gh", "api", f"repos/{repo}/{path}"]
    if method:
        command += ["--method", method]
    if data is not None:
        command += ["--input", "-"]
    result = subprocess.run(command, input=json.dumps(data) if data is not None else None,
                            env=env, capture_output=True, text=True, check=True)
    return json.loads(result.stdout)


def download(tag, name, destination):
    subprocess.run(["gh", "release", "download", tag, "-R", SOURCE, "--pattern", name,
                    "--dir", str(destination)], check=True)


def published(tag, source, names):
    release = api(SOURCE, "releases/tags/" + tag)
    claim = "<!-- collie-desktop-source:" + json.dumps({"source_sha": source, "tag": tag}, sort_keys=True) + " -->"
    if release["draft"] or release["prerelease"] or release["target_commitish"] != source or (release.get("body") or "").splitlines().count(claim) != 1:
        raise ValueError("published release source differs")
    assets = release["assets"]
    if len(assets) != 8 or {item["name"] for item in assets} != set(names):
        raise ValueError("published release asset inventory differs")
    if any(not re.fullmatch(r"sha256:[0-9a-f]{64}", item.get("digest") or "") or item["size"] <= 0 for item in assets):
        raise ValueError("published asset digest is missing")
    tag_ref = api(SOURCE, "git/ref/tags/" + tag)
    if tag_ref["object"]["type"] != "commit" or tag_ref["object"]["sha"] != source:
        raise ValueError("published tag source differs")
    return release


def release_identity(release):
    return (release["id"], release["target_commitish"], sorted(
        (item["id"], item["name"], item["size"], item["digest"]) for item in release["assets"]))


def fields(text):
    values = FIELDS.findall(text)
    if len(values) != 3 or {key for key, _ in values} != {"url", "version", "sha256"}:
        raise ValueError("tap definition metadata is ambiguous")
    return dict(values)


def update(tag, source):
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", tag) or not re.fullmatch(r"[0-9a-f]{40}", source):
        raise ValueError("exact release tag and source SHA required")
    version = tag[1:]
    if not os.environ.get("HOMEBREW_TAP_TOKEN"):
        raise ValueError("Set the homebrew-tap environment secret HOMEBREW_TAP_TOKEN with tap Contents read/write access")
    names = [f"collied-{version}-linux-x86_64.tar.gz", f"CollieTray-{version}-omarchy.tar.gz",
             f"collied-{version}-macos-arm64.tar.gz", f"collied-{version}-macos-arm64.dmg",
             f"CollieBar-{version}-macos-arm64.zip", "source.json", f"homebrew-{version}.tar.gz", "SHA256SUMS"]
    release = published(tag, source, names)
    assets = {item["name"]: item for item in release["assets"]}
    with tempfile.TemporaryDirectory() as temporary:
        temporary = Path(temporary)
        for name in ["SHA256SUMS", "source.json", f"homebrew-{version}.tar.gz"]:
            download(tag, name, temporary)
            path = temporary / name
            if path.stat().st_size != assets[name]["size"] or "sha256:" + digest(path) != assets[name]["digest"]:
                raise ValueError("downloaded release asset differs")
        sums = {}
        for line in (temporary / "SHA256SUMS").read_text().splitlines():
            checksum, name = line.split("  ", 1)
            if name not in assets or name == "SHA256SUMS" or name in sums or "sha256:" + checksum != assets[name]["digest"]:
                raise ValueError("published checksum differs")
            sums[name] = checksum
        if set(sums) != set(names) - {"SHA256SUMS"}:
            raise ValueError("published checksums are incomplete")
        if json.loads((temporary / "source.json").read_text()) != {"tag": tag, "source_sha": source}:
            raise ValueError("published source asset differs")
        contents = {}
        with tarfile.open(temporary / f"homebrew-{version}.tar.gz") as archive:
            for member in archive.getmembers():
                if member.isdir() and member.name in ["homebrew", "homebrew/Formula", "homebrew/Casks"]:
                    continue
                if not member.isfile() or member.name not in ["homebrew/" + path for path in FILES] or member.name in contents or not 0 < member.size < 65536:
                    raise ValueError("unexpected Homebrew archive member")
                contents[member.name] = archive.extractfile(member).read().decode()
        if set(contents) != {"homebrew/" + path for path in FILES}:
            raise ValueError("Homebrew definitions are missing")
    ref = api(TAP, "git/ref/heads/master")
    parent = ref["object"]["sha"]
    commit = api(TAP, "git/commits/" + parent)
    changes = []
    previous_versions = []
    for path, asset in zip(FILES, [f"collied-{version}-macos-arm64.tar.gz", f"CollieBar-{version}-macos-arm64.zip"]):
        item = api(TAP, "contents/" + path + "?ref=" + parent)
        if item["type"] != "file" or item["encoding"] != "base64":
            raise ValueError("tap definition is not a regular file")
        old = base64.b64decode(item["content"]).decode()
        new = contents["homebrew/" + path]
        old_fields, new_fields = fields(old), fields(new)
        if new_fields != {"url": f"https://github.com/{SOURCE}/releases/download/{tag}/{asset}", "version": version, "sha256": sums[asset]}:
            raise ValueError("generated tap definition differs from published assets")
        if FIELDS.sub(lambda match: match[1], old) != FIELDS.sub(lambda match: match[1], new):
            raise ValueError("tap behavior changed; update it in a reviewed pull request")
        previous = old_fields["version"]
        if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", previous):
            raise ValueError("tap version is invalid")
        previous_versions.append(previous)
        if tuple(map(int, previous.split("."))) > tuple(map(int, version.split("."))):
            raise ValueError("refusing a tap downgrade")
        if old != new:
            if previous == version:
                raise ValueError("same-version tap definition differs")
            changes.append({"path": path, "mode": "100644", "type": "blob", "content": new})
    if len(set(previous_versions)) != 1:
        raise ValueError("tap definition versions differ")
    if not changes:
        print("Homebrew tap already matches the published release")
        return
    if len(changes) != 2 or release_identity(published(tag, source, names)) != release_identity(release):
        raise ValueError("release or tap changed during verification")
    tree = api(TAP, "git/trees", {"base_tree": commit["tree"]["sha"], "tree": changes}, "POST")
    created = api(TAP, "git/commits", {"message": f"Update Collie to {version}", "tree": tree["sha"], "parents": [parent]}, "POST")
    api(TAP, "git/refs/heads/master", {"sha": created["sha"], "force": False}, "PATCH")
    print(f"Homebrew tap updated: https://github.com/{TAP}/commit/{created['sha']}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", required=True)
    parser.add_argument("--sha", required=True)
    args = parser.parse_args()
    update(args.tag, args.sha)
