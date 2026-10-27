#!/usr/bin/env python3
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]
TOOLS = ROOT / "target/tools"
LIBTAILSCALE = ROOT / "crates/tailscale-sys/libtailscale"
GHOSTTY_LICENSES = ROOT / "Collie/GhosttyTerminal/GhosttyVt.xcframework/licenses"

CARGO_ABOUT = "0.9.2"
CARGO_ABOUT_SHA256 = {
    "aarch64-apple-darwin": "ae72f0df0c399a1e96336f696fa55b1b28679fd725632eba8cf8e4568467cc3e",
    "x86_64-unknown-linux-musl": "9099a59e820c38a68b9d65f300662a567d56562f9a10f6aa4c7e86c17c2566af",
}
GO_LICENSES = "v2.0.1"

# Written by scripts/ghostty/build-xcframework.sh from the pinned sources.
GHOSTTY = {
    "libghostty-vt": "MIT",
    "simdutf": "MIT AND BSD-3-Clause",
    "Highway": "BSD-3-Clause",
    "uucode": "MIT AND Unicode-3.0",
    "Zig compiler-rt": "MIT",
}
FONTS = "Collie/GhosttyTerminal/Sources/GhosttyTerminal/Fonts"
IOS_FILES = [
    ("JetBrains Mono", "OFL-1.1", f"{FONTS}/JetBrainsMono-LICENSE.txt"),
    ("Symbols Nerd Font", "MIT AND OFL-1.1 AND Apache-2.0 AND CC-BY-4.0 AND Unlicense",
     f"{FONTS}/NerdFonts-LICENSE.txt"),
    ("Octicons", "MIT", "Collie/Resources/Octicons-LICENSE.txt"),
]
COLLIED_FILES = [
    ("LiteLLM model prices", "MIT", "crates/collied/data/prices-LICENSE.txt"),
]


def accepted():
    match = re.search(r"^accepted\s*=\s*\[(.*?)\]", (ROOT / "about.toml").read_text(), re.S | re.M)
    return set(re.findall(r'"([^"]+)"', match[1]))


def component(name, version, license, text):
    return {"name": name, "version": version, "license": license, "text": text.strip() + "\n"}


def cargo_about():
    host = {("Darwin", "arm64"): "aarch64-apple-darwin", ("Linux", "x86_64"): "x86_64-unknown-linux-musl"}.get(
        (platform.system(), platform.machine()))
    if not host:
        sys.exit(f"no pinned cargo-about for {platform.system()} {platform.machine()}")
    name = f"cargo-about-{CARGO_ABOUT}-{host}"
    binary = TOOLS / name / "cargo-about"
    if not binary.exists():
        url = f"https://github.com/EmbarkStudios/cargo-about/releases/download/{CARGO_ABOUT}/{name}.tar.gz"
        data = subprocess.check_output(["curl", "-fsSL", "--retry", "3", url])
        if hashlib.sha256(data).hexdigest() != CARGO_ABOUT_SHA256[host]:
            sys.exit(f"{url} does not match its pinned SHA-256")
        TOOLS.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryFile() as file:
            file.write(data)
            file.seek(0)
            with tarfile.open(fileobj=file) as archive:
                archive.extract(f"{name}/cargo-about", TOOLS)
    return binary


def rust(manifest, targets):
    targets = [arg for target in targets for arg in ("--target", target)]
    report = json.loads(subprocess.check_output(
        [str(cargo_about()), "generate", "--fail", "--locked", "--format", "json",
         "-c", str(ROOT / "about.toml"), "-m", str(ROOT / manifest), *targets], cwd=ROOT))
    texts = {}
    for license in report["licenses"]:
        for use in license["used_by"]:
            key = (use["crate"]["name"], use["crate"]["version"])
            entries = texts.setdefault(key, [])
            if (license["id"], license["text"]) not in entries:
                entries.append((license["id"], license["text"]))
    result = []
    for crate in report["crates"]:
        key = (crate["package"]["name"], crate["package"]["version"])
        if key not in texts:
            sys.exit(f"cargo-about found no license text for {key[0]} {key[1]}")
        ids = list(dict.fromkeys(id for id, _ in texts[key]))
        result.append(component(*key, " AND ".join(ids), "\n\n".join(text.strip() for _, text in texts[key])))
    return sorted(result, key=lambda c: (c["name"].lower(), c["version"]))


def rust_std():
    version = subprocess.check_output(["rustc", "--version"], text=True).split()[1]
    licenses = Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip()) / "share/doc/rust/licenses"
    mit = (licenses / "MIT.txt").read_text().replace("<year> <copyright holders>", "The Rust Project Developers")
    text = ("Copyright The Rust Project Developers (see https://thanks.rust-lang.org).\n"
            "The Rust standard library is dual-licensed under the MIT license and the Apache License 2.0.\n\n"
            + mit.strip() + "\n\n" + (licenses / "Apache-2.0.txt").read_text())
    return component("Rust standard library", version, "MIT OR Apache-2.0", text), licenses


