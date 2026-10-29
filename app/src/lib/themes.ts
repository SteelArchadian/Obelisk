// Block themes: colours, the material their frames and buttons are made of, and the scene painted
// behind the app. applyBlockTheme() turns one into CSS variables (including generated textures).

import { texture, type Material } from "./textures";

export type BlockTheme = {
  id: string;
  label: string;
  blurb: string;
  /** Scene painted behind the app (see scene.ts). */
  scene: string;
  /** Frames (panel borders, tabs, buttons). */
  frame: Material;
  /** Colours. Panels are near-opaque so text always stands out against the scene. */
  bg: string; panel: string; panel2: string; line: string;
  text: string; text2: string; muted: string;
  accent: string; accentHover: string; accentText: string;
  /** Section headings ("CRASHES AND LOGS") and small highlights. */
  heading: string;
  /** Frame edge colours: the light bevel and the dark outline. */
  edgeLight: string; edgeDark: string;
  /** CSS filter for the (blue) logo so it matches the theme. */
  logo: string;
  /** Painted art in static/themes/<id>/ (bg, frame, button, row, header) replaces the drawn scene. */
  art?: boolean;
};

const DARK_OUTLINE = "#0b0806";

export const BLOCK_THEMES: BlockTheme[] = [
  { id: "deepslate", label: "Deepslate", blurb: "Diamond ore glowing in a deepslate cave", scene: "deepslate", frame: "deepslate",
    bg: "#0b0f15", panel: "#141a22", panel2: "#1c2430", line: "#3a4554", text: "#eef4fb", text2: "#c9d5e2", muted: "#8e9db0",
    accent: "#2f9bff", accentHover: "#5cb3ff", accentText: "#ffffff", heading: "#7fd3ff", edgeLight: "#6b7684", edgeDark: "#07090c", logo: "none" },
  { id: "oak", label: "Oak Mine", blurb: "Lantern-lit mineshaft of oak and stone", scene: "mine", frame: "oak",
    bg: "#140d07", panel: "#1d150d", panel2: "#2a1e12", line: "#5a4128", text: "#f6ecdc", text2: "#e0cfb4", muted: "#b19a78",
    accent: "#4fbf35", accentHover: "#6ad64c", accentText: "#ffffff", heading: "#f2c46b", edgeLight: "#c08a4e", edgeDark: DARK_OUTLINE, logo: "hue-rotate(-170deg) saturate(1.4)" },
  { id: "redstone", label: "Redstone", blurb: "Blackstone halls and glowing redstone", scene: "redstone", frame: "blackstone",
    bg: "#0d0909", panel: "#171213", panel2: "#221a1b", line: "#4a3234", text: "#f6eaea", text2: "#dcc6c6", muted: "#a68f8f",
    accent: "#ff3434", accentHover: "#ff5a5a", accentText: "#ffffff", heading: "#ff7a6b", edgeLight: "#6e5a5c", edgeDark: "#050303", logo: "hue-rotate(150deg) saturate(1.4)" },
  { id: "amethyst", label: "Amethyst", blurb: "Inside a glowing geode", scene: "amethyst", frame: "amethyst",
    bg: "#0e0918", panel: "#170f26", panel2: "#211736", line: "#4b3a78", text: "#f3edff", text2: "#d6cbf2", muted: "#a497c8",
    accent: "#8b5cff", accentHover: "#a47dff", accentText: "#ffffff", heading: "#c9a6ff", edgeLight: "#9a7ad6", edgeDark: "#06040b", logo: "hue-rotate(55deg) saturate(1.2)" },
  { id: "birch", label: "Birch Grove", blurb: "Golden hour in a birch forest, bees included", scene: "birch", frame: "birch",
    bg: "#16120c", panel: "#2a2016", panel2: "#3a2c1d", line: "#5b5143", text: "#fbf6ec", text2: "#efe4cf", muted: "#cbbd9f",
    accent: "#3fbf3f", accentHover: "#5bd65a", accentText: "#ffffff", heading: "#f3e7c9", edgeLight: "#efe8d8", edgeDark: "#2a1a0c", logo: "hue-rotate(-175deg) saturate(1.6) brightness(1.15)", art: true },
  { id: "nether", label: "Nether Fortress", blurb: "Lava falls, crimson vines and soul lanterns", scene: "nether", frame: "netherbrick",
    bg: "#120707", panel: "#180d0c", panel2: "#241210", line: "#6b2a1c", text: "#fff0e8", text2: "#f0d2c4", muted: "#c0907c",
    accent: "#ff5a1f", accentHover: "#ff7b45", accentText: "#ffffff", heading: "#ff8a3d", edgeLight: "#ff7a2e", edgeDark: "#060202", logo: "hue-rotate(185deg) saturate(1.8)" },
  { id: "end", label: "The End", blurb: "End stone islands, obsidian pillars and endermen", scene: "end", frame: "endstone",
    bg: "#0c0814", panel: "#151023", panel2: "#1e1731", line: "#4a3d66", text: "#f6f1ff", text2: "#dcd2f2", muted: "#a99cc8",
    accent: "#a24dff", accentHover: "#b872ff", accentText: "#ffffff", heading: "#d7b8ff", edgeLight: "#e8e2a6", edgeDark: "#08060c", logo: "hue-rotate(60deg) saturate(1.3)" },
  { id: "plains", label: "Plains Village", blurb: "Sunny village, mountains and a waterfall", scene: "plains", frame: "oak",
    bg: "#16212b", panel: "#1b222b", panel2: "#242e39", line: "#56473a", text: "#f7f9fb", text2: "#dde5ec", muted: "#a8b5c2",
    accent: "#3fcf3a", accentHover: "#5be356", accentText: "#ffffff", heading: "#ffe28a", edgeLight: "#c48d52", edgeDark: DARK_OUTLINE, logo: "hue-rotate(-175deg) saturate(1.6) brightness(1.15)" },
  { id: "jungle", label: "Jungle Temple", blurb: "Mossy temple ruins under the canopy", scene: "jungle", frame: "jungle",
    bg: "#11110a", panel: "#19170f", panel2: "#232014", line: "#5c4b2a", text: "#f8f3e4", text2: "#e4dcc2", muted: "#b2a684",
    accent: "#2fd35a", accentHover: "#4fe679", accentText: "#ffffff", heading: "#f0b84a", edgeLight: "#b48a4c", edgeDark: "#070604", logo: "hue-rotate(-100deg) saturate(1.5)" },
  { id: "ocean", label: "Ocean Monument", blurb: "Prismarine ruins deep under the sea", scene: "ocean", frame: "prismarine",
    bg: "#061820", panel: "#0c2029", panel2: "#112b36", line: "#2f6f74", text: "#eefcff", text2: "#c8eef2", muted: "#8bbcc4",
    accent: "#21c7e8", accentHover: "#4fd9f2", accentText: "#ffffff", heading: "#6ff0ff", edgeLight: "#5fd0c0", edgeDark: "#02090c", logo: "hue-rotate(-20deg) saturate(1.3)" },
  { id: "taiga", label: "Taiga", blurb: "Misty spruce forest and lantern-lit walkways", scene: "taiga", frame: "spruce",
    bg: "#0f1211", panel: "#171513", panel2: "#211e1a", line: "#4f3d2c", text: "#f4efe8", text2: "#ddd4c8", muted: "#a99d8e",
    accent: "#21c9b8", accentHover: "#47ddce", accentText: "#ffffff", heading: "#e9c58f", edgeLight: "#8f6a46", edgeDark: "#060504", logo: "hue-rotate(-175deg) saturate(1.6) brightness(1.1)" },
];

