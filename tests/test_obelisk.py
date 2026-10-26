"""Offline tests for obelisk.py. Run from the project root: python -m unittest -v"""
import hashlib
import io
import json
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import obelisk  # noqa: E402


def sha1(data):
    return hashlib.sha1(data).hexdigest()


class FakeInstance:
    """A CurseForge instance on disk: minecraftinstance.json plus the files it lists."""

    def __init__(self, root):
        self.root = Path(root)
        self.addons = []
        self.next_id = 1000

    def add(self, name, file_name, content, class_id=6, blocked=False, enabled=True):
        folder = obelisk.CLASS_DIRS.get(class_id, "mods")
        on_disk = file_name if enabled else file_name + ".disabled"
        path = self.root / folder / on_disk
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        self.next_id += 1
        self.addons.append({
            "addonID": self.next_id, "name": name, "categoryClassID": class_id,
            "fileNameOnDisk": on_disk, "isEnabled": enabled,
            "allowModDistribution": not blocked,
            "webSiteURL": f"https://www.curseforge.com/minecraft/mc-mods/{name.lower()}",
            "installedFile": {
                "id": self.next_id * 10, "fileName": file_name, "fileLength": len(content),
                "hashes": [{"type": 1, "value": sha1(content).upper()}, {"type": 2, "value": "md5"}],
            },
        })
        return self

    def add_loose(self, folder, file_name, content):
        path = self.root / folder / file_name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        return self

    def save(self):
        self.root.mkdir(parents=True, exist_ok=True)
        (self.root / "minecraftinstance.json").write_text(json.dumps({
            "name": "Test", "gameVersion": "1.21.1", "customAuthor": "me",
            "baseModLoader": {"name": "neoforge-21.1.252"},
            "installedAddons": self.addons,
        }), encoding="utf-8")
        return self


class TempDirTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        patcher = mock.patch("builtins.print")
        patcher.start()
        self.addCleanup(patcher.stop)


class LoadBlockedTests(TempDirTest):
    def test_returns_only_blocked_files_with_metadata(self):
        inst = (FakeInstance(self.tmp / "src")
                .add("Free", "free.jar", b"free")
                .add("Locked", "locked.jar", b"locked", blocked=True)
                .save())
        blocked = obelisk.load_blocked(inst.root)
        self.assertEqual(len(blocked), 1)
        e = blocked[0]
        self.assertEqual(e["name"], "Locked")
        self.assertEqual(e["file_name"], "locked.jar")
        self.assertEqual(e["sha1"], sha1(b"locked"))  # lowercased from the manifest
        self.assertEqual(e["length"], 6)
        self.assertEqual(e["folder"], "mods")
        self.assertTrue(e["page"].endswith("/locked/files/%d" % inst.addons[1]["installedFile"]["id"]))

    def test_maps_class_ids_to_folders(self):
        inst = (FakeInstance(self.tmp / "src")
                .add("Pack", "pack.zip", b"rp", class_id=12, blocked=True)
                .add("Shader", "shader.zip", b"sh", class_id=6552, blocked=True)
                .add("Odd", "odd.jar", b"odd", class_id=99999, blocked=True)
                .save())
        folders = {e["name"]: e["folder"] for e in obelisk.load_blocked(inst.root)}
        self.assertEqual(folders, {"Pack": "resourcepacks", "Shader": "shaderpacks", "Odd": "mods"})


class IsPresentTests(TempDirTest):
    def test_matches_by_hash_even_if_renamed(self):
        target = self.tmp / "target"
        (target / "mods").mkdir(parents=True)
        (target / "mods" / "renamed.jar").write_bytes(b"locked")
        entry = {"folder": "mods", "sha1": sha1(b"locked")}
        self.assertTrue(obelisk.is_present(entry, target, {}))
        self.assertFalse(obelisk.is_present({"folder": "mods", "sha1": sha1(b"other")}, target, {}))

    def test_missing_folder_is_not_present(self):
        self.assertFalse(obelisk.is_present({"folder": "mods", "sha1": "x"}, self.tmp / "none", {}))


def fake_urlopen(payload):
    """urlopen stand-in returning bytes (or raising if payload is an exception)."""
    def opener(req, timeout=None):
        if isinstance(payload, Exception):
            raise payload
        return io.BytesIO(payload)
    return opener


class ModrinthLookupTests(TempDirTest):
    def test_keeps_only_files_with_exact_hash(self):
        good, other = sha1(b"a"), sha1(b"b")
        response = {
            good: {"files": [{"hashes": {"sha1": other}, "url": "https://cdn/other.jar"},
                             {"hashes": {"sha1": good}, "url": "https://cdn/good.jar"}]},
            other: {"files": [{"hashes": {"sha1": "nope"}, "url": "https://cdn/wrong.jar"}]},
        }
        with mock.patch.object(obelisk.urllib.request, "urlopen",
                               fake_urlopen(json.dumps(response).encode())):
            self.assertEqual(obelisk.modrinth_lookup([good, other]), {good: "https://cdn/good.jar"})

    def test_network_error_falls_back_to_empty(self):
        with mock.patch.object(obelisk.urllib.request, "urlopen", fake_urlopen(OSError("offline"))):
            self.assertEqual(obelisk.modrinth_lookup(["abc"]), {})

    def test_no_hashes_makes_no_request(self):
        with mock.patch.object(obelisk.urllib.request, "urlopen") as urlopen:
            self.assertEqual(obelisk.modrinth_lookup([]), {})
        urlopen.assert_not_called()


