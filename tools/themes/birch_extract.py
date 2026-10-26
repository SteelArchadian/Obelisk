"""Cut the Birch Grove theme pieces straight out of the concept image (real pixels, no repainting).

    python tools/themes/birch_extract.py

Writes to app/static/themes/birch/. Coordinates are for ref/birch-concept.png (1367x1150).
"""

from pathlib import Path

from PIL import Image

HERE = Path(__file__).parent
OUT = HERE.parent.parent / "app" / "static" / "themes" / "birch"
OUT.mkdir(parents=True, exist_ok=True)
src = Image.open(HERE / "ref" / "birch-concept.png").convert("RGB")


def seamless_x(tile: Image.Image) -> Image.Image:
    """Tile + its mirror, so it repeats horizontally without a seam."""
    w, h = tile.size
    out = Image.new("RGB", (w * 2, h))
    out.paste(tile, (0, 0))
    out.paste(tile.transpose(Image.Transpose.FLIP_LEFT_RIGHT), (w, 0))
    return out


# Wood button ("Shaders (3)"): keep both ends with their nails, rebuild the middle from a text-free strip.
btn = src.crop((372, 271, 486, 319))  # outer dark ring included, background excluded
left = btn.crop((0, 0, 16, btn.height))        # ring + nails
strip = btn.crop((17, 0, 24, btn.height))      # plain plank between the left nails and the text
right = btn.crop((94, 0, 114, btn.height))     # plain plank, nails, ring
# One strip only: the middle of a border-image is stretched, so repeats would show as seams.
button = Image.new("RGB", (16 + 7 + 20, btn.height))
button.paste(left, (0, 0))
button.paste(strip, (16, 0))
button.paste(right, (23, 0))
button.save(OUT / "button.png")

# Mod list surface: one row (separator line included) from an empty stretch of the table.
seamless_x(src.crop((560, 459, 684, 509))).save(OUT / "row.png")

# Table header bar (dark wood).
seamless_x(src.crop((560, 384, 684, 408))).save(OUT / "header.png")

print("wrote", sorted(p.name for p in OUT.iterdir()))
