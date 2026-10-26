"""Turn generated art into theme assets: slice grid sheets, key out the magenta background, trim.

    python tools/cutout.py key in.png out.png            # magenta -> transparent, trimmed
    python tools/cutout.py grid in.png COLSxROWS a,b,c… # slice a sheet into named cells (in the same folder)
    python tools/cutout.py hole in.png out.png           # frame: make the magenta centre transparent, keep size

Magenta is matched loosely (generators never hit #FF00FF exactly) and edge pixels are de-spilled so no
pink fringe is left on soft edges.
"""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np
from PIL import Image


def magenta_alpha(rgb: np.ndarray) -> np.ndarray:
    """0 for magenta, 1 for anything clearly not magenta, smooth in between."""
    r, g, b = (rgb[..., i].astype(np.float32) for i in range(3))
    # How "magenta" a pixel is: red and blue high, green well below both.
    m = (np.minimum(r, b) - g) / 255.0
    return np.clip((0.55 - m) / 0.35, 0.0, 1.0)


def despill(rgb: np.ndarray, a: np.ndarray) -> np.ndarray:
    out = rgb.astype(np.float32)
    edge = a < 0.999
    r, g, b = out[..., 0].copy(), out[..., 1], out[..., 2].copy()
    # On fringe pixels, pull red and blue down toward green by the amount of magenta mixed in.
    spill = np.minimum(r, b) - g
    fix = edge & (spill > 0)
    out[..., 0] = np.where(fix, r - spill * (1 - a), r)
    out[..., 2] = np.where(fix, b - spill * (1 - a), b)
    return np.clip(out, 0, 255).astype(np.uint8)


def key(img: Image.Image, trim: bool = True) -> Image.Image:
    rgb = np.asarray(img.convert("RGB"))
    a = magenta_alpha(rgb)
    rgba = np.dstack([despill(rgb, a), (a * 255).astype(np.uint8)])
    out = Image.fromarray(rgba, "RGBA")
    if trim:
        box = out.getchannel("A").point(lambda v: 255 if v > 8 else 0).getbbox()
        if box:
            out = out.crop(box)
    return out


def grid(path: Path, spec: str, names: list[str]) -> None:
    cols, rows = map(int, spec.lower().split("x"))
    img = Image.open(path).convert("RGB")
    w, h = img.size
    for i, name in enumerate(names):
        if not name or name == "-":
            continue
        c, r = i % cols, i // cols
        # Inset a little so grid gutters/lines are not included.
        x0, y0 = round(c * w / cols) + 6, round(r * h / rows) + 6
        x1, y1 = round((c + 1) * w / cols) - 6, round((r + 1) * h / rows) - 6
        cell = img.crop((x0, y0, x1, y1))
        out = path.parent / f"{name}.png"
        (cell if name.endswith("surface") else key(cell)).save(out)
        print("saved", out, (cell if name.endswith("surface") else Image.open(out)).size)


def main() -> None:
    cmd = sys.argv[1]
    if cmd == "key":
        key(Image.open(sys.argv[2])).save(sys.argv[3])
    elif cmd == "hole":
        key(Image.open(sys.argv[2]), trim=False).save(sys.argv[3])
    elif cmd == "grid":
        grid(Path(sys.argv[2]), sys.argv[3], sys.argv[4].split(","))
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
