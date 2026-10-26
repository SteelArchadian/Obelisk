// Paints the landscape behind the app for the block themes. Everything is drawn on a canvas at
// 1/4 resolution and scaled up without smoothing, so it reads as real pixel art; glows are added
// at full resolution on top. Seeded, so a theme looks the same every launch.
// Composition rule: big scenery (sky, mountains, water) can sit anywhere, but small props
// (lanterns, flowers, mobs) stay near the edges so they never look like part of the UI.

const PX = 4; // CSS px per scene pixel

type G = CanvasRenderingContext2D;
type Glow = { x: number; y: number; r: number; color: string };

function rng(seed: number) {
  return () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 4294967296);
}
function noise1(seed: number) {
  const r = rng(seed);
  const pts = Array.from({ length: 512 }, () => r());
  return (x: number) => {
    const i = Math.floor(x), f = x - i, s = f * f * (3 - 2 * f);
    return pts[((i % 512) + 512) % 512] * (1 - s) + pts[(((i + 1) % 512) + 512) % 512] * s;
  };
}
const mix = (a: string, b: string, t: number) => {
  const p = (c: string) => [1, 3, 5].map((i) => parseInt(c.slice(i, i + 2), 16));
  const A = p(a), B = p(b);
  return `rgb(${A.map((v, i) => Math.round(v + (B[i] - v) * t)).join(",")})`;
};

