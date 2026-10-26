import { invoke } from "@tauri-apps/api/core";
import { BLOCK_THEMES, applyBlockTheme, blockTheme } from "./themes";

export type Theme = string;

export const THEMES: { id: Theme; label: string; group: string; swatch: string; light: boolean; blurb?: string }[] = [
  { id: "obelisk", label: "Obelisk", group: "Standard", swatch: "#9b87f5", light: false },
  { id: "light", label: "Obelisk Light", group: "Standard", swatch: "#7a64e8", light: true },
  ...BLOCK_THEMES.map((t) => ({ id: t.id, label: t.label, group: "Block", swatch: t.accent, light: false, blurb: t.blurb })),
];
/** Themes that use the block skin (cave background, pixel headings, bevels). */
export const isBlock = (t: Theme) => !!blockTheme(t);

type Settings = {
  theme: Theme;
  startPage: "library" | "discover";
  /** Launchers Sync looks at. */
  launchers: Record<string, boolean>;
  /** Extra folders that hold CurseForge-style instances. */
  extraDirs: string[];
  /** Offer Fabric builds on NeoForge through Sinytra Connector. */
  connector: boolean;
  /** The first-run welcome (CurseForge key how-to) has been seen. */
  welcomed: boolean;
  /** How long deleted content stays in a profile's trash. */
  trashHours: number;
  /** How long deleted modpacks stay in the modpack trash. */
  packTrashHours: number;
  /** Change-based backups: automatic ones at noon and midnight, plus checkpoints. */
  backups: { auto: boolean; keepAuto: number; keepManual: number };
  /** Closing the window keeps Obelisk running in the tray. */
  closeToTray: boolean;
  /** Start Obelisk in the tray when Windows starts. */
  startWithWindows: boolean;
  /** Local control API for the user's Minecraft chats (127.0.0.1, token, test bench copies only). */
  controlApi: boolean;
  /** Which assistant the Connect button sets up. */
  aiAssistant: "claude" | "gpt";
  /** Hide Prism Launcher's progress windows when starting a game. */
  quietLaunch: boolean;
  /** Optional integrations, off until ticked in Settings → Plugins. */
  plugins: { lightning: boolean };
};

const DEFAULTS: Settings = {
  theme: "obelisk",
  startPage: "library",
  launchers: { curseforge: true },
  extraDirs: [],
  connector: true,
  welcomed: false,
  trashHours: 336,
  packTrashHours: 336,
  backups: { auto: true, keepAuto: 10, keepManual: 20 },
  quietLaunch: true,
  closeToTray: true,
  startWithWindows: false,
  controlApi: false,
  aiAssistant: "claude",
  plugins: { lightning: false },
};

export const settings = $state<Settings>({ ...DEFAULTS });

export async function loadSettings() {
  const saved = await invoke<Partial<Settings>>("get_settings").catch(() => ({}));
  Object.assign(settings, DEFAULTS, saved);
  settings.plugins = { ...DEFAULTS.plugins, ...((saved as Partial<Settings>).plugins ?? {}) };
  settings.backups = { ...DEFAULTS.backups, ...((saved as Partial<Settings>).backups ?? {}) };
  if (!THEMES.some((t) => t.id === settings.theme)) settings.theme = "obelisk";
  applyTheme();
}

export async function saveSettings() {
  applyTheme();
  await invoke("set_settings", { settings: $state.snapshot(settings) });
}

export function applyTheme() {
  document.documentElement.dataset.theme = settings.theme;
  document.documentElement.dataset.skin = isBlock(settings.theme) ? "block" : "flat";
  applyBlockTheme(settings.theme);
}
