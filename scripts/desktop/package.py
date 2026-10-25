#!/usr/bin/env python3
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def identity(tag, sha):
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", tag):
        raise ValueError("tag must be vMAJOR.MINOR.PATCH")
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("source SHA must be a full commit SHA")
    if run("git", "rev-parse", "HEAD") != sha:
        raise ValueError("checkout differs from source SHA")
    if run("git", "status", "--porcelain", "--untracked-files=no"):
        raise ValueError("release checkout has tracked changes")
    return tag[1:]


def archive(source, dest):
    with tarfile.open(dest, "w:gz") as output:
        output.add(source, arcname=source.name)


def metadata(version, sha, platform, binary=None):
    protocol = re.search(r"pub const PROTOCOL_VERSION: u32 = (\d+);", (ROOT / "crates/protocol/src/lib.rs").read_text())
    if not protocol:
        raise ValueError("cannot determine protocol version")
    floor = re.search(r"pub const PROTOCOL_MIN_VERSION: u32 = (\d+);",
                      (ROOT / "crates/protocol/src/compatibility.rs").read_text())
    if not floor:
        raise ValueError("cannot determine protocol floor")
    result = dict(version=version, source_sha=sha, platform=platform,
                  protocol_min=int(floor[1]), protocol_max=int(protocol[1]))
    if platform == "linux-x86_64":
        symbols = run("readelf", "--version-info", str(binary))
        versions = re.findall(r"\bGLIBC_(\d+\.\d+)(?:\.\d+)?\b", symbols)
        floor = max(versions, key=lambda value: tuple(map(int, value.split("."))))
        if tuple(map(int, floor.split("."))) > (2, 39):
            raise ValueError(f"binary requires glibc {floor}, above the 2.39 runtime floor")
        dynamic = run("readelf", "-d", str(binary))
        result.update(glibc_required=floor, glibc_supported_min="2.39",
                      needed_libraries=re.findall(r"\(NEEDED\).*\[(.*?)\]", dynamic),
                      runtime="Linux x86_64, glibc 2.39+, systemd 255+, ca-certificates, herdr")
    elif platform == "macos-arm64":
        result["runtime"] = "macOS 26+, Apple silicon, herdr"
    else:
        result["runtime"] = "Omarchy 4, Quickshell, qrencode, version-matched collied"
    return result


def package(args):
    version = identity(args.tag, args.sha)
    args.out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as temp:
        stage = Path(temp)
        if args.platform == "omarchy":
            package_dir = stage / f"CollieTray-{version}-omarchy"
            package_dir.mkdir()
            shutil.copytree(ROOT / "Linux/CollieTray", package_dir / "CollieTray")
            manifest = package_dir / "CollieTray/manifest.json"
            content = json.loads(manifest.read_text())
            content["version"] = version
            manifest.write_text(json.dumps(content, indent=2) + "\n")
            shutil.copy2(ROOT / "scripts/desktop/install-tray.sh", package_dir / "install.sh")
            meta = metadata(version, args.sha, "omarchy")
        else:
            package_dir = stage / f"collied-{version}-{args.platform}"
            package_dir.mkdir()
            if run(str(args.binary.resolve()), "--version") != f"collied {version}":
                raise ValueError("daemon version differs from release tag")
            shutil.copy2(args.binary, package_dir / "collied")
            shutil.copy2(ROOT / "scripts/desktop/install.sh", package_dir / "install.sh")
            subprocess.check_call([sys.executable, str(ROOT / "scripts/notices.py"), "collied",
                                   str(package_dir / "THIRD_PARTY_NOTICES")])
            meta = metadata(version, args.sha, args.platform, args.binary)
        shutil.copy2(ROOT / "LICENSE", package_dir / "LICENSE")
        shutil.copy2(ROOT / "docs/desktop-releases.md", package_dir / "README.md")
        (package_dir / "release.json").write_text(json.dumps(meta, indent=2) + "\n")
        archive(package_dir, args.out / f"{package_dir.name}.tar.gz")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def homebrew(args):
    version = identity(args.tag, args.sha)
    daemon = args.out / f"collied-{version}-macos-arm64.tar.gz"
    app = args.out / f"CollieBar-{version}-macos-arm64.zip"
    base = f"https://github.com/rbstp/collie/releases/download/{args.tag}"
    with tempfile.TemporaryDirectory() as temp:
        tap = Path(temp) / "homebrew"
        (tap / "Formula").mkdir(parents=True)
        (tap / "Casks").mkdir()
        (tap / "Formula/collied.rb").write_text(f'''class Collied < Formula
  desc "Control local coding agents from an iPhone"
  homepage "https://collie.rbstp.dev"
  url "{base}/{daemon.name}"
  version "{version}"
  sha256 "{digest(daemon)}"
  license "Apache-2.0"

  depends_on arch: :arm64
  depends_on macos: :tahoe

  def install
    bin.install "collied"
    (share/"collied").install "install.sh"
    (bin/"collied-install").write <<~SH
      #!/bin/bash
      exec /bin/bash "#{{share}}/collied/install.sh" "$@"
    SH
    chmod 0755, bin/"collied-install"
    doc.install "release.json", "README.md", "THIRD_PARTY_NOTICES"
  end

  def caveats
    <<~EOS
      To install or refresh the daemon service executable explicitly, run:
        collied-install --replace
      This preserves user data and the existing service running state.
      After first install, add ~/.cargo/bin to PATH and run collied setup.
      Do not use brew services. Uninstall the service before brew uninstall:
        ~/.cargo/bin/collied service uninstall
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{{bin}}/collied --version")
  end
end
''')
        (tap / "Casks/colliebar.rb").write_text(f'''cask "colliebar" do
  version "{version}"
  sha256 "{digest(app)}"

  url "{base}/{app.name}"
  name "CollieBar"
  desc "Menu bar companion for collied"
  homepage "https://collie.rbstp.dev/"

  depends_on arch: :arm64
  depends_on macos: :tahoe

  app "CollieBar.app"

  uninstall quit: "dev.rbstp.colliebar"
end
''')
        archive(tap, args.out / f"homebrew-{version}.tar.gz")
    assets = sorted(args.out.iterdir())
    (args.out / "SHA256SUMS").write_text("".join(f"{digest(asset)}  {asset.name}\n" for asset in assets if asset.name != "SHA256SUMS"))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=["package", "homebrew"])
    parser.add_argument("--tag", required=True)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--platform", choices=["macos-arm64", "linux-x86_64", "omarchy"])
    parser.add_argument("--binary", type=Path)
    args = parser.parse_args()
    if args.command == "package":
        if not args.platform or (args.platform != "omarchy" and not args.binary):
            parser.error("package requires --platform and a daemon --binary")
        package(args)
    else:
        homebrew(args)