class Painter {
  r: () => number;
  glows: Glow[] = [];
  constructor(public g: G, public W: number, public H: number, seed: number) { this.r = rng(seed); }
  rect(x: number, y: number, w: number, h: number, c: string) { this.g.fillStyle = c; this.g.fillRect(Math.round(x), Math.round(y), Math.max(1, Math.round(w)), Math.max(1, Math.round(h))); }
  /** Banded vertical gradient (stepped like pixel art). */
  sky(stops: string[], bands = 22) {
    for (let i = 0; i < bands; i++) {
      const t = i / (bands - 1), seg = t * (stops.length - 1), k = Math.min(stops.length - 2, Math.floor(seg));
      this.rect(0, (i * this.H) / bands, this.W, this.H / bands + 1, mix(stops[k], stops[k + 1], seg - k));
    }
  }
  stars(n: number, maxY: number, colors = ["#ffffff", "#d8c8ff", "#b9a4ff"]) {
    for (let i = 0; i < n; i++) this.rect(this.r() * this.W, this.r() * maxY, 1, 1, colors[Math.floor(this.r() * colors.length)]);
  }
  clouds(n: number, maxY: number, c = "#ffffff", shadow = "#dfe8f2") {
    for (let i = 0; i < n; i++) {
      const x = this.r() * this.W, y = this.r() * maxY, w = 14 + this.r() * 26;
      this.rect(x, y, w, 4, c); this.rect(x + 4, y - 3, w - 10, 3, c); this.rect(x, y + 4, w, 2, shadow);
    }
  }
  /** Stepped ridge line from y=base, `amp` tall; fills down to the bottom. Optional snow caps. */
  ridge(base: number, amp: number, freq: number, c: string, opts: { snow?: string; top?: string; step?: number } = {}) {
    const n = noise1(Math.floor(this.r() * 1e6)), step = opts.step ?? 2;
    for (let x = 0; x < this.W; x += step) {
      const h = Math.round(base - amp * (0.5 * n(x * freq) + 0.35 * n(x * freq * 2.7 + 9) + 0.15 * n(x * freq * 6 + 3)));
      this.rect(x, h, step, this.H - h, c);
      if (opts.top) this.rect(x, h, step, 2, opts.top);
      if (opts.snow && base - h > amp * 0.55) this.rect(x, h, step, Math.max(2, (base - h - amp * 0.55) * 0.6), opts.snow);
    }
  }
  /** A field of 4px blocks along the bottom with a top colour (grass, sand...). */
  ground(y: number, top: string, body: string[], wobble = 3) {
    const n = noise1(Math.floor(this.r() * 1e6));
    for (let x = 0; x < this.W; x += 4) {
      const gy = Math.round((y + n(x / 30) * wobble * 2) / 4) * 4;
      for (let yy = gy; yy < this.H; yy += 4) this.rect(x, yy, 4, 4, yy === gy ? top : body[Math.floor(this.r() * body.length)]);
      if (body.length) this.rect(x, gy + 3, 4, 1, body[0]);
    }
  }
  spruce(x: number, base: number, h: number, dark: string, light: string, snow?: string) {
    this.rect(x - 1, base - h * 0.25, 2, h * 0.25, "#3b2716");
    for (let i = 0; i < 5; i++) {
      const y = base - h * 0.2 - (i * h * 0.8) / 5, w = Math.max(2, (h * 0.55) * (1 - i / 5.5));
      this.rect(x - w / 2, y - h * 0.16, w, h * 0.16, i % 2 ? light : dark);
      if (snow) this.rect(x - w / 2 + 1, y - h * 0.16, w - 2, 1, snow);
    }
  }
  birch(x: number, base: number, h: number, leaves: string[]) {
    this.rect(x - 1.5, base - h, 3, h, "#e9e3d4");
    for (let y = base - h; y < base; y += 3) if (this.r() < 0.55) this.rect(x - 1.5 + Math.floor(this.r() * 2), y, 2, 1, "#2a2824");
    this.blob(x, base - h, Math.min(h * 0.28, 14), leaves);
  }
  oak(x: number, base: number, h: number, leaves: string[], trunk = "#5b3d22") {
    this.rect(x - 1.5, base - h * 0.55, 3, h * 0.55, trunk);
    this.blob(x, base - h * 0.62, Math.min(h * 0.32, 14), leaves);
  }
  blob(cx: number, cy: number, r: number, cols: string[]) {
    for (let y = -r; y <= r; y += 2) for (let x = -r * 1.2; x <= r * 1.2; x += 2) {
      if ((x / 1.2) ** 2 + y * y <= r * r * (0.75 + this.r() * 0.35)) this.rect(cx + x, cy + y, 2, 2, cols[Math.floor(this.r() * cols.length)]);
    }
  }
  vines(n: number, c: string[], maxLen: number, xs: () => number) {
    for (let i = 0; i < n; i++) {
      const x = Math.round(xs()), len = 6 + this.r() * maxLen;
      for (let y = 0; y < len; y++) if (this.r() < 0.92) this.rect(x + (y % 5 === 0 ? 1 : 0), y, 1, 1, c[Math.floor(this.r() * c.length)]);
    }
  }
  lantern(x: number, y: number, light = "#ffcf6a", glow = "rgba(255,170,70,0.55)") {
    this.rect(x - 1, y - 6, 2, 3, "#2a2a2e");
    this.rect(x - 2, y - 3, 5, 6, "#2f2f35");
    this.rect(x - 1, y - 2, 3, 4, light);
    this.glows.push({ x: x * PX, y: y * PX, r: 70, color: glow });
  }
  flower(x: number, y: number, c: string) { this.rect(x, y - 3, 1, 3, "#3f7a2a"); this.rect(x - 1, y - 4, 3, 2, c); this.rect(x, y - 4, 1, 1, "#f2d24a"); }
  mushroom(x: number, y: number, cap: string) { this.rect(x, y - 3, 2, 3, "#e8dcc4"); this.rect(x - 2, y - 5, 6, 2, cap); this.rect(x - 1, y - 6, 4, 1, cap); }
  pillar(x: number, base: number, w: number, h: number, c: string, light: string) {
    this.rect(x, base - h, w, h, c);
    this.rect(x, base - h, 1, h, light);
  }
  house(x: number, base: number, w: number, h: number) {
    this.rect(x, base - h, w, h, "#b98a4e");
    this.rect(x, base - h, w, 2, "#6b4a2a");
    for (let i = 0; i < w / 2 + 2; i += 1) this.rect(x - 2 + i, base - h - i * 0.9, w + 4 - i * 2, 1, "#5a3b22"); // roof
    this.rect(x + w / 2 - 2, base - 7, 4, 7, "#5a3b22"); // door
    this.rect(x + 2, base - h + 5, 3, 3, "#ffd77a"); this.rect(x + w - 5, base - h + 5, 3, 3, "#ffd77a");
  }
  /** Stepped haze band (fog, heat shimmer) fading out above and below its centre. */
  haze(cy: number, h: number, c: string, alpha: number) {
    const [r, g, b] = [1, 3, 5].map((i) => parseInt(c.slice(i, i + 2), 16));
    for (let y = -h; y <= h; y += 2) this.rect(0, cy + y, this.W, 2, `rgba(${r},${g},${b},${(alpha * (1 - Math.abs(y) / h)).toFixed(3)})`);
  }
  /** A fortress block: wall, crenellations, towers and glowing windows. */
  fort(x: number, base: number, w: number, h: number, c: string, light: string, win: string, glow: string) {
    this.rect(x, base - h, w, h, c);
    for (let k = 0; k < w; k += 3) this.rect(x + k, base - h - 2, 2, 2, c);
    for (let t = 0; t < 2 + Math.floor(w / 30); t++) {
      const tx = x + this.r() * (w - 8), th = h * (0.4 + this.r() * 0.6), tw = 6 + Math.floor(this.r() * 4);
      this.rect(tx, base - h - th, tw, th, c);
      this.rect(tx, base - h - th, 1, th, light);
      for (let k = 0; k < tw; k += 2) this.rect(tx + k, base - h - th - 2, 1, 2, c);
      if (this.r() < 0.8) { this.rect(tx + tw / 2 - 1, base - h - th + 4, 2, 3, win); this.glows.push({ x: (tx + tw / 2) * PX, y: (base - h - th + 5) * PX, r: 34, color: glow }); }
    }
    for (let k = 4; k < w - 4; k += 9) if (this.r() < 0.5) { this.rect(x + k, base - h + 4, 2, 3, win); this.glows.push({ x: (x + k + 1) * PX, y: (base - h + 5) * PX, r: 26, color: glow }); }
    this.rect(x, base - h, w, 1, light);
  }
  /** Dark foreground cliffs framing both sides, with a lit inner rim. */
  cliffs(c: string[], rim: string, frac = 0.14) {
    for (const side of [0, 1]) {
      const n = noise1(Math.floor(this.r() * 1e6));
      for (let y = 0; y < this.H; y += 2) {
        const t = y / this.H, w = Math.round(this.W * frac * (0.25 + 0.75 * t * t) * (0.7 + 0.6 * n(y / 14)));
        const x = side ? this.W - w : 0;
        for (let k = 0; k < w; k += 2) this.rect(x + k, y, 2, 2, c[Math.floor(this.r() * c.length)]);
        this.rect(side ? x : w - 1, y, 1, 2, rim);
      }
    }
  }
  rays(n: number, c: string) {
    this.g.fillStyle = c;
    for (let i = 0; i < n; i++) {
      const x = this.r() * this.W, w = 6 + this.r() * 14;
      this.g.beginPath(); this.g.moveTo(x, 0); this.g.lineTo(x + w, 0); this.g.lineTo(x + w + this.H * 0.45, this.H); this.g.lineTo(x + this.H * 0.45, this.H); this.g.closePath(); this.g.fill();
    }
  }
}

