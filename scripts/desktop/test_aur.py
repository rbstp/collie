import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import aur


class AurTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        shutil.copytree(aur.ROOT / "packaging/aur", self.root / "packaging/aur")
        self.downloads = self.root / "downloads"
        self.downloads.mkdir()
        (self.downloads / "source.json").write_text(json.dumps({"tag": "v1.2.3", "source_sha": "a" * 40}))

    def assets(self, tray_source="a" * 40, glibc="2.39"):
        sums = []
        for stem, platform in [("collied-1.2.3-linux-x86_64", "linux-x86_64"), ("CollieTray-1.2.3-omarchy", "omarchy")]:
            path = self.downloads / f"{stem}.tar.gz"
            metadata = dict(version="1.2.3", source_sha="a" * 40 if platform == "linux-x86_64" else tray_source,
                            platform=platform, protocol_min=26, protocol_max=26, glibc_required=glibc)
            members = {f"{stem}/release.json": metadata}
            if platform == "omarchy":
                members[f"{stem}/CollieTray/manifest.json"] = dict(id="rbstp.collie", version="1.2.3")
            with tarfile.open(path, "w:gz") as archive:
                for name, value in members.items():
                    data = json.dumps(value).encode()
                    info = tarfile.TarInfo(name)
                    info.size = len(data)
                    archive.addfile(info, io.BytesIO(data))
            sums.append(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}")
        (self.downloads / "SHA256SUMS").write_text("\n".join(sums) + "\n")

    def test_update_pins_both_actual_assets_and_helpers(self):
        self.assets()
        with patch.object(aur, "ROOT", self.root):
            aur.update(self.downloads)
        for name, asset, helper in [("collied-bin", "collied-1.2.3-linux-x86_64", "collied-aur-adopt"),
                                    ("collie-tray", "CollieTray-1.2.3-omarchy", "collie-tray-activate")]:
            recipe = self.root / "packaging/aur" / name / "PKGBUILD"
            text = recipe.read_text()
            self.assertIn("pkgver=1.2.3\npkgrel=1\n", text)
            self.assertIn(hashlib.sha256((self.downloads / f"{asset}.tar.gz").read_bytes()).hexdigest(), text)
            self.assertIn(hashlib.sha256((recipe.parent / helper).read_bytes()).hexdigest(), text)

    def test_rejects_mismatched_release_without_partial_update(self):
        self.assets(tray_source="b" * 40)
        recipe = self.root / "packaging/aur/collied-bin/PKGBUILD"
        before = recipe.read_bytes()
        with patch.object(aur, "ROOT", self.root), self.assertRaisesRegex(ValueError, "identity mismatch"):
            aur.update(self.downloads)
        self.assertEqual(recipe.read_bytes(), before)

    def test_rejects_corrupt_download_and_higher_runtime_floor(self):
        self.assets()
        (self.downloads / "CollieTray-1.2.3-omarchy.tar.gz").write_bytes(b"corrupt")
        with patch.object(aur, "ROOT", self.root), self.assertRaisesRegex(ValueError, "checksum mismatch"):
            aur.update(self.downloads)
        self.assets(glibc="2.40")
        with patch.object(aur, "ROOT", self.root), self.assertRaisesRegex(ValueError, "glibc dependency"):
            aur.update(self.downloads)

    @unittest.skipUnless(sys.platform == "linux", "Arch helper uses GNU coreutils")
    def test_adoption_preserves_custom_unit_and_running_state(self):
        for active in ["yes", "no"]:
            with self.subTest(active=active), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                unit = root / "systemd/user/collied.service"
                unit.parent.mkdir(parents=True)
                original = '[Service]\nExecStart="/home/user/.cargo/bin/collied" "--config" "/home/user/custom config.toml" "run"\nEnvironment="XDG_DATA_HOME=/custom/data"\n'
                unit.write_text(original)
                commands = root / "bin"
                commands.mkdir()
                for name, source in {
                    "id": '#!/bin/sh\necho 1000\n',
                    "systemctl": '''#!/bin/sh
case "$*" in
  *FragmentPath*) echo "$UNIT" ;;
  *DropInPaths*) echo "${DROP_INS:-}" ;;
  *is-active*) [ "$ACTIVE" = yes ] ;;
  *) echo "$*" >> "$CALLS" ;;
esac
''',
                }.items():
                    command = commands / name
                    command.write_text(source)
                    command.chmod(0o755)
                calls = root / "calls"
                env = dict(os.environ, PATH=f"{commands}:{os.environ['PATH']}", UNIT=str(unit), ACTIVE=active,
                           XDG_CONFIG_HOME=str(root), CALLS=str(calls))
                helper = aur.ROOT / "packaging/aur/collied-bin/collied-aur-adopt"
                for command in [
                    '/usr/bin/env /home/user/.cargo/bin/collied run',
                    '"/usr/bin/env" "/home/user/.cargo/bin/collied" "run"',
                    '"/usr/bin/unrelated" "run"',
                    '"/home/user/.cargo/bin/collied" "--unexpected" "run"',
                ]:
                    rejected = f'[Service]\nExecStart={command}\n'
                    unit.write_text(rejected)
                    result = subprocess.run(["bash", str(helper)], env=env, capture_output=True)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(b"Unrecognized ExecStart", result.stderr)
                    self.assertEqual(unit.read_text(), rejected)
                    self.assertFalse(unit.with_name("collied.service.before-aur").exists())
                    self.assertFalse(calls.exists())
                unit.write_text(original)
                subprocess.run(["bash", str(helper)], env=env, check=True, capture_output=True)
                self.assertEqual(unit.read_text(), original.replace('"/home/user/.cargo/bin/collied"', '"/usr/bin/collied"'))
                self.assertEqual(unit.with_name("collied.service.before-aur").read_text(), original)
                expected = ["--user daemon-reload"]
                if active == "yes":
                    expected.append("--user restart collied.service")
                self.assertEqual(calls.read_text().splitlines(), expected)
                before = unit.read_bytes()
                result = subprocess.run(["bash", str(helper)], env=env, capture_output=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(unit.read_bytes(), before)
