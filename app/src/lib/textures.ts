// 16×16 pixel textures for the block themes' frames, buttons and panels, generated once per
// material and turned into data URLs (used with image-rendering: pixelated).

export type Material = "oak" | "spruce" | "jungle" | "birch" | "deepslate" | "blackstone" | "amethyst" | "netherbrick" | "endstone" | "prismarine";
type Variant = "frame" | "plate" | "panel";

const cache = new Map<string, string>();

function rng(seed: number) {
  return () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 4294967296);
}
const hex = (c: string) => [1, 3, 5].map((i) => parseInt(c.slice(i, i + 2), 16));
const shade = (c: string, f: number) => `rgb(${hex(c).map((v) => Math.max(0, Math.min(255, Math.round(v * f)))).join(",")})`;

/** Paint the bare material onto a 16×16 context. */
function paint(g: CanvasRenderingContext2D, m: Material) {
  const r = rng(m.length * 7919 + m.charCodeAt(0));
  const px = (x: number, y: number, c: string) => { g.fillStyle = c; g.fillRect(x, y, 1, 1); };
  const noise = (base: string, spread: number) => {
    for (let y = 0; y < 16; y++) for (let x = 0; x < 16; x++) px(x, y, shade(base, 1 + (r() - 0.5) * spread));
  };
  const planks = (base: string, seam: string) => {
    noise(base, 0.18);
    for (let y = 0; y < 16; y++) {
      if (y % 4 === 3) for (let x = 0; x < 16; x++) px(x, y, seam);
      // grain streaks
      if (y % 4 === 1 && r() < 0.8) { const x0 = Math.floor(r() * 12); for (let x = x0; x < x0 + 4; x++) px(x, y, shade(base, 0.86)); }
    }
    for (let row = 0; row < 4; row++) { const x = (row * 7 + 3) % 16; px(x, row * 4, seam); px(x, row * 4 + 1, seam); px(x, row * 4 + 2, seam); }
  };
  const bricks = (base: string, mortar: string) => {
    noise(base, 0.2);
    for (let y = 0; y < 16; y++) for (let x = 0; x < 16; x++) {
      const row = Math.floor(y / 4), off = row % 2 ? 4 : 0;
      if (y % 4 === 3 || (x + off) % 8 === 7) px(x, y, mortar);
    }
  };
  switch (m) {
    case "oak": planks("#a2743f", "#5e4022"); break;
    case "spruce": planks("#6e4b2c", "#3a2615"); break;
    case "jungle":
      planks("#9e6a43", "#5a3a22");
      for (let i = 0; i < 14; i++) px(Math.floor(r() * 16), Math.floor(r() * 16), r() < 0.5 ? "#4f7f2a" : "#3d6a20"); // moss
      break;
    case "birch":
      noise("#e4ded0", 0.08);
      for (let i = 0; i < 7; i++) { const y = Math.floor(r() * 16), x = Math.floor(r() * 13); const l = 2 + Math.floor(r() * 3);
        for (let k = 0; k < l; k++) px(x + k, y, "#2c2a26"); }
      break;
    case "deepslate": bricks("#4d535c", "#2c3036"); break;
    case "blackstone":
      bricks("#2e2829", "#171314");
      for (let i = 0; i < 5; i++) px(Math.floor(r() * 16), Math.floor(r() * 16), "#c42424");
      break;
    case "amethyst":
      noise("#7d58c9", 0.3);
      for (let i = 0; i < 10; i++) px(Math.floor(r() * 16), Math.floor(r() * 16), "#d9c2ff");
      break;
    case "netherbrick": bricks("#47191a", "#220b0c"); break;
    case "endstone":
      noise("#dcd79f", 0.12);
      for (let i = 0; i < 18; i++) px(Math.floor(r() * 16), Math.floor(r() * 16), "#b9b37b");
      break;
    case "prismarine":
      noise("#5aa999", 0.25);
      for (let i = 0; i < 12; i++) px(Math.floor(r() * 16), Math.floor(r() * 16), r() < 0.5 ? "#7fd4c3" : "#3e7f78");
      break;
  }
}

export function texture(m: Material, v: Variant): string {
  const key = `${m}/${v}`;
  if (cache.has(key)) return cache.get(key)!;
  const c = document.createElement("canvas");
  c.width = c.height = 16;
  const g = c.getContext("2d")!;
  paint(g, m);
  if (v !== "frame") {
    // Buttons: the material, darkened so light text reads on it. Panels: just a hint of it.
    g.fillStyle = v === "plate" ? "rgba(0,0,0,0.55)" : "rgba(0,0,0,0.86)";
    g.fillRect(0, 0, 16, 16);
  }
  const url = c.toDataURL("image/png");
  cache.set(key, url);
  return url;
}
