import argparse
import json
import os
import shutil
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
import package
import publish


class PackageTests(unittest.TestCase):
    def test_widget_archive_is_complete_and_versioned(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            args = argparse.Namespace(tag="v1.2.3", sha="a" * 40, out=out, platform="omarchy")
            with patch.object(package, "identity", return_value="1.2.3"):
                package.package(args)
            with tarfile.open(out / "CollieTray-1.2.3-omarchy.tar.gz") as archive:
                prefix = "CollieTray-1.2.3-omarchy/"
                expected = {prefix + "CollieTray/" + path.name for path in (package.ROOT / "Linux/CollieTray").iterdir()}
                self.assertTrue(expected <= set(archive.getnames()))
                manifest = json.load(archive.extractfile(prefix + "CollieTray/manifest.json"))
                self.assertEqual(manifest["version"], "1.2.3")
                metadata = json.load(archive.extractfile(prefix + "release.json"))
                self.assertEqual(metadata["source_sha"], "a" * 40)
                self.assertEqual(archive.getmember(prefix + "install.sh").mode & 0o111, 0o111)

    def test_linux_runtime_floor_and_dependencies_are_checked(self):
        with patch.object(package, "run", side_effect=["GLIBC_2.17 GLIBC_2.39", "(NEEDED) Shared library: [libc.so.6]"]):
            result = package.metadata("1.2.3", "a" * 40, "linux-x86_64", Path("collied"))
        self.assertEqual(result["glibc_required"], "2.39")
        self.assertEqual(result["needed_libraries"], ["libc.so.6"])
        with patch.object(package, "run", return_value="GLIBC_2.40"):
            with self.assertRaisesRegex(ValueError, "above"):
                package.metadata("1.2.3", "a" * 40, "linux-x86_64", Path("collied"))

    def test_wrong_binary_version_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            args = argparse.Namespace(tag="v1.2.3", sha="a" * 40, out=Path(temp), platform="macos-arm64", binary=Path("collied"))
            with patch.object(package, "identity", return_value="1.2.3"), patch.object(package, "run", return_value="collied 0.1.0"):
                with self.assertRaisesRegex(ValueError, "version differs"):
                    package.package(args)

    def test_generated_tap_uses_final_asset_hashes_and_explicit_install(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            daemon = out / "collied-1.2.3-macos-arm64.tar.gz"
            app = out / "CollieBar-1.2.3-macos-arm64.zip"
            daemon.write_bytes(b"signed daemon")
            app.write_bytes(b"stapled app")
            args = argparse.Namespace(tag="v1.2.3", sha="a" * 40, out=out)
            with patch.object(package, "identity", return_value="1.2.3"):
                package.homebrew(args)
            with tarfile.open(out / "homebrew-1.2.3.tar.gz") as archive:
                formula = archive.extractfile("homebrew/Formula/collied.rb").read().decode()
                cask = archive.extractfile("homebrew/Casks/colliebar.rb").read().decode()
            self.assertIn(package.digest(daemon), formula)
            self.assertIn(package.digest(app), cask)
            self.assertIn("depends_on macos: :tahoe", formula)
            self.assertIn('depends_on macos: :tahoe', cask)
            self.assertIn("collied-install --replace", formula)
            self.assertNotIn("def post_install", formula)
            self.assertNotIn("service do", formula)
            for line in (out / "SHA256SUMS").read_text().splitlines():
                checksum, name = line.split("  ", 1)
                self.assertEqual(checksum, package.digest(out / name))

    @unittest.skipUnless(sys.platform == "darwin" and shutil.which("brew"), "requires macOS Homebrew")
    def test_generated_tap_loads_in_real_homebrew_dsl(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            (out / "collied-1.2.3-macos-arm64.tar.gz").write_bytes(b"daemon")
            (out / "CollieBar-1.2.3-macos-arm64.zip").write_bytes(b"app")
            args = argparse.Namespace(tag="v1.2.3", sha="a" * 40, out=out)
            with patch.object(package, "identity", return_value="1.2.3"):
                package.homebrew(args)
            env = dict(os.environ, HOMEBREW_NO_AUTO_UPDATE="1", HOMEBREW_NO_ANALYTICS="1")
            with tarfile.open(out / "homebrew-1.2.3.tar.gz") as archive:
                for name, ruby in [("Formula/collied.rb", 'require "formula"; puts Formulary.factory(ARGV.first).name'),
                                   ("Casks/colliebar.rb", 'require "cask/cask_loader"; puts Cask::CaskLoader.load(ARGV.first).token')]:
                    path = out / Path(name).name
                    path.write_bytes(archive.extractfile("homebrew/" + name).read())
                    result = subprocess.run(["brew", "ruby", "-e", ruby, str(path)], env=env, capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stdout.strip(), path.stem)


class InstallerTests(unittest.TestCase):
    def install(self, running, unit_text=None, replace=True):
        with tempfile.TemporaryDirectory(prefix="collie-install-") as temp:
            root = Path(temp)
            home = root / "home with spaces % $"
            home.mkdir()
            shim = root / "shim"
            shim.mkdir()
            package_dir = root / "package"
            package_dir.mkdir()
            (package_dir / "collied").write_text("#!/bin/bash\necho 'collied 1.2.3'\n")
            (package_dir / "collied").chmod(0o755)
            (package_dir / "install.sh").write_bytes((package.ROOT / "scripts/desktop/install.sh").read_bytes())
            unit = home / ".config/systemd/user/collied.service"
            unit.parent.mkdir(parents=True)
            original = unit_text or '[Service]\nExecStart="/old/Cellar/collied" "--config" "/my config/custom.toml" "run"\nEnvironment="XDG_DATA_HOME=/retained"\n'
            unit.write_text(original)
            dest = home / ".cargo/bin/collied"
            dest.parent.mkdir(parents=True)
            dest.write_text("old binary")
            (shim / "id").write_text("#!/bin/bash\necho 501\n")
            (shim / "uname").write_text('#!/bin/bash\nif [ "$1" = -s ]; then echo Linux; else echo x86_64; fi\n')
            (shim / "systemctl").write_text('''#!/bin/bash
printf '%s\\n' "$*" >> "$CALLS"
if [[ "$*" = *is-active* ]]; then exit "$ACTIVE"; fi
if [[ "$*" = *FragmentPath* ]]; then printf '%s\\n' "$UNIT"; fi
''')
            for path in shim.iterdir():
                path.chmod(0o755)
            calls = root / "calls"
            env = dict(os.environ, HOME=str(home), PATH=f"{shim}:{os.environ['PATH']}", CALLS=str(calls), UNIT=str(unit), ACTIVE="0" if running else "3")
            env.pop("XDG_CONFIG_HOME", None)
            result = subprocess.run(["bash", str(package_dir / "install.sh")] + (["--replace"] if replace else []), env=env, capture_output=True, text=True)
            log = calls.read_text() if calls.exists() else ""
            return result, dest.read_text(), unit.read_text(), original, log, str(dest)

    def test_updates_running_service_with_config_and_data_preserved(self):
        result, binary, unit, original, log, dest = self.install(True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("collied 1.2.3", binary)
        self.assertIn('"--config" "/my config/custom.toml" "run"', unit)
        self.assertIn('Environment="XDG_DATA_HOME=/retained"', unit)
        self.assertIn(dest.replace("%", "%%"), unit)
        self.assertIn("daemon-reload", log)
        self.assertIn("restart collied.service", log)

    def test_stopped_service_remains_stopped(self):
        result, _, _, _, log, _ = self.install(False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("restart collied.service", log)
        self.assertNotIn("enable collied.service", log)

    def test_refuses_unrecognized_unit_before_replacing_binary(self):
        result, binary, unit, original, _, _ = self.install(True, '[Service]\nExecStart=/custom/wrapper\n')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(binary, "old binary")
        self.assertEqual(unit, original)

    def test_requires_explicit_replacement(self):
        result, binary, _, _, _, _ = self.install(True, replace=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(binary, "old binary")


class PublisherTests(unittest.TestCase):
    def test_refuses_existing_release_with_another_owner(self):
        with tempfile.TemporaryDirectory() as temp:
            args = argparse.Namespace(tag="v1.2.3", sha="a" * 40, out=Path(temp))
            with patch.object(publish, "identity", return_value="1.2.3"), patch.object(publish, "gh", side_effect=["a" * 40, json.dumps([[dict(tag_name="v1.2.3", assets=[])]])]):
                with self.assertRaisesRegex(ValueError, "no matching source identity"):
                    publish.main(args)

    def test_draft_retry_retains_signed_bytes_and_completed_retry_is_read_only(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            sha = "a" * 40
            args = argparse.Namespace(tag="v1.2.3", sha=sha, out=out)
            names = ["collied-1.2.3-linux-x86_64.tar.gz", "CollieTray-1.2.3-omarchy.tar.gz",
                     "collied-1.2.3-macos-arm64.tar.gz", "collied-1.2.3-macos-arm64.dmg",
                     "CollieBar-1.2.3-macos-arm64.zip"]
            for name in names:
                (out / name).write_bytes(b"new build")
            retained = b"already notarized daemon"
            assets = {names[2]: retained, "source.json": json.dumps(dict(tag=args.tag, source_sha=sha)).encode()}
            state = dict(draft=True, writes=0)

            def fake_gh(*command):
                if command[0] == "api":
                    if "commits" in command[1]:
                        return sha
                    return json.dumps([[dict(tag_name=args.tag, target_commitish=sha, body="<!-- collie-desktop-source:" + json.dumps(dict(tag=args.tag, source_sha=sha), sort_keys=True) + " -->", draft=state["draft"], assets=[dict(name=name) for name in assets])]])
                if command[:2] == ("release", "download"):
                    dest = Path(command[command.index("--dir") + 1])
                    selected = [command[command.index("--pattern") + 1]] if "--pattern" in command else assets.keys()
                    for name in selected:
                        (dest / name).write_bytes(assets[name])
                elif command[:2] == ("release", "upload"):
                    path = Path(command[3])
                    self.assertNotIn(path.name, assets)
                    assets[path.name] = path.read_bytes()
                    state["writes"] += 1
                elif command[:2] == ("release", "edit"):
                    state["draft"] = False
                    state["writes"] += 1
                else:
                    self.fail(f"Unexpected GitHub operation: {command}")
                return ""

            with patch.object(publish, "identity", return_value="1.2.3"), patch.object(package, "identity", return_value="1.2.3"), patch.object(publish, "gh", side_effect=fake_gh):
                publish.main(args)
                self.assertEqual(assets[names[2]], retained)
                self.assertFalse(state["draft"])
                writes = state["writes"]
                publish.main(args)
                self.assertEqual(state["writes"], writes)
                assets[names[0]] = b"tampered after publication"
                with self.assertRaisesRegex(ValueError, "checksum validation failed"):
                    publish.main(args)

    def test_interrupted_creation_before_marker_upload_is_recoverable(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            sha = "a" * 40
            args = argparse.Namespace(tag="v1.2.3", sha=sha, out=out)
            names = ["collied-1.2.3-linux-x86_64.tar.gz", "CollieTray-1.2.3-omarchy.tar.gz",
                     "collied-1.2.3-macos-arm64.tar.gz", "collied-1.2.3-macos-arm64.dmg",
                     "CollieBar-1.2.3-macos-arm64.zip"]
            for name in names:
                (out / name).write_bytes(b"build")
            assets = {}
            state = dict(release=None, interrupt=True)

            def fake_gh(*command):
                if command[0] == "api":
                    if "commits" in command[1]:
                        return sha
                    return json.dumps([[state["release"]] if state["release"] else []])
                if command[:2] == ("release", "create"):
                    self.assertIsNone(state["release"])
                    state["release"] = dict(tag_name=args.tag, target_commitish=command[command.index("--target") + 1],
                                            body=command[command.index("--notes") + 1], draft=True, assets=[])
                elif command[:2] == ("release", "upload"):
                    path = Path(command[3])
                    if path.name == "source.json" and state["interrupt"]:
                        state["interrupt"] = False
                        raise RuntimeError("interrupted before source asset upload")
                    self.assertNotIn(path.name, assets)
                    assets[path.name] = path.read_bytes()
                    state["release"]["assets"].append(dict(name=path.name))
                elif command[:2] == ("release", "download"):
                    dest = Path(command[command.index("--dir") + 1])
                    selected = [command[command.index("--pattern") + 1]] if "--pattern" in command else assets.keys()
                    for name in selected:
                        (dest / name).write_bytes(assets[name])
                elif command[:2] == ("release", "edit"):
                    state["release"]["draft"] = False
                else:
                    self.fail(f"Unexpected GitHub operation: {command}")
                return ""

            with patch.object(publish, "identity", return_value="1.2.3"), patch.object(package, "identity", return_value="1.2.3"), patch.object(publish, "gh", side_effect=fake_gh):
                with self.assertRaisesRegex(RuntimeError, "interrupted"):
                    publish.main(args)
                self.assertTrue(state["release"]["draft"])
                self.assertEqual(assets, {})
                publish.main(args)
                self.assertFalse(state["release"]["draft"])
                self.assertEqual(json.loads(assets["source.json"]), dict(tag=args.tag, source_sha=sha))

    def test_marker_repair_rejects_foreign_claim_or_wrong_commit(self):
        sha = "a" * 40
        matching = "<!-- collie-desktop-source:" + json.dumps(dict(tag="v1.2.3", source_sha=sha), sort_keys=True) + " -->"
        for body, target in [("", sha), (matching.replace(sha, "b" * 40), sha), (matching, "b" * 40)]:
            with self.subTest(body=body, target=target), tempfile.TemporaryDirectory() as temp:
                args = argparse.Namespace(tag="v1.2.3", sha=sha, out=Path(temp))
                release = dict(tag_name=args.tag, body=body, target_commitish=target, draft=True, assets=[])
                with patch.object(publish, "identity", return_value="1.2.3"), patch.object(publish, "gh", side_effect=[sha, json.dumps([[release]])]) as github:
                    with self.assertRaisesRegex(ValueError, "no matching source identity"):
                        publish.main(args)
                    self.assertEqual(github.call_count, 2)

    def test_refuses_a_moved_remote_tag(self):
        with tempfile.TemporaryDirectory() as temp:
            args = argparse.Namespace(tag="v1.2.3", sha="a" * 40, out=Path(temp))
            with patch.object(publish, "identity", return_value="1.2.3"), patch.object(publish, "gh", return_value="b" * 40):
                with self.assertRaisesRegex(ValueError, "no longer matches"):
                    publish.main(args)


if __name__ == "__main__":
    unittest.main()