/** An x position near the left or right edge (where props can go). */
const edge = (p: Painter, frac = 0.16) => (p.r() < 0.5 ? p.r() * p.W * frac : p.W - p.r() * p.W * frac);

const SCENES: Record<string, (p: Painter) => void> = {
  birch(p) {
    p.sky(["#3d6fa8", "#f2b46a", "#ffd9a0"]);
    p.ridge(p.H * 0.42, p.H * 0.18, 0.02, "#7c8aa6", { snow: "#e9eef5" });
    p.ridge(p.H * 0.55, p.H * 0.1, 0.03, "#4f6b45");
    p.rect(0, p.H * 0.58, p.W, p.H * 0.05, "#5d8fb3"); // river
    for (let x = 0; x < p.W; x += 6) if (p.r() < 0.5) p.rect(x, p.H * 0.6, 3, 1, "#ffe3a8");
    p.haze(p.H * 0.55, 16, "#ffd9a0", 0.35);
    p.ground(p.H * 0.62, "#6cab3a", ["#79553a", "#6a4a31", "#5f432c"]);
    for (let i = 0; i < 24; i++) p.birch(p.r() * p.W, p.H * 0.62, p.H * (0.1 + p.r() * 0.08), ["#8cb85a", "#a6c96a", "#c9d97a"]);
    for (let i = 0; i < 18; i++) { const x = edge(p, 0.2); p.birch(x, p.H * (0.7 + p.r() * 0.3), p.H * (0.3 + p.r() * 0.25), ["#7fb34a", "#9cc955", "#6a9c3d", "#c8d96a"]); }
    for (let i = 0; i < 40; i++) p.flower(edge(p, 0.2), p.H - p.r() * p.H * 0.08, ["#ffffff", "#ffd84a", "#ff8fb8"][i % 3]);
    for (let i = 0; i < 10; i++) p.mushroom(edge(p, 0.12), p.H - p.r() * p.H * 0.1, p.r() < 0.5 ? "#c4512f" : "#b07a4a");
    for (let i = 0; i < 6; i++) p.lantern(edge(p, 0.04), p.H * (0.35 + p.r() * 0.6));
  },
  nether(p) {
    p.sky(["#160202", "#3a0905", "#7a1a07", "#c2410c", "#ff7a1a"], 30);
    p.glows.push({ x: p.W * PX * 0.5, y: p.H * PX * 0.62, r: p.W * PX * 0.6, color: "rgba(255,110,30,0.30)" });
    // far fortress, hazy
    p.ridge(p.H * 0.58, p.H * 0.18, 0.012, "#5c1a0e", { step: 2 });
    for (let x = 0; x < p.W; x += 60 + p.r() * 50) p.fort(x, p.H * 0.5, 24 + p.r() * 30, 10 + p.r() * 10, "#4e160c", "#7a2a14", "#ffb347", "rgba(255,150,60,0.35)");
    p.haze(p.H * 0.5, 20, "#ff7a2a", 0.22);
    // the bridge, with arches, and nearer towers
    const by = Math.round(p.H * 0.44);
    p.rect(0, by, p.W, 4, "#240707"); p.rect(0, by, p.W, 1, "#7a2414");
    for (let x = 0; x < p.W; x += 3) p.rect(x, by - 2, 2, 2, "#240707");
    for (let x = 10; x < p.W; x += 44) { p.rect(x, by + 4, 5, p.H, "#1e0606"); p.rect(x, by + 4, 1, p.H, "#5a1a0e"); }
    for (let x = 10; x < p.W; x += 70 + p.r() * 50) p.fort(x, by, 24 + p.r() * 20, 14 + p.r() * 16, "#260808", "#6a2010", "#ffc35a", "rgba(255,160,60,0.5)");
    p.glows.push({ x: p.W * PX * 0.82, y: (by - 30) * PX, r: 90, color: "rgba(170,80,255,0.45)" }); // a portal glow
    p.rect(p.W * 0.82 - 4, by - 34, 8, 12, "#2a0a3a"); p.rect(p.W * 0.82 - 3, by - 33, 6, 10, "#8a3cff"); p.rect(p.W * 0.82 - 1, by - 31, 2, 6, "#d6a8ff");
    // lava falls from under the bridge into the lake
    for (let i = 0; i < 7; i++) {
      const x = Math.round(p.r() * p.W), top = by + 4 + p.r() * 8, w = 1 + Math.floor(p.r() * 3);
      for (let y = top; y < p.H * 0.88; y++) p.rect(x, y, w, 1, p.r() < 0.6 ? "#ff7a1a" : "#ffc04a");
      p.glows.push({ x: x * PX, y: (p.H * 0.65) * PX, r: 90, color: "rgba(255,110,20,0.3)" });
    }
    p.haze(p.H * 0.82, 16, "#ff5a10", 0.25);
    // dark lower basin so the lake reads as far below
    for (let y = p.H * 0.6; y < p.H * 0.88; y += 2) p.rect(0, y, p.W, 2, `rgba(20,2,2,${(0.55 * (y - p.H * 0.6) / (p.H * 0.28)).toFixed(3)})`);
    // lava lake with a bright shimmer
    for (let y = p.H * 0.88; y < p.H; y += 2) p.rect(0, y, p.W, 2, mix("#ff8a1a", "#8a1f05", (y - p.H * 0.88) / (p.H * 0.12)));
    for (let i = 0; i < p.W * 0.3; i++) p.rect(p.r() * p.W, p.H * 0.88 + p.r() * p.H * 0.12, 2 + p.r() * 4, 1, p.r() < 0.4 ? "#ffd27a" : "#b02a08");
    p.glows.push({ x: (p.W * PX) / 2, y: p.H * PX, r: p.W * PX * 0.55, color: "rgba(255,90,10,0.4)" });
    // netherrack cliffs, crimson fungi and vines at the sides
    p.cliffs(["#2a0909", "#330b0b", "#240707", "#3a0e0c"], "#ff6a2a", 0.13);
    for (let i = 0; i < 22; i++) { const x = edge(p, 0.06), y = p.H * (0.3 + p.r() * 0.7); p.rect(x, y, 2 + p.r() * 3, 1, "#ff8a2a"); }
    for (let i = 0; i < 6; i++) {
      const x = edge(p, 0.1), base = p.H * (0.55 + p.r() * 0.4), h = 14 + p.r() * 18;
      p.rect(x - 1, base - h, 3, h, "#7a1f2a");
      p.blob(x, base - h, 6 + p.r() * 4, ["#a3122a", "#c81e3a", "#7a0c1f", "#ff4a3a"]);
    }
    p.vines(26, ["#a3122a", "#7a0c1f", "#c81e3a", "#ff3a4a"], 34, () => edge(p, 0.12));
    for (let i = 0; i < 6; i++) p.lantern(edge(p, 0.05), p.H * (0.25 + p.r() * 0.6), "#7ff3ff", "rgba(60,220,255,0.6)");
  },
  end(p) {
    p.sky(["#07030f", "#1c0b33", "#2f1550"]);
    p.stars(260, p.H * 0.9);
    p.glows.push({ x: p.W * PX * 0.7, y: p.H * PX * 0.18, r: 180, color: "rgba(170,90,255,0.4)" });
    for (let i = 0; i < 9; i++) {
      const x = p.r() * p.W, y = p.H * (0.15 + p.r() * 0.45), w = 16 + p.r() * 36;
      for (let k = 0; k < 8; k++) p.rect(x + k * 1.5, y + k * 2, w - k * 3, 2, k === 0 ? "#efe9ad" : mix("#d6d08e", "#8f8a5c", k / 8));
    }
    for (let i = 0; i < 10; i++) {
      const x = p.r() * p.W, w = 6 + p.r() * 6, h = p.H * (0.3 + p.r() * 0.45);
      p.pillar(x, p.H * 0.8, w, h, "#120a1c", "#2b1a40");
      p.rect(x + w / 2 - 1.5, p.H * 0.8 - h - 4, 3, 4, "#e9b6ff");
      p.glows.push({ x: (x + w / 2) * PX, y: (p.H * 0.8 - h - 2) * PX, r: 50, color: "rgba(220,140,255,0.45)" });
    }
    p.ground(p.H * 0.82, "#efe9ad", ["#d6d08e", "#c8c27e", "#bdb774"]);
    for (let i = 0; i < 5; i++) {
      const x = edge(p, 0.12), base = p.H * 0.84;
      p.rect(x, base - 26, 2, 14, "#0a0a0a"); p.rect(x + 3, base - 26, 2, 14, "#0a0a0a"); p.rect(x - 1, base - 34, 7, 9, "#0a0a0a");
      p.rect(x - 1, base - 40, 7, 6, "#0a0a0a"); p.rect(x, base - 38, 2, 1, "#d36bff"); p.rect(x + 3, base - 38, 2, 1, "#d36bff");
      p.rect(x - 3, base - 33, 2, 16, "#0a0a0a"); p.rect(x + 6, base - 33, 2, 16, "#0a0a0a");
    }
    p.cliffs(["#140b20", "#1b1029", "#100818"], "#c27bff", 0.1);
    for (let i = 0; i < 14; i++) { const x = edge(p, 0.15); p.blob(x, p.H * (0.6 + p.r() * 0.35), 4 + p.r() * 4, ["#5c2a7a", "#7a3a9c", "#3f1c55"]); }
  },
  plains(p) {
    p.sky(["#4f9be8", "#8cc6f7", "#cfe9ff"]);
    p.clouds(14, p.H * 0.35);
    p.ridge(p.H * 0.45, p.H * 0.28, 0.012, "#8a9db3", { snow: "#f4f8fc" });
    p.ridge(p.H * 0.55, p.H * 0.12, 0.03, "#4f8a3a");
    const wx = p.W * 0.72;
    for (let y = p.H * 0.35; y < p.H * 0.62; y++) p.rect(wx, y, 6, 1, p.r() < 0.5 ? "#bfe6ff" : "#8fd0ff");
    p.rect(0, p.H * 0.62, p.W, p.H * 0.06, "#3f8fd2");
    for (let x = 0; x < p.W; x += 5) if (p.r() < 0.4) p.rect(x, p.H * 0.64, 3, 1, "#a8dcff");
    p.haze(p.H * 0.52, 18, "#e8f4ff", 0.35);
    p.ground(p.H * 0.67, "#6cc43a", ["#8a5c34", "#7a5030", "#6b4528"]);
    for (let i = 0; i < 5; i++) p.house(p.r() * p.W * 0.9, p.H * 0.66, 16 + p.r() * 8, 12 + p.r() * 6);
    for (let i = 0; i < 18; i++) p.oak(edge(p, 0.2), p.H * (0.72 + p.r() * 0.26), p.H * (0.25 + p.r() * 0.25), ["#4f9a2e", "#62b23a", "#3f8424", "#7cc64a"]);
    for (let i = 0; i < 30; i++) p.flower(edge(p, 0.2), p.H - p.r() * p.H * 0.1, ["#ffffff", "#ffd84a", "#ff5a5a"][i % 3]);
  },
  jungle(p) {
    p.sky(["#1f3a14", "#3f6a22", "#8fbf4a"]);
    p.rays(7, "rgba(255,230,140,0.12)");
    p.ridge(p.H * 0.5, p.H * 0.25, 0.02, "#2c4a1c");
    const tx = p.W * 0.62, tw = p.W * 0.22;
    for (let k = 0; k < 6; k++) p.rect(tx + k * 4, p.H * 0.6 - k * 7, tw - k * 8, 7, k % 2 ? "#6f7562" : "#7f8670");
    p.rect(tx + tw / 2 - 4, p.H * 0.6 - 14, 8, 14, "#2a2a1e");
    for (let i = 0; i < 20; i++) p.rect(tx + p.r() * tw, p.H * 0.6 - p.r() * 40, 2, 2, "#4f8a2a");
    p.haze(p.H * 0.6, 22, "#d8f0a0", 0.25);
    p.ground(p.H * 0.7, "#4c8a28", ["#5a4a2a", "#4f4024", "#6a5a3a"]);
    for (let i = 0; i < 14; i++) {
      const x = edge(p, 0.18), base = p.H * (0.75 + p.r() * 0.22), h = p.H * (0.5 + p.r() * 0.4);
      p.rect(x - 2, base - h, 4, h, "#5a4126");
      p.blob(x, base - h, Math.min(h * 0.22, 16), ["#2f7a1c", "#3f9a24", "#24601a", "#56b030"]);
    }
    p.vines(70, ["#2f7a1c", "#3f9a24", "#56b030"], 70, () => p.r() * p.W);
    for (let i = 0; i < 6; i++) p.lantern(edge(p, 0.06), p.H * (0.3 + p.r() * 0.6));
    for (let i = 0; i < 18; i++) p.flower(edge(p, 0.18), p.H - p.r() * p.H * 0.08, ["#ff6a3a", "#ffd84a"][i % 2]);
  },
  ocean(p) {
    p.sky(["#0c6a8f", "#0a4a6a", "#062a40"]);
    p.rays(9, "rgba(170,240,255,0.10)");
    const mx = p.W * 0.35, mw = p.W * 0.35;
    for (let k = 0; k < 5; k++) p.rect(mx + k * 6, p.H * 0.62 - k * 9, mw - k * 12, 9, k % 2 ? "#3f8f86" : "#4fa598");
    for (let i = 0; i < 6; i++) { const x = mx + 8 + (i * (mw - 16)) / 5; p.rect(x, p.H * 0.62 - 50, 3, 3, "#d8fff4"); p.glows.push({ x: x * PX, y: (p.H * 0.62 - 48) * PX, r: 45, color: "rgba(140,255,230,0.45)" }); }
    for (let i = 0; i < 10; i++) { const x = p.r() * p.W, h = p.H * (0.2 + p.r() * 0.4); p.pillar(x, p.H * 0.85, 6, h, "#2f6f68", "#5fb3a3"); }
    p.ground(p.H * 0.84, "#d8c78a", ["#c8b67a", "#bba86e"]);
    p.haze(p.H * 0.7, 26, "#5fd8e6", 0.18);
    p.cliffs(["#0b2a33", "#0e3440", "#082028"], "#6ff0ff", 0.1);
    const coral = ["#ff5fa2", "#ff8a3d", "#c45fff", "#ffd84a", "#4fe3ff"];
    for (let i = 0; i < 40; i++) { const x = edge(p, 0.2); p.blob(x, p.H - p.r() * p.H * 0.12, 2 + p.r() * 4, [coral[i % coral.length]]); }
    for (let i = 0; i < 70; i++) p.rect(p.r() * p.W, p.r() * p.H, 1, 1, "rgba(200,250,255,0.7)");
    for (let i = 0; i < 6; i++) p.lantern(edge(p, 0.05), p.H * (0.3 + p.r() * 0.6), "#b8fff0", "rgba(120,255,220,0.5)");
  },
  taiga(p) {
    p.sky(["#3a4a5a", "#6f7f8c", "#a7b0b5"]);
    p.ridge(p.H * 0.4, p.H * 0.3, 0.012, "#59636d", { snow: "#e8eef2" });
    for (let i = 0; i < 40; i++) p.spruce(p.r() * p.W, p.H * 0.55, p.H * (0.12 + p.r() * 0.08), "#24372e", "#2c4537");
    p.rect(0, p.H * 0.45, p.W, p.H * 0.2, "rgba(200,210,215,0.18)");
    const wy = p.H * 0.42;
    p.rect(0, wy, p.W, 3, "#5a3d26"); for (let x = 4; x < p.W; x += 18) p.rect(x, wy + 3, 2, p.H * 0.25, "#4a321e");
    for (let x = 20; x < p.W; x += 90) p.lantern(x, wy - 2);
    p.ground(p.H * 0.72, "#3f5a2e", ["#5a4128", "#4c3721"]);
    for (let i = 0; i < 26; i++) p.spruce(edge(p, 0.2), p.H * (0.76 + p.r() * 0.24), p.H * (0.3 + p.r() * 0.3), "#1c2e25", "#25402f", "#dfe7ea");
    for (let i = 0; i < 5; i++) p.lantern(edge(p, 0.05), p.H * (0.5 + p.r() * 0.45));
  },
};