class DownloadModrinthTests(TempDirTest):
    entry = {"folder": "mods", "file_name": "locked.jar", "sha1": sha1(b"locked")}

    def test_places_verified_file(self):
        with mock.patch.object(obelisk.urllib.request, "urlopen", fake_urlopen(b"locked")):
            dest = obelisk.download_modrinth("https://cdn/x", self.entry, self.tmp)
        self.assertEqual(dest, self.tmp / "mods" / "locked.jar")
        self.assertEqual(dest.read_bytes(), b"locked")
        self.assertEqual(list((self.tmp / "mods").iterdir()), [dest])

    def test_hash_mismatch_raises_and_leaves_nothing(self):
        with mock.patch.object(obelisk.urllib.request, "urlopen", fake_urlopen(b"tampered")):
            with self.assertRaises(ValueError):
                obelisk.download_modrinth("https://cdn/x", self.entry, self.tmp)
        self.assertEqual(list((self.tmp / "mods").iterdir()), [])

    def test_network_error_leaves_no_temp_file(self):
        class Broken(io.BytesIO):
            def read(self, *a):
                raise OSError("connection reset")
        with mock.patch.object(obelisk.urllib.request, "urlopen", lambda *a, **k: Broken()):
            with self.assertRaises(OSError):
                obelisk.download_modrinth("https://cdn/x", self.entry, self.tmp)
        self.assertEqual(list((self.tmp / "mods").iterdir()), [])


class ExportPackTests(TempDirTest):
    def test_manifest_and_overrides(self):
        inst = (FakeInstance(self.tmp / "src")
                .add("Free", "free.jar", b"free")
                .add("Locked", "locked.jar", b"locked", blocked=True)
                .add("Off", "off.zip", b"off", class_id=12, enabled=False)
                .add_loose("mods", "mydev.jar", b"dev")
                .add_loose("shaderpacks", "bliss.zip.txt", b"cfg")
                .save())
        out = self.tmp / "pack.zip"
        obelisk.export_pack(inst.root, out)
        with zipfile.ZipFile(out) as z:
            manifest = json.loads(z.read("manifest.json"))
            names = set(z.namelist())
        self.assertEqual(manifest["minecraft"], {
            "version": "1.21.1", "modLoaders": [{"id": "neoforge-21.1.252", "primary": True}]})
        free = inst.addons[0]
        self.assertEqual(manifest["files"], [
            {"projectID": free["addonID"], "fileID": free["installedFile"]["id"], "required": True}])
        self.assertEqual(names, {
            "manifest.json",
            "overrides/mods/locked.jar",
            "overrides/resourcepacks/off.zip.disabled",
            "overrides/mods/mydev.jar",
            "overrides/shaderpacks/bliss.zip.txt",
        })


class WatchDownloadsTests(TempDirTest):
    def setUp(self):
        super().setUp()
        self.downloads = self.tmp / "Downloads"
        self.downloads.mkdir()
        self.target = self.tmp / "target"
        self.opened = []

    def entry(self, name, content, folder="mods"):
        return {"name": name, "file_name": f"{name}.jar", "sha1": sha1(content),
                "length": len(content), "folder": folder,
                "page": f"https://www.curseforge.com/{name}/files/1"}

    def run_watch(self, pending, on_open=None, batch=5, max_ticks=20):
        """Run watch_downloads with the browser and clock faked out."""
        def fake_open(url):
            self.opened.append(url)
            if on_open:
                on_open(url)
        ticks = iter(range(max_ticks))

        def fake_sleep(_):
            if next(ticks, None) is None:
                raise KeyboardInterrupt  # stop a hung loop instead of blocking the suite
        with mock.patch.object(obelisk.webbrowser, "open", fake_open), \
                mock.patch.object(obelisk.time, "sleep", fake_sleep):
            return obelisk.watch_downloads(pending, self.downloads, self.target, batch)

    def test_claims_file_the_user_downloads(self):
        a = self.entry("alpha", b"alpha-bytes")
        ok = self.run_watch([a], on_open=lambda url: (self.downloads / "alpha (1).jar").write_bytes(b"alpha-bytes"))
        self.assertTrue(ok)
        self.assertEqual((self.target / "mods" / "alpha.jar").read_bytes(), b"alpha-bytes")
        self.assertEqual(list(self.downloads.iterdir()), [])

    def test_ignores_partial_and_wrong_hash_files(self):
        a = self.entry("alpha", b"alpha-bytes")

        def on_open(url):
            (self.downloads / "alpha.jar.crdownload").write_bytes(b"alpha-bytes")
            (self.downloads / "impostor.jar").write_bytes(b"alpha-BYTES")  # same size
        ok = self.run_watch([a], on_open=on_open, max_ticks=3)
        self.assertFalse(ok)
        self.assertFalse((self.target / "mods" / "alpha.jar").exists())
        self.assertEqual(len(list(self.downloads.iterdir())), 2)

    def test_uses_file_already_in_downloads_without_opening_page(self):
        a = self.entry("alpha", b"alpha-bytes")
        (self.downloads / "alpha.jar").write_bytes(b"alpha-bytes")
        ok = self.run_watch([a], max_ticks=3)
        self.assertTrue(ok)
        self.assertEqual(self.opened, [])
        self.assertTrue((self.target / "mods" / "alpha.jar").exists())

    def test_opens_pages_in_batches(self):
        entries = [self.entry(n, n.encode() * 3) for n in ("a", "b", "c")]
        files = {e["page"]: e for e in entries}

        def on_open(url):
            e = files[url]
            (self.downloads / e["file_name"]).write_bytes(e["name"].encode() * 3)
        ok = self.run_watch(entries, on_open=on_open, batch=2)
        self.assertTrue(ok)
        self.assertEqual(self.opened, [e["page"] for e in entries])
        self.assertEqual(sorted(p.name for p in (self.target / "mods").iterdir()), ["a.jar", "b.jar", "c.jar"])


if __name__ == "__main__":
    unittest.main()
