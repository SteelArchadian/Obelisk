"""Obelisk: get the CurseForge files whose authors disabled third-party downloads.

Reads a CurseForge instance's minecraftinstance.json, finds every file with
allowModDistribution = false, and places each one into a target instance:
  1. If Modrinth hosts the exact same file (same SHA-1), download it from Modrinth.
  2. Otherwise open its CurseForge page in your browser; you click download and
     Obelisk picks the file up from your Downloads folder, verifies its SHA-1,
     and moves it into place.
No CurseForge API calls, no impersonation: CurseForge files are always fetched by you.
"""
import argparse
import hashlib
import json
import shutil
import sys
import time
import urllib.request
import webbrowser
import zipfile
from pathlib import Path

USER_AGENT = "Obelisk/0.1 (personal tool)"
# CurseForge classId -> instance subfolder
CLASS_DIRS = {6: "mods", 12: "resourcepacks", 6552: "shaderpacks", 6945: "datapacks"}
PARTIAL_SUFFIXES = (".crdownload", ".part", ".tmp", ".partial", ".download")


def sha1_of(path):
    h = hashlib.sha1()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def load_blocked(source):
    manifest = Path(source) / "minecraftinstance.json"
    data = json.loads(manifest.read_text(encoding="utf-8"))
    blocked = []
    for addon in data.get("installedAddons", []):
        if addon.get("allowModDistribution") is not False:
            continue
        f = addon["installedFile"]
        sha1 = next((h["value"] for h in f.get("hashes", []) if h.get("type") == 1), None)
        blocked.append({
            "name": addon["name"].strip(),
            "file_name": addon.get("fileNameOnDisk") or f["fileName"],
            "sha1": sha1.lower() if sha1 else None,
            "length": f.get("fileLength"),
            "folder": CLASS_DIRS.get(addon.get("categoryClassID"), "mods"),
            "page": f"{addon['webSiteURL'].rstrip('/')}/files/{f['id']}",
        })
    return blocked


def is_present(entry, target, index):
    """True if the target folder already holds a file with this entry's hash."""
    folder = Path(target) / entry["folder"]
    if entry["folder"] not in index:
        hashes = set()
        if folder.is_dir():
            for p in folder.iterdir():
                if p.is_file():
                    hashes.add(sha1_of(p))
        index[entry["folder"]] = hashes
    return entry["sha1"] in index[entry["folder"]]


