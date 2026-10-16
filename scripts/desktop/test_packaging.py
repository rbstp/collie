import argparse
import json
import os
import plistlib
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
            repository = Path(subprocess.check_output(["brew", "--repository"], text=True).strip())
            prefix = out.resolve() / "brew"
            (prefix / "bin").mkdir(parents=True)
            (prefix / "Library").mkdir()
            shutil.copy2(repository / "bin/brew", prefix / "bin/brew")
            (prefix / "Library/Homebrew").symlink_to(repository / "Library/Homebrew", target_is_directory=True)
            tap = prefix / "Library/Taps/collie-tests/homebrew-fixture"
            (tap / "Formula").mkdir(parents=True)
            (tap / "Casks").mkdir()
            for path in [prefix, tap]:
                subprocess.run(["git", "init", "--quiet", str(path)], check=True)
            env = dict(os.environ, HOMEBREW_NO_AUTO_UPDATE="1", HOMEBREW_NO_ANALYTICS="1",
                       HOMEBREW_FORBID_PACKAGES_FROM_PATHS="1", HOMEBREW_REQUIRE_TAP_TRUST="1",
                       XDG_CONFIG_HOME=str(out / "config"), HOMEBREW_CACHE=str(out / "cache"),
                       HOMEBREW_LOGS=str(out / "logs"), HOMEBREW_TEMP=str(out / "tmp"))
            for name in ["HOMEBREW_INTERNAL_ALLOW_PACKAGES_FROM_PATHS", "HOMEBREW_NO_REQUIRE_TAP_TRUST"]:
                env.pop(name, None)
            brew = str(prefix / "bin/brew")
            with tarfile.open(out / "homebrew-1.2.3.tar.gz") as archive:
                for name in ["Formula/collied.rb", "Casks/colliebar.rb"]:
                    (tap / name).write_bytes(archive.extractfile("homebrew/" + name).read())
            loose = out.resolve() / "collied.rb"
            shutil.copy2(tap / "Formula/collied.rb", loose)
            result = subprocess.run([brew, "ruby", "-e", 'require "formula"; Formulary.factory(ARGV.first)', str(loose)],
                                    env=env, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Homebrew requires formulae to be in a tap", result.stderr)
            result = subprocess.run([brew, "trust", "--tap", "collie-tests/fixture"], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            for name, ruby in [("Formula/collied.rb", 'require "formula"; puts Formulary.factory(ARGV.first).full_name'),
                               ("Casks/colliebar.rb", 'require "cask/cask_loader"; puts Cask::CaskLoader.load(ARGV.first).full_name')]:
                path = tap / name
                result = subprocess.run([brew, "ruby", "-e", ruby, str(path)], env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.strip(), "collie-tests/fixture/" + path.stem)


class SourceValidationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="collie-source-")
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        self.git("init", "--quiet", "--initial-branch=master")
        (self.repo / "fixture").write_text("first master commit")
        self.git("add", "fixture")
        self.git("commit", "--quiet", "-m", "Initial fixture")
        self.previous = self.git("rev-parse", "HEAD")
        (self.repo / "fixture").write_text("current master commit")
        validator = self.repo / "scripts/desktop/validate-source.sh"
        validator.parent.mkdir(parents=True)
        shutil.copy2(package.ROOT / "scripts/desktop/validate-source.sh", validator)
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "Update fixture")
        self.master = self.git("rev-parse", "HEAD")
        self.git("update-ref", "refs/remotes/origin/master", self.master)
        self.output = self.repo / "outputs"

    def git(self, *args):
        return subprocess.check_output(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", *args], cwd=self.repo, text=True).strip()

    def validate(self, source, tag="v1.2.3", checkout=False):
        env = dict(os.environ, SOURCE_SHA=source, TAG=tag, GITHUB_OUTPUT=str(self.output))
        script = package.ROOT / "scripts/desktop/validate-source.sh"
        if checkout:
            script = self.repo.parent / (self.repo.name + "-trusted-validator.sh")
            self.addCleanup(script.unlink, missing_ok=True)
            shutil.copy2(self.repo / "scripts/desktop/validate-source.sh", script)
        return subprocess.run(["bash", str(script)] + (["--checkout"] if checkout else []), cwd=self.repo, env=env, capture_output=True, text=True)

    def test_master_ancestor_produces_canonical_identity_without_changing_checkout(self):
        result = self.validate(self.previous)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.output.read_text(), f"source_sha={self.previous}\ntag=v1.2.3\n")
        self.assertEqual(self.git("rev-parse", "HEAD"), self.master)

    def test_existing_annotated_tag_at_same_commit_allows_retry(self):
        self.git("tag", "-a", "v1.2.3", self.previous, "-m", "Fixture release")
        result = self.validate(self.previous)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(self.previous, self.output.read_text())

    def test_off_master_commit_cannot_produce_trusted_output(self):
        self.git("checkout", "--quiet", "-b", "untrusted", self.previous)
        (self.repo / "fixture").write_text("untrusted commit")
        self.git("commit", "--quiet", "-a", "-m", "Untrusted fixture")
        untrusted = self.git("rev-parse", "HEAD")
        self.git("checkout", "--quiet", "master")
        self.git("tag", "v1.2.3", untrusted)
        result = self.validate(untrusted, checkout=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.output.exists())
        self.assertEqual(self.git("rev-parse", "HEAD"), self.master)

    def test_tag_cannot_be_reused_for_another_commit(self):
        self.git("tag", "v1.2.3", self.previous)
        result = self.validate(self.master, checkout=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.output.exists())
        self.assertEqual(self.git("rev-parse", "HEAD"), self.master)

    def test_checkout_uses_trusted_validator_when_selected_commit_lacks_it(self):
        self.git("tag", "v1.2.3", self.previous)
        result = self.validate(self.previous, checkout=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.git("rev-parse", "HEAD"), self.previous)
        self.assertFalse((self.repo / "scripts/desktop/validate-source.sh").exists())
        self.assertEqual(self.output.read_text(), f"source_sha={self.previous}\ntag=v1.2.3\n")

    def test_checkout_requires_tag_before_detaching_from_master(self):
        result = self.validate(self.previous, checkout=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.git("rev-parse", "HEAD"), self.master)
        self.assertFalse(self.output.exists())

    def test_dispatch_input_is_data_and_invalid_values_are_rejected(self):
        injected = self.repo / "injected"
        for source, tag in [(f"$(touch {injected})", "v1.2.3"), (self.master, f"v1.2.3$(touch {injected})")]:
            with self.subTest(source=source, tag=tag):
                result = self.validate(source, tag)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(injected.exists())
                self.assertFalse(self.output.exists())

    def test_validator_refuses_to_run_from_a_different_checkout(self):
        self.git("checkout", "--quiet", "--detach", self.previous)
        result = self.validate(self.previous)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.output.exists())


