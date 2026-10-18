import argparse
import base64
import copy
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

sys.path.insert(0, str(Path(__file__).parent))
import package
import update_tap


class UpdateTapTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = "a" * 40
        self.tag = "v1.2.3"
        for name in ["collied-1.2.3-linux-x86_64.tar.gz", "CollieTray-1.2.3-omarchy.tar.gz",
                     "collied-1.2.3-macos-arm64.tar.gz", "collied-1.2.3-macos-arm64.dmg", "CollieBar-1.2.3-macos-arm64.zip"]:
            (self.root / name).write_bytes(name.encode())
        (self.root / "source.json").write_text(json.dumps({"tag": self.tag, "source_sha": self.source}))
        with patch.object(package, "identity", return_value="1.2.3"):
            package.homebrew(argparse.Namespace(tag=self.tag, sha=self.source, out=self.root))
        files = sorted(path for path in self.root.iterdir() if path.name != "SHA256SUMS")
        (self.root / "SHA256SUMS").write_text("".join(f"{package.digest(path)}  {path.name}\n" for path in files))
        self.release = dict(id=1, target_commitish=self.source, draft=False, prerelease=False,
                            body='<!-- collie-desktop-source:' + json.dumps({"source_sha": self.source, "tag": self.tag}, sort_keys=True) + ' -->',
                            assets=[dict(id=index, name=path.name, size=path.stat().st_size, digest="sha256:" + package.digest(path))
                                    for index, path in enumerate(sorted(self.root.iterdir()))])
        with tarfile.open(self.root / "homebrew-1.2.3.tar.gz") as archive:
            self.current = {path: archive.extractfile("homebrew/" + path).read().decode().replace("1.2.3", "1.2.2") for path in update_tap.FILES}
        self.writes = []
        self.reads = []
        self.release_reads = 0
        self.drift = False
        self.concurrent = False

    def api(self, repo, path, data=None, method=None):
        if data is not None:
            self.writes.append((repo, path, data, method))
            if path == "git/trees":
                return {"sha": "new-tree"}
            if path == "git/commits":
                return {"sha": "new-commit"}
            if self.concurrent:
                raise subprocess.CalledProcessError(1, ["gh", "api", path])
            return {"object": {"sha": "new-commit"}}
        self.reads.append((repo, path))
        if path.startswith("releases/tags/"):
            self.release_reads += 1
            result = copy.deepcopy(self.release)
            if self.drift and self.release_reads == 2:
                result["assets"][0]["digest"] = "sha256:" + "f" * 64
            return result
        if path.startswith("git/ref/tags/"):
            return {"object": {"type": "commit", "sha": self.source}}
        if path == "git/ref/heads/master":
            return {"object": {"sha": "previous-commit"}}
        if path == "git/commits/previous-commit":
            return {"tree": {"sha": "previous-tree-with-heed"}}
        relative = path.removeprefix("contents/").split("?", 1)[0]
        return {"type": "file", "encoding": "base64", "content": base64.b64encode(self.current[relative].encode()).decode()}

    def run_update(self):
        with patch.dict(os.environ, {"HOMEBREW_TAP_TOKEN": "test-only"}), \
             patch.object(update_tap, "api", side_effect=self.api), \
             patch.object(update_tap, "download", side_effect=lambda tag, name, destination: shutil.copyfile(self.root / name, destination / name)):
            update_tap.update(self.tag, self.source)

    def test_updates_only_two_files_in_one_non_forced_commit(self):
        self.run_update()
        self.assertEqual([item[1] for item in self.writes], ["git/trees", "git/commits", "git/refs/heads/master"])
        tree = self.writes[0][2]
        self.assertEqual(tree["base_tree"], "previous-tree-with-heed")
        self.assertEqual([item["path"] for item in tree["tree"]], update_tap.FILES)
        self.assertEqual(self.writes[1][2]["parents"], ["previous-commit"])
        self.assertEqual(self.writes[2][2], {"sha": "new-commit", "force": False})

    def test_matching_release_is_a_noop(self):
        self.current = {path: text.replace("1.2.2", "1.2.3") for path, text in self.current.items()}
        self.run_update()
        self.assertEqual(self.writes, [])

    def test_newer_or_conflicting_same_version_is_not_overwritten(self):
        for version in ["1.2.4", "1.2.3"]:
            with self.subTest(version=version):
                self.current = {path: text.replace('version "1.2.2"', f'version "{version}"') for path, text in self.current.items()}
                with self.assertRaisesRegex(ValueError, "downgrade|same-version"):
                    self.run_update()
                self.assertEqual(self.writes, [])
                self.current = {path: text.replace(f'version "{version}"', 'version "1.2.2"') for path, text in self.current.items()}

    def test_changed_tap_behavior_needs_a_pull_request(self):
        self.current[update_tap.FILES[0]] += "\nchanged behavior\n"
        with self.assertRaisesRegex(ValueError, "behavior changed"):
            self.run_update()
        self.assertEqual(self.writes, [])

    def test_changed_release_stops_before_tap_write(self):
        self.drift = True
        with self.assertRaisesRegex(ValueError, "changed during"):
            self.run_update()
        self.assertEqual(self.writes, [])

    def test_missing_digest_is_refused(self):
        self.release["assets"][0]["digest"] = None
        with self.assertRaisesRegex(ValueError, "digest"):
            self.run_update()
        self.assertEqual(self.writes, [])

    def test_corrupt_download_is_refused(self):
        (self.root / "source.json").write_text("corrupt")
        with self.assertRaisesRegex(ValueError, "downloaded release asset differs"):
            self.run_update()
        self.assertEqual(self.writes, [])

    def test_unsafe_archive_member_is_refused(self):
        with tarfile.open(self.root / "homebrew-1.2.3.tar.gz", "w:gz") as archive:
            member = tarfile.TarInfo("../heed.rb")
            member.size = 4
            archive.addfile(member, io.BytesIO(b"evil"))
        for asset in self.release["assets"]:
            if asset["name"] == "homebrew-1.2.3.tar.gz":
                asset.update(size=(self.root / asset["name"]).stat().st_size, digest="sha256:" + package.digest(self.root / asset["name"]))
        sums = self.root / "SHA256SUMS"
        sums.write_text("".join(f"{asset['digest'][7:]}  {asset['name']}\n" for asset in self.release["assets"] if asset["name"] != "SHA256SUMS"))
        next(asset for asset in self.release["assets"] if asset["name"] == "SHA256SUMS").update(size=sums.stat().st_size, digest="sha256:" + package.digest(sums))
        with self.assertRaisesRegex(ValueError, "archive member"):
            self.run_update()
        self.assertEqual(self.writes, [])

    def test_concurrent_tap_commit_is_not_forced_or_retried(self):
        self.concurrent = True
        with self.assertRaises(subprocess.CalledProcessError):
            self.run_update()
        self.assertEqual(self.writes[-1][2]["force"], False)
        self.assertEqual(sum(item[1] == "git/refs/heads/master" for item in self.writes), 1)

    def test_source_claim_or_inventory_mismatch_stops_before_write(self):
        original = copy.deepcopy(self.release)
        for change in ["source", "inventory"]:
            with self.subTest(change=change):
                self.release = copy.deepcopy(original)
                if change == "source":
                    self.release["body"] = "wrong source"
                else:
                    self.release["assets"].pop()
                with self.assertRaisesRegex(ValueError, "source differs|inventory differs"):
                    self.run_update()
                self.assertEqual(self.writes, [])

    def test_tap_credential_is_used_only_for_tap_api(self):
        with patch.dict(os.environ, {"GH_TOKEN": "source-test-only", "HOMEBREW_TAP_TOKEN": "tap-test-only"}), \
             patch.object(update_tap.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, "{}")) as native:
            update_tap.api(update_tap.SOURCE, "releases/tags/v1.2.3")
            update_tap.api(update_tap.TAP, "git/ref/heads/master")
            self.assertEqual([call.kwargs["env"]["GH_TOKEN"] for call in native.call_args_list],
                             ["source-test-only", "tap-test-only"])
            self.assertTrue(all("tap-test-only" not in call.args[0] for call in native.call_args_list))

    def test_missing_token_or_invalid_identity_stops_before_requests(self):
        with patch.dict(os.environ, {}, clear=True), patch.object(update_tap, "api") as api:
            with self.assertRaisesRegex(ValueError, "HOMEBREW_TAP_TOKEN"):
                update_tap.update(self.tag, self.source)
            with self.assertRaisesRegex(ValueError, "exact release"):
                update_tap.update("master", self.source)
            api.assert_not_called()


if __name__ == "__main__":
    unittest.main()