def modrinth_lookup(hashes):
    """Map sha1 -> download URL for files Modrinth hosts byte-for-byte."""
    if not hashes:
        return {}
    body = json.dumps({"hashes": hashes, "algorithm": "sha1"}).encode()
    req = urllib.request.Request(
        "https://api.modrinth.com/v2/version_files", data=body, method="POST",
        headers={"Content-Type": "application/json", "User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            versions = json.load(resp)
    except OSError as e:
        print(f"  ! Modrinth lookup failed ({e}); falling back to browser for everything")
        return {}
    urls = {}
    for sha1, version in versions.items():
        for f in version.get("files", []):
            if f.get("hashes", {}).get("sha1", "").lower() == sha1.lower():
                urls[sha1.lower()] = f["url"]
    return urls


def place(src, entry, target, move):
    dest = Path(target) / entry["folder"] / entry["file_name"]
    dest.parent.mkdir(parents=True, exist_ok=True)
    (shutil.move if move else shutil.copy2)(str(src), str(dest))
    return dest


def download_modrinth(url, entry, target):
    tmp = Path(target) / entry["folder"] / (entry["file_name"] + ".obelisk-tmp")
    tmp.parent.mkdir(parents=True, exist_ok=True)
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(req, timeout=120) as resp, open(tmp, "wb") as out:
            shutil.copyfileobj(resp, out)
        if sha1_of(tmp) != entry["sha1"]:
            raise ValueError("hash mismatch")
        return place(tmp, entry, target, move=True)
    finally:
        tmp.unlink(missing_ok=True)


def watch_downloads(pending, downloads, target, batch):
    """Open pages in batches and claim matching files as they land in Downloads.

    Files already sitting in Downloads count too, so nothing is downloaded twice.
    """
    remaining = list(pending)
    opened = set()
    seen = {}
    print(f"\nWatching {downloads} - click Download on each page that opens. Ctrl+C to stop.")
    try:
        while True:
            for p in list(downloads.iterdir()):
                if not p.is_file() or p.name.lower().endswith(PARTIAL_SUFFIXES):
                    continue
                stat = p.stat()
                if seen.get(p) == stat.st_mtime:
                    continue
                seen[p] = stat.st_mtime
                candidates = [e for e in remaining if e["length"] == stat.st_size]
                if not candidates:
                    continue
                digest = sha1_of(p)
                match = next((e for e in candidates if e["sha1"] == digest), None)
                if match:
                    dest = place(p, match, target, move=True)
                    remaining.remove(match)
                    print(f"  ok {match['name']} -> {dest.parent.name}/{dest.name}")
            if not remaining:
                break
            waiting = sum(e["page"] in opened for e in remaining)
            for e in remaining:
                if waiting >= batch:
                    break
                if e["page"] not in opened:
                    opened.add(e["page"])
                    waiting += 1
                    print(f"  -> opening {e['name']}")
                    webbrowser.open(e["page"])
            time.sleep(1.5)
    except KeyboardInterrupt:
        left = [e["name"] for e in remaining]
        print(f"\nStopped. Still missing ({len(left)}): " + ", ".join(left))
        return False
    print("\nAll blocked files are in place.")
    return True


def export_pack(source, out_zip):
    """Write a CurseForge-format modpack zip that Prism can import.

    Distributable, enabled CurseForge files go in manifest.json (Prism downloads them
    and can update them later). Blocked files, disabled files and anything CurseForge
    doesn't track (local/dev mods) are copied into overrides/ from the source instance.
    """
    source = Path(source)
    data = json.loads((source / "minecraftinstance.json").read_text(encoding="utf-8"))
    files, copied, tracked = [], [], set()
    for addon in data["installedAddons"]:
        f = addon["installedFile"]
        on_disk = addon.get("fileNameOnDisk") or f["fileName"]
        folder = CLASS_DIRS.get(addon.get("categoryClassID"), "mods")
        tracked.add((folder, on_disk))
        if addon.get("allowModDistribution") is False or not addon.get("isEnabled", True):
            copied.append(Path(folder) / on_disk)
        else:
            files.append({"projectID": addon["addonID"], "fileID": f["id"], "required": True})
    for folder in ("mods", "resourcepacks", "shaderpacks"):
        d = source / folder
        if d.is_dir():
            copied += [Path(folder) / p.name for p in d.iterdir()
                       if p.is_file() and (folder, p.name) not in tracked]

    manifest = {
        "minecraft": {"version": data["gameVersion"],
                      "modLoaders": [{"id": data["baseModLoader"]["name"], "primary": True}]},
        "manifestType": "minecraftModpack", "manifestVersion": 1,
        "name": data["name"], "version": "1.0", "author": data.get("customAuthor") or "",
        "files": files, "overrides": "overrides",
    }

    out_zip = Path(out_zip)
    out_zip.parent.mkdir(parents=True, exist_ok=True)
    missing = []
    with zipfile.ZipFile(out_zip, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("manifest.json", json.dumps(manifest, indent=2))
        for rel in copied:
            if (source / rel).is_file():
                z.write(source / rel, f"overrides/{rel.as_posix()}")
            else:
                missing.append(str(rel))
    print(f"Wrote {out_zip}: {len(files)} CurseForge files in manifest, "
          f"{len(copied) - len(missing)} files in overrides")
    for m in missing:
        print(f"  ! not on disk, skipped: {m}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=["scan", "fetch", "export"])
    ap.add_argument("source", help="CurseForge instance folder (contains minecraftinstance.json)")
    ap.add_argument("--target", help="instance game folder to fill (default: source); "
                                     "for Prism use .../instances/<name>/minecraft")
    ap.add_argument("--downloads", default=str(Path.home() / "Downloads"))
    ap.add_argument("--batch", type=int, default=5, help="browser pages open at once")
    ap.add_argument("--no-modrinth", action="store_true")
    ap.add_argument("--out", help="export: zip path (default: <name>-prism.zip here)")
    args = ap.parse_args()
    if args.command == "export":
        export_pack(args.source, args.out or f"{Path(args.source).name}-prism.zip")
        return
    target = Path(args.target or args.source)

    blocked = load_blocked(args.source)
    index = {}
    missing = [e for e in blocked if not is_present(e, target, index)]
    mr = {} if args.no_modrinth else modrinth_lookup([e["sha1"] for e in missing if e["sha1"]])

    print(f"{len(blocked)} blocked files, {len(blocked) - len(missing)} already in {target}")
    for e in blocked:
        state = "present " if e not in missing else ("modrinth" if e["sha1"] in mr else "browser ")
        print(f"  [{state}] {e['folder']:<13} {e['name']}")
    if args.command == "scan" or not missing:
        return

    browser = []
    for e in missing:
        if e["sha1"] in mr:
            try:
                dest = download_modrinth(mr[e["sha1"]], e, target)
                print(f"  ok {e['name']} (Modrinth) -> {dest.parent.name}/{dest.name}")
                continue
            except (OSError, ValueError) as err:
                print(f"  ! Modrinth download failed for {e['name']} ({err}); using browser")
        browser.append(e)
    if browser:
        sys.exit(0 if watch_downloads(browser, Path(args.downloads), target, args.batch) else 1)


if __name__ == "__main__":
    main()