/** Underground themes: textured stone with ores or crystals, lanterns at the edges. */
function cave(p: Painter, stone: string[], ore: string, oreGlow: string, style: "ore" | "crystal" | "planks") {
  const n = noise1(7), B = 7;
  for (let y = 0; y < p.H; y += B) for (let x = 0; x < p.W; x += B) {
    const dx = Math.abs(x - p.W * 0.55) / (p.W * 0.5), dy = Math.abs(y - p.H * 0.5) / (p.H * 0.5);
    const near = Math.max(dx, dy * 1.1) + (n(x / 25 + y / 40) - 0.5) * 0.5 > 0.6;
    const base = near ? 2 : 0;
    for (let yy = 0; yy < B; yy++) for (let xx = 0; xx < B; xx++) {
      let i = Math.min(stone.length - 1, base + Math.floor(p.r() * 3));
      if (style === "planks" && near && (yy === 0 || (yy === 3 && (xx + x) % 14 === 0))) i = 0;
      else if (xx === 0 || yy === 0) i = Math.max(0, i - 1);
      p.rect(x + xx, y + yy, 1, 1, stone[i]);
    }
    if (near && p.r() < 0.06) {
      if (style === "crystal") {
        const h = 4 + Math.floor(p.r() * 8);
        for (let k = 0; k < h; k++) p.rect(x + 3 - Math.floor(k / 4), y + B - k, 1 + Math.floor((h - k) / 3), 1, ore);
      } else for (let k = 0; k < 4; k++) p.rect(x + 1 + Math.floor(p.r() * 5), y + 1 + Math.floor(p.r() * 5), 1 + Math.floor(p.r() * 2), 1, ore);
      p.glows.push({ x: (x + 3) * PX, y: (y + 3) * PX, r: 40, color: oreGlow });
    }
  }
  if (style === "planks") { // mineshaft beams
    for (let x = 0; x < p.W; x += 60) p.rect(x, 0, 4, p.H, "#5e4022");
    p.rect(0, 10, p.W, 4, "#6e4b2a");
  }
  for (let i = 0; i < 7; i++) p.lantern(edge(p, 0.05), p.H * (0.25 + p.r() * 0.7));
}
SCENES.deepslate = (p) => cave(p, ["#1a1e24", "#252a32", "#2e343d", "#38404b", "#454e5b"], "#5fe3ff", "rgba(70,200,255,0.55)", "ore");
SCENES.redstone = (p) => cave(p, ["#171415", "#211d1e", "#2b2627", "#363031", "#433b3c"], "#ff2e2e", "rgba(255,30,30,0.6)", "ore");
SCENES.amethyst = (p) => cave(p, ["#1d1530", "#271c41", "#322452", "#3e2e66", "#4b387b"], "#c69bff", "rgba(170,110,255,0.6)", "crystal");
SCENES.mine = (p) => cave(p, ["#2a2622", "#36312b", "#433c34", "#524a40", "#625849"], "#ffcf6b", "rgba(255,170,60,0.5)", "planks");

