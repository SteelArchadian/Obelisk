"""Generate theme art with OpenAI's image API.

The API key lives in Windows Credential Manager (generic credential "obelisk-openai-key"), or in
the OPENAI_API_KEY environment variable. It is never printed. Store it once with:

    cmdkey /generic:obelisk-openai-key /user:openai /pass

(cmdkey asks for the key without echoing it.)

Usage:
    python tools/imagegen.py check
    python tools/imagegen.py gen "prompt" -o out.png [--size 1536x1024] [--quality high] [--transparent] [--ref img.png ...]
    python tools/imagegen.py batch tools/themes/birch.json [--only name,name] [--force]

With --ref the images are sent as references (the edits endpoint), which keeps a style consistent.
Stdlib only, so it runs anywhere Python does.
"""

from __future__ import annotations

import argparse
import base64
import ctypes
import json
import mimetypes
import os
import sys
import time
import urllib.error
import urllib.request
import uuid
from ctypes import wintypes
from pathlib import Path

API = "https://api.openai.com/v1/images"
CRED = "obelisk-openai-key"
DEFAULT_MODEL = "gpt-image-1"


def api_key() -> str:
    key = os.environ.get("OPENAI_API_KEY", "").strip()
    if key:
        return key
    if sys.platform == "win32":
        class CREDENTIAL(ctypes.Structure):
            _fields_ = [
                ("Flags", wintypes.DWORD), ("Type", wintypes.DWORD), ("TargetName", wintypes.LPWSTR),
                ("Comment", wintypes.LPWSTR), ("LastWritten", wintypes.FILETIME),
                ("CredentialBlobSize", wintypes.DWORD), ("CredentialBlob", ctypes.POINTER(ctypes.c_ubyte)),
                ("Persist", wintypes.DWORD), ("AttributeCount", wintypes.DWORD), ("Attributes", ctypes.c_void_p),
                ("TargetAlias", wintypes.LPWSTR), ("UserName", wintypes.LPWSTR),
            ]
        advapi = ctypes.WinDLL("advapi32", use_last_error=True)
        pcred = ctypes.POINTER(CREDENTIAL)()
        if advapi.CredReadW(CRED, 1, 0, ctypes.byref(pcred)):
            try:
                c = pcred.contents
                blob = bytes(c.CredentialBlob[: c.CredentialBlobSize])
                # cmdkey stores UTF-16; other tools may store UTF-8.
                key = blob.decode("utf-16-le") if len(blob) > 1 and blob[1] == 0 else blob.decode("utf-8")
                return key.strip("\x00 \r\n")
            finally:
                advapi.CredFree(pcred)
    sys.exit(f"No OpenAI key. Store one with:  cmdkey /generic:{CRED} /user:openai /pass")


def post(url: str, body: bytes, content_type: str) -> dict:
    req = urllib.request.Request(url, data=body, method="POST", headers={
        "Authorization": f"Bearer {api_key()}", "Content-Type": content_type,
    })
    for attempt in range(3):
        try:
            with urllib.request.urlopen(req, timeout=300) as r:
                return json.load(r)
        except urllib.error.HTTPError as e:
            detail = e.read().decode("utf-8", "replace")[:600]
            if e.code in (429, 500, 502, 503) and attempt < 2:
                time.sleep(5 * (attempt + 1))
                continue
            sys.exit(f"OpenAI error {e.code}: {detail}")
    raise AssertionError


def multipart(fields: dict[str, str], files: list[tuple[str, Path]]) -> tuple[bytes, str]:
    b = uuid.uuid4().hex
    out = bytearray()
    for k, v in fields.items():
        out += f'--{b}\r\nContent-Disposition: form-data; name="{k}"\r\n\r\n{v}\r\n'.encode()
    for k, p in files:
        mime = mimetypes.guess_type(p.name)[0] or "image/png"
        out += f'--{b}\r\nContent-Disposition: form-data; name="{k}"; filename="{p.name}"\r\nContent-Type: {mime}\r\n\r\n'.encode()
        out += p.read_bytes() + b"\r\n"
    out += f"--{b}--\r\n".encode()
    return bytes(out), f"multipart/form-data; boundary={b}"


def generate(prompt: str, out: Path, size: str = "1536x1024", quality: str = "high", transparent: bool = False,
             refs: list[Path] | None = None, model: str = DEFAULT_MODEL) -> Path:
    params = {"model": model, "prompt": prompt, "size": size, "quality": quality, "n": "1", "output_format": "png"}
    if transparent:
        params["background"] = "transparent"
    t0 = time.time()
    if refs:
        body, ctype = multipart(params, [("image[]", r) for r in refs])
        res = post(f"{API}/edits", body, ctype)
    else:
        res = post(f"{API}/generations", json.dumps({**params, "n": 1}).encode(), "application/json")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_bytes(base64.b64decode(res["data"][0]["b64_json"]))
    usage = res.get("usage", {})
    print(f"saved {out}  ({time.time() - t0:.0f}s, {usage.get('total_tokens', '?')} tokens)")
    return out


def batch(manifest: Path, only: set[str] | None, force: bool) -> None:
    m = json.loads(manifest.read_text(encoding="utf-8"))
    root = (manifest.parent / m["out"]).resolve()
    style = m.get("style", "")
    for a in m["assets"]:
        if only and a["name"] not in only:
            continue
        out = root / f"{a['name']}.png"
        if out.exists() and not force:
            print(f"skip {a['name']} (exists; --force to redo)")
            continue
        refs = [(manifest.parent / r).resolve() for r in a.get("refs", m.get("refs", []))]
        refs = [r for r in refs if r.exists()]
        generate(f"{a['prompt']}\n\nStyle: {style}".strip(), out, a.get("size", "1024x1024"), a.get("quality", "high"),
                 a.get("transparent", False), refs or None, m.get("model", DEFAULT_MODEL))


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check", help="confirm a key is stored (never prints it)")
    g = sub.add_parser("gen")
    g.add_argument("prompt")
    g.add_argument("-o", "--out", required=True, type=Path)
    g.add_argument("--size", default="1536x1024")
    g.add_argument("--quality", default="high", choices=["low", "medium", "high", "auto"])
    g.add_argument("--transparent", action="store_true")
    g.add_argument("--ref", action="append", type=Path, default=[])
    g.add_argument("--model", default=DEFAULT_MODEL)
    b = sub.add_parser("batch")
    b.add_argument("manifest", type=Path)
    b.add_argument("--only", default="")
    b.add_argument("--force", action="store_true")
    a = ap.parse_args()
    if a.cmd == "check":
        k = api_key()
        print(f"key found ({len(k)} chars, starts {k[:3]}…)")
    elif a.cmd == "gen":
        generate(a.prompt, a.out, a.size, a.quality, a.transparent, a.ref or None, a.model)
    else:
        batch(a.manifest, set(filter(None, a.only.split(","))) or None, a.force)


if __name__ == "__main__":
    main()
