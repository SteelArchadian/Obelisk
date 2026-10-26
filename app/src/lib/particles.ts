// Drifting particles over the scene (embers, fireflies, snow, bubbles...). Cheap: a few dozen
// glowing pixels redrawn at ~30 fps, paused while the window is hidden or motion is reduced.

type Fx = { colors: string[]; n: number; vy: number; vx: number; size: number; twinkle?: boolean };

const FX: Record<string, Fx> = {
  nether: { colors: ["#ffb347", "#ff7a1a", "#ffd27a"], n: 70, vy: -0.55, vx: 0.15, size: 3 },
  redstone: { colors: ["#ff3434", "#ff7a6b"], n: 40, vy: -0.25, vx: 0.05, size: 3 },
  end: { colors: ["#d36bff", "#a24dff", "#f0c8ff"], n: 55, vy: -0.15, vx: 0.08, size: 3, twinkle: true },
  amethyst: { colors: ["#e2c8ff", "#a47dff", "#ffffff"], n: 45, vy: -0.1, vx: 0.05, size: 3, twinkle: true },
  deepslate: { colors: ["#7fe3ff", "#c8f4ff"], n: 35, vy: -0.12, vx: 0.04, size: 2, twinkle: true },
  mine: { colors: ["#ffcf6b", "#e8d2a6"], n: 35, vy: -0.08, vx: 0.1, size: 2 },
  birch: { colors: ["#fff2a8", "#ffd27a", "#ffffff"], n: 40, vy: -0.08, vx: 0.18, size: 3, twinkle: true },
  plains: { colors: ["#ffffff", "#fff6c8"], n: 30, vy: -0.05, vx: 0.25, size: 2, twinkle: true },
  jungle: { colors: ["#d8ff7a", "#fff27a", "#8cff9a"], n: 45, vy: -0.06, vx: 0.08, size: 3, twinkle: true },
  ocean: { colors: ["#c8fbff", "#8ff0ff"], n: 50, vy: -0.45, vx: 0.04, size: 3 },
  taiga: { colors: ["#ffffff", "#e6eef2"], n: 90, vy: 0.45, vx: 0.2, size: 3 },
};

let stop: (() => void) | null = null;

export function runParticles(canvas: HTMLCanvasElement, scene: string | null) {
  stop?.();
  stop = null;
  const g = canvas.getContext("2d")!;
  const fx = scene ? FX[scene] : undefined;
  const fit = () => { canvas.width = window.innerWidth; canvas.height = window.innerHeight; };
  fit();
  g.clearRect(0, 0, canvas.width, canvas.height);
  if (!fx || matchMedia("(prefers-reduced-motion: reduce)").matches) return;

  const W = () => canvas.width, H = () => canvas.height;
  const spawn = (anywhere: boolean) => ({
    x: Math.random() * W(),
    y: anywhere ? Math.random() * H() : fx.vy < 0 ? H() + 10 : -10,
    s: 0.5 + Math.random() * 1.2,
    ph: Math.random() * Math.PI * 2,
    c: fx.colors[Math.floor(Math.random() * fx.colors.length)],
  });
  const ps = Array.from({ length: fx.n }, () => spawn(true));
  let raf = 0, last = 0, t = 0;
  const frame = (now: number) => {
    raf = requestAnimationFrame(frame);
    if (now - last < 33) return;
    last = now; t += 1;
    g.clearRect(0, 0, W(), H());
    for (const p of ps) {
      p.y += fx.vy * p.s * 2;
      p.x += (fx.vx + Math.sin(t / 40 + p.ph) * 0.3) * p.s;
      if (p.y < -12 || p.y > H() + 12 || p.x < -12 || p.x > W() + 12) Object.assign(p, spawn(false));
      const a = fx.twinkle ? 0.35 + 0.65 * Math.abs(Math.sin(t / 25 + p.ph)) : 0.85;
      const sz = Math.round(fx.size * p.s);
      g.globalAlpha = a * 0.35;
      g.fillStyle = p.c;
      g.fillRect(Math.round(p.x) - sz, Math.round(p.y) - sz, sz * 3, sz * 3); // soft halo
      g.globalAlpha = a;
      g.fillRect(Math.round(p.x), Math.round(p.y), sz, sz);
    }
    g.globalAlpha = 1;
  };
  raf = requestAnimationFrame(frame);
  const onVis = () => { cancelAnimationFrame(raf); if (!document.hidden) raf = requestAnimationFrame(frame); };
  document.addEventListener("visibilitychange", onVis);
  window.addEventListener("resize", fit);
  stop = () => { cancelAnimationFrame(raf); document.removeEventListener("visibilitychange", onVis); window.removeEventListener("resize", fit); };
}
