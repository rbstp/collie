#!/usr/bin/env python3
import argparse
import json
from pathlib import Path
import tempfile

from package import digest, homebrew, identity, run


def gh(*args):
    return run("gh", *args)


def main(args):
    version = identity(args.tag, args.sha)
    remote = gh("api", f"repos/{{owner}}/{{repo}}/commits/{args.tag}", "--jq", ".sha")
    if remote != args.sha:
        raise ValueError("remote tag no longer matches source SHA")
    names = [f"collied-{version}-linux-x86_64.tar.gz", f"CollieTray-{version}-omarchy.tar.gz",
             f"collied-{version}-macos-arm64.tar.gz", f"collied-{version}-macos-arm64.dmg",
             f"CollieBar-{version}-macos-arm64.zip"]
    source = dict(tag=args.tag, source_sha=args.sha)
    source_claim = "<!-- collie-desktop-source:" + json.dumps(source, sort_keys=True) + " -->"
    marker = args.out / "source.json"
    marker.write_text(json.dumps(source, indent=2) + "\n")
    names.append(marker.name)
    pages = json.loads(gh("api", "repos/{owner}/{repo}/releases", "--paginate", "--slurp"))
    existing = [release for page in pages for release in page]
    release = next((item for item in existing if item["tag_name"] == args.tag), None)
    if release:
        assets = {asset["name"] for asset in release["assets"]}
        if source_claim not in (release.get("body") or "").splitlines() or release.get("target_commitish") != args.sha:
            raise ValueError("existing release has no matching source identity; refusing to adopt it")
        if "source.json" in assets:
            with tempfile.TemporaryDirectory() as temp:
                gh("release", "download", args.tag, "--pattern", "source.json", "--dir", temp)
                if json.loads((Path(temp) / "source.json").read_text()) != source:
                    raise ValueError("existing release belongs to another source SHA")
        elif release["draft"]:
            gh("release", "upload", args.tag, str(marker))
            assets.add(marker.name)
        else:
            raise ValueError("published release has no source identity asset; refusing to mutate it")
        if not release["draft"]:
            expected = set(names + [f"homebrew-{version}.tar.gz", "SHA256SUMS"])
            if not expected <= assets:
                raise ValueError("published release is incomplete; refusing to mutate it")
            with tempfile.TemporaryDirectory() as temp:
                gh("release", "download", args.tag, "--dir", temp)
                for line in (Path(temp) / "SHA256SUMS").read_text().splitlines():
                    checksum, name = line.split("  ", 1)
                    if name not in expected or name == "SHA256SUMS" or digest(Path(temp) / name) != checksum:
                        raise ValueError("published release checksum validation failed")
                covered = {line.split("  ", 1)[1] for line in (Path(temp) / "SHA256SUMS").read_text().splitlines()}
                if covered != expected - {"SHA256SUMS"}:
                    raise ValueError("published release checksum coverage is incomplete")
            print("Published release and checksums verified; no assets changed")
            return
    else:
        gh("release", "create", args.tag, "--verify-tag", "--target", args.sha, "--draft",
           "--title", args.tag, "--notes", "Desktop downloads: macOS 26+ Apple silicon and Linux x86_64 (glibc 2.39+). CollieTray requires Omarchy 4, Quickshell and qrencode. See the packaged README and release.json for installation, protocol and runtime requirements.\n\n" + source_claim)
        assets = set()
        gh("release", "upload", args.tag, str(marker))
        assets.add(marker.name)
    for name in names:
        if name not in assets:
            gh("release", "upload", args.tag, str(args.out / name))
    # Existing signed assets win on retry; signatures and notarization are not reproducible.
    for name in names:
        gh("release", "download", args.tag, "--pattern", name, "--dir", str(args.out), "--clobber")
    homebrew(args)
    tap_name = f"homebrew-{version}.tar.gz"
    if tap_name not in assets:
        gh("release", "upload", args.tag, str(args.out / tap_name))
    gh("release", "download", args.tag, "--pattern", tap_name, "--dir", str(args.out), "--clobber")
    checksum_names = sorted(names + [tap_name])
    (args.out / "SHA256SUMS").write_text("".join(f"{digest(args.out / name)}  {name}\n" for name in checksum_names))
    if "SHA256SUMS" in assets:
        with tempfile.TemporaryDirectory() as temp:
            gh("release", "download", args.tag, "--pattern", "SHA256SUMS", "--dir", temp)
            if (Path(temp) / "SHA256SUMS").read_bytes() != (args.out / "SHA256SUMS").read_bytes():
                raise ValueError("existing draft checksums differ; refusing replacement")
    else:
        gh("release", "upload", args.tag, str(args.out / "SHA256SUMS"))
    remote = gh("api", f"repos/{{owner}}/{{repo}}/commits/{args.tag}", "--jq", ".sha")
    if remote != args.sha:
        raise ValueError("tag changed during publishing; draft retained")
    gh("release", "edit", args.tag, "--draft=false")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", required=True)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--out", type=Path, required=True)
    main(parser.parse_args())