class InstallerTests(unittest.TestCase):
    def install(self, running, unit_text=None, replace=True, macos=False, notarized=True, metadata_id="dev.rbstp.collied", metadata_team="RM3UT3MMSR", metadata_exit=0, verbose_metadata=False):
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
            if macos and running:
                unit = home / "Library/LaunchAgents/dev.rbstp.collied.plist"
                unit.parent.mkdir(parents=True)
                original = plistlib.dumps({"Label": "dev.rbstp.collied", "ProgramArguments": ["/old/collied", "--config", "/my config/custom.toml", "run"]}).decode()
                unit.write_text(original)
            dest = home / ".cargo/bin/collied"
            dest.parent.mkdir(parents=True)
            dest.write_text("old binary")
            (shim / "id").write_text("#!/bin/bash\necho 501\n")
            system, arch = ("Darwin", "arm64") if macos else ("Linux", "x86_64")
            (shim / "uname").write_text(f'#!/bin/bash\nif [ "$1" = -s ]; then echo {system}; else echo {arch}; fi\n')
            (shim / "codesign").write_text('''#!/bin/bash
set -e
printf 'codesign %s\\n' "$*" >> "$CALLS"
if [ "$1" = -dv ]; then
    echo "Identifier=$METADATA_ID" >&2
    echo "TeamIdentifier=$METADATA_TEAM" >&2
    if [ "$VERBOSE_METADATA" = 1 ]; then
        head -c 1048576 /dev/zero | tr '\\0' x >&2
        echo >&2
    fi
    exit "$METADATA_EXIT"
fi
if [[ "$*" = *--check-notarization* ]]; then exit "$NOTARIZATION"; fi
''')
            (shim / "spctl").write_text('#!/bin/bash\nexit 99\n')
            (shim / "launchctl").write_text('#!/bin/bash\nprintf "launchctl %s\\n" "$*" >> "$CALLS"\nexit 1\n')
            (shim / "systemctl").write_text('''#!/bin/bash
printf '%s\\n' "$*" >> "$CALLS"
if [[ "$*" = *is-active* ]]; then exit "$ACTIVE"; fi
if [[ "$*" = *FragmentPath* ]]; then printf '%s\\n' "$UNIT"; fi
''')
            for path in shim.iterdir():
                path.chmod(0o755)
            calls = root / "calls"
            env = dict(os.environ, HOME=str(home), PATH=f"{shim}:{os.environ['PATH']}", CALLS=str(calls), UNIT=str(unit), ACTIVE="0" if running else "3", NOTARIZATION="0" if notarized else "3", METADATA_ID=metadata_id, METADATA_TEAM=metadata_team, METADATA_EXIT=str(metadata_exit), VERBOSE_METADATA="1" if verbose_metadata else "0")
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

    def test_macos_uses_standalone_notarization_without_an_app_assessment(self):
        result, binary, _, _, _, _ = self.install(False, macos=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("collied 1.2.3", binary)

    def test_macos_consumes_verbose_signing_metadata_before_asserting_identity(self):
        result, binary, _, _, log, _ = self.install(False, macos=True, verbose_metadata=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("collied 1.2.3", binary)
        self.assertEqual(sum(line.startswith("codesign -dv ") for line in log.splitlines()), 1)

    def test_macos_metadata_failure_preserves_existing_binary_and_service(self):
        result, binary, unit, original, log, _ = self.install(True, macos=True, metadata_exit=7)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(binary, "old binary")
        self.assertEqual(unit, original)
        self.assertNotIn("launchctl", log)
        self.assertNotIn("--check-notarization", log)

    def test_macos_requires_literal_signing_identifier_and_team(self):
        for identity in ({"metadata_id": "other.collied"}, {"metadata_id": "devXrbstpXcollied"}, {"metadata_team": "OTHERTEAM"}):
            with self.subTest(identity=identity):
                result, binary, unit, original, log, _ = self.install(True, macos=True, **identity)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(binary, "old binary")
                self.assertEqual(unit, original)
                self.assertNotIn("launchctl", log)
                self.assertNotIn("--check-notarization", log)

    def test_macos_notarization_failure_preserves_existing_binary_and_service(self):
        result, binary, unit, original, log, _ = self.install(True, macos=True, notarized=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(binary, "old binary")
        self.assertEqual(unit, original)
        self.assertNotIn("launchctl", log)

    @unittest.skipUnless(sys.platform == "darwin" and shutil.which("clang"), "requires macOS signing tools")
    def test_valid_adhoc_cli_fails_the_notarized_requirement(self):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "scratch.c"
            binary = Path(temp) / "scratch"
            source.write_text("int main(void) { return 0; }\n")
            subprocess.run(["clang", str(source), "-o", str(binary)], check=True, capture_output=True)
            subprocess.run(["codesign", "--force", "--sign", "-", str(binary)], check=True, capture_output=True)
            subprocess.run(["codesign", "--verify", "--strict", str(binary)], check=True, capture_output=True)
            result = subprocess.run(["codesign", "--verify", "--strict", "-R=notarized", "--check-notarization", str(binary)], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("specified code requirement", result.stderr)


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