def go(platforms, tags=""):
    go = os.environ.get("GO", "go")
    env = dict(os.environ, GOTOOLCHAIN="local")
    tool = TOOLS / f"go-licenses-{GO_LICENSES}" / "go-licenses"
    if not tool.exists():
        # go install verifies the module against the Go checksum database.
        subprocess.check_call([go, "install", f"github.com/google/go-licenses/v2@{GO_LICENSES}"],
                              env=dict(env, GOBIN=str(tool.parent), GOFLAGS=""))
    allowed = accepted()
    libraries = {}
    with tempfile.NamedTemporaryFile("w", suffix=".tpl") as template:
        template.write("{{range .}}{{.Name}}\t{{.Version}}\t{{.LicenseName}}\t{{.LicensePath}}\n{{end}}")
        template.flush()
        for goos, goarch in platforms:
            flags = f"-tags={tags}" if tags else ""
            report = subprocess.run(
                [str(tool), "report", "--template", template.name, "."], cwd=LIBTAILSCALE, text=True,
                capture_output=True, env=dict(env, GOOS=goos, GOARCH=goarch, CGO_ENABLED="1", GOFLAGS=flags))
            if report.returncode:
                sys.exit(report.stderr)
            for line in report.stdout.splitlines():
                name, version, license, path = line.split("\t")
                if license not in allowed:
                    sys.exit(f"{name} {version}: license {license} is not accepted in about.toml")
                entry = libraries.setdefault(path, {"name": name, "version": version, "licenses": []})
                if license not in entry["licenses"]:
                    entry["licenses"].append(license)
    modules = []
    first = None
    for path, entry in libraries.items():
        version = None if entry["version"] in ("", "Unknown") else entry["version"]
        item = component(entry["name"], version, " AND ".join(entry["licenses"]), Path(path).read_text())
        if entry["name"] == "github.com/tailscale/libtailscale":
            item["name"] = "libtailscale"
            first = item
        else:
            modules.append(item)
    if not first:
        sys.exit("go-licenses did not report libtailscale")
    goroot = subprocess.check_output([go, "env", "GOROOT"], env=env, text=True).strip()
    version = subprocess.check_output([go, "env", "GOVERSION"], env=env, text=True).strip()
    # Homebrew keeps the license beside GOROOT.
    license = next(path for path in (Path(goroot) / "LICENSE", Path(goroot).parent / "LICENSE") if path.exists())
    runtime = component("Go", version, "BSD-3-Clause", license.read_text())
    return [first, runtime] + sorted(modules, key=lambda c: c["name"].lower())


def ghostty():
    found = sorted(path.name for path in GHOSTTY_LICENSES.iterdir()) if GHOSTTY_LICENSES.is_dir() else []
    if sorted(GHOSTTY) != found:
        sys.exit(f"{GHOSTTY_LICENSES} lists {found}, expected {sorted(GHOSTTY)}: run just ios-ghostty")
    return [component(name, None, license, (GHOSTTY_LICENSES / name).read_text()) for name, license in GHOSTTY.items()]


def main():
    if len(sys.argv) != 3 or sys.argv[1] not in ("ios", "collied"):
        sys.exit("usage: notices.py ios|collied <output>")
    kind, out = sys.argv[1], Path(sys.argv[2])
    std, licenses = rust_std()
    if kind == "ios":
        items = go([("ios", "arm64")], "ios,ts_omit_portmapper") + ghostty()
        items += [component(name, None, license, (ROOT / path).read_text()) for name, license, path in IOS_FILES]
        # Material Design Icons' license only links Apache-2.0; section 4(a) asks for the full text.
        next(c for c in items if c["name"] == "Symbols Nerd Font")["text"] += "\n" + (licenses / "Apache-2.0.txt").read_text()
        items += [std] + rust("crates/collie-core/Cargo.toml", ["aarch64-apple-ios"])
        content = json.dumps(items, indent=1, ensure_ascii=False) + "\n"
    else:
        items = go([("darwin", "arm64"), ("linux", "amd64")])
        items += [component(name, None, license, (ROOT / path).read_text()) for name, license, path in COLLIED_FILES]
        items += [std] + rust("crates/collied/Cargo.toml", ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"])
        rule = "-" * 80
        content = "Third-party software in collied\n\n" + "".join(
            f"{rule}\n{c['name']}{' ' + c['version'] if c['version'] else ''}\nLicense: {c['license']}\n\n{c['text']}\n"
            for c in items)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(content)
    print(f"{out}: {len(items)} components")


if __name__ == "__main__":
    main()