export const blockTheme = (id: string) => BLOCK_THEMES.find((t) => t.id === id);

/** Set every CSS variable the block skin uses. Textures are generated once per material. */
export function applyBlockTheme(id: string) {
  const t = blockTheme(id);
  const s = document.documentElement.style;
  const vars: Record<string, string> = t ? {
    "--bg": t.bg, "--rail": t.panel, "--panel": t.panel, "--panel-2": t.panel2, "--line": t.line,
    "--text": t.text, "--text-2": t.text2, "--muted": t.muted, "--row-line": "rgb(255 255 255 / 0.07)",
    "--scroll": t.line, "--switch-off": t.line,
    "--accent": t.accent, "--accent-hover": t.accentHover, "--accent-text": t.accentText,
    "--glow": `color-mix(in srgb, ${t.accent} 55%, transparent)`, "--heading": t.heading,
    "--edge-light": t.edgeLight, "--edge-dark": t.edgeDark, "--logo-filter": t.logo,
    "--tex-frame": `url(${texture(t.frame, "frame")})`,
    "--tex-plate": `url(${texture(t.frame, "plate")})`,
    "--tex-panel": `url(${texture(t.frame, "panel")})`,
  } : {};
  const art = t?.art ? `/themes/${t.id}` : "";
  if (art) Object.assign(vars, Object.fromEntries(ART.map((n) => [`--art-${n}`, `url(${art}/${n}.png)`])));
  if (art) document.documentElement.dataset.art = t!.id;
  else delete document.documentElement.dataset.art;
  for (const k of ALL_VARS) {
    if (vars[k]) s.setProperty(k, vars[k]);
    else s.removeProperty(k);
  }
}
const ART = ["bg", "frame", "button", "row", "header", "green", "red", "plank", "well", "win-min", "win-max", "win-close", "garland", "leaves", "daisies"];
const ALL_VARS = ["--bg", "--rail", "--panel", "--panel-2", "--line", "--text", "--text-2", "--muted", "--row-line", "--scroll", "--switch-off",
  "--accent", "--accent-hover", "--accent-text", "--glow", "--heading", "--edge-light", "--edge-dark", "--logo-filter",
  "--tex-frame", "--tex-plate", "--tex-panel", ...ART.map((n) => `--art-${n}`)];