export function drawScene(canvas: HTMLCanvasElement, scene: string | null) {
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const w = window.innerWidth, h = window.innerHeight;
  canvas.width = Math.ceil(w * dpr);
  canvas.height = Math.ceil(h * dpr);
  const ctx = canvas.getContext("2d")!;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  const paint = scene ? SCENES[scene] : null;
  if (!paint) return void ctx.clearRect(0, 0, w, h);

  const low = document.createElement("canvas");
  low.width = Math.ceil(w / PX);
  low.height = Math.ceil(h / PX);
  const p = new Painter(low.getContext("2d")!, low.width, low.height, 1337 + scene!.length * 101 + scene!.charCodeAt(0));
  paint(p);
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(low, 0, 0, low.width * PX, low.height * PX);
  // Bloom: a blurred, brightened copy laid over the top so bright parts glow.
  ctx.save();
  ctx.globalCompositeOperation = "screen";
  ctx.globalAlpha = 0.45;
  ctx.filter = "blur(10px) brightness(0.9) saturate(1.3)";
  ctx.imageSmoothingEnabled = true;
  ctx.drawImage(low, 0, 0, low.width * PX, low.height * PX);
  ctx.restore();
  ctx.globalCompositeOperation = "lighter";
  for (const gl of p.glows) {
    const g = ctx.createRadialGradient(gl.x, gl.y, 0, gl.x, gl.y, gl.r);
    g.addColorStop(0, gl.color);
    g.addColorStop(1, "rgba(0,0,0,0)");
    ctx.fillStyle = g;
    ctx.fillRect(gl.x - gl.r, gl.y - gl.r, gl.r * 2, gl.r * 2);
  }
  ctx.globalCompositeOperation = "source-over";
  // A soft darkening toward the bottom-centre, where most of the UI sits.
  const v = ctx.createRadialGradient(w * 0.55, h * 0.6, Math.min(w, h) * 0.2, w * 0.55, h * 0.6, Math.max(w, h) * 0.75);
  v.addColorStop(0, "rgba(0,0,0,0.35)");
  v.addColorStop(1, "rgba(0,0,0,0)");
  ctx.fillStyle = v;
  ctx.fillRect(0, 0, w, h);
}
