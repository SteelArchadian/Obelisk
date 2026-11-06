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
  /** An effect over the modpack tiles (an experiment, user 2026-11-06): "none", or "crt" for a TV-like shimmer. */
  tileFx: "none" | "crt";
  /** Typing sounds (uisound.svelte.ts): a tiny tip per key in text boxes. */
  typingSounds: { on: boolean; volume: number };
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
  /** Each game starts with Iris shaders off (turn them on in-game). */
  shadersOffAtStart: boolean;
  /** Optional integrations, off until ticked in Settings → Plugins. */
  plugins: { lightning: boolean };
  /** The jingle on the first start after an update (sound.rs): one of JINGLES, "random" or "off". */
  updateJingle: string;
  /** Settings → Sound volumes are 0-100 on the scale soundGain gives. */
  jingleVolume: number;
  /** Background music (player.svelte.ts): Obelisk's own tracks (one of TRACKS or "all") or the user's own folder. */
  music: { on: boolean; source: "obelisk" | "mine"; track: string; folder: string; volume: number };
  /** Every Obelisk sound (music, update sound, alerts, menu sounds) holds back while another window is in front. */
  muteInactive: boolean;
  /** The social alert sounds (cues.rs): ids in `muted` stay silent; Do Not Disturb lets only invites through. */
  cues: { on: boolean; volume: number; dnd: boolean; muted: string[] };
  /** Menu clicks, hover ticks and open/close sounds (uisound.svelte.ts), in one of UI_SETS. */
  uiSounds: { on: boolean; volume: number; set: string };
  /** 2 since the quieter volume scale (2026-11-06); older saves get the new sound defaults once. */
  soundScale: number;
};

/** The update jingles sound.rs synthesizes, by setting id. */
export const JINGLES: [string, string][] = [
  ["glow", "Glow"], ["bubbles", "Bubbles"], ["marimba", "Marimba"],
];

/** Obelisk's own background music, which music.rs synthesizes, by setting id. */
export const TRACKS: [string, string][] = [["lounge", "Lounge"], ["shopping", "Window Shopping"]];

/** The social alert sounds cues.rs plays, by id, in its order. */
export const CUES: [string, string][] = [
  ["message", "New message"], ["message_sent", "Message sent"], ["mention", "Mention"], ["friend_request", "Friend request"],
  ["friend_accepted", "Friend request accepted"], ["friend_online", "Friend came online"], ["invite", "Game invite"],
  ["invite_accepted", "Invite accepted"], ["invite_declined", "Invite declined"], ["joined_world", "Joined a friend's world"],
  ["disconnected", "Disconnected"], ["reconnected", "Reconnected"], ["error", "Something went wrong"],
];

/** The menu sound sets uisound.svelte.ts synthesizes, by setting id. */
export const UI_SETS: [string, string][] = [["soft", "Soft"], ["glass", "Glass"], ["bubble", "Bubble"]];

/** A 0-100 volume setting as a gain. Squared, so the slider spreads the quiet end; 50% is what 7.5% used to be
 *  (user, 2026-11-06: the old 5-10% is the new 50%). Matches gain() in sound.rs. */
export const soundGain = (volume: number) => 0.3 * (Math.min(100, Math.max(0, volume)) / 100) ** 2;

const DEFAULTS: Settings = {
  theme: "obelisk",
  tileFx: "none",
  typingSounds: { on: true, volume: 50 },
  startPage: "library",
  launchers: { curseforge: true },
  extraDirs: [],
  connector: true,
  welcomed: false,
  trashHours: 336,
  packTrashHours: 336,
  backups: { auto: true, keepAuto: 10, keepManual: 20 },
  quietLaunch: true,
  shadersOffAtStart: true,
  closeToTray: true,
  startWithWindows: false,
  controlApi: false,
  aiAssistant: "claude",
  plugins: { lightning: false },
  updateJingle: "off", // staged: none plays until the user picks the update sound (user, 2026-11-06)
  jingleVolume: 50,
  music: { on: true, source: "obelisk", track: "all", folder: "", volume: 50 },
  muteInactive: true,
  cues: { on: true, volume: 50, dnd: false, muted: [] },
  uiSounds: { on: true, volume: 50, set: "soft" },
  soundScale: 2,
};

export const settings = $state<Settings>({ ...DEFAULTS });

export async function loadSettings() {
  const saved = await invoke<Partial<Settings>>("get_settings").catch(() => ({}));
  Object.assign(settings, DEFAULTS, saved);
  settings.plugins = { ...DEFAULTS.plugins, ...((saved as Partial<Settings>).plugins ?? {}) };
  settings.backups = { ...DEFAULTS.backups, ...((saved as Partial<Settings>).backups ?? {}) };
  settings.music = { ...DEFAULTS.music, ...((saved as Partial<Settings>).music ?? {}) };
  settings.typingSounds = { ...DEFAULTS.typingSounds, ...((saved as Partial<Settings>).typingSounds ?? {}) };
  settings.cues = { ...DEFAULTS.cues, ...((saved as Partial<Settings>).cues ?? {}) };
  settings.uiSounds = { ...DEFAULTS.uiSounds, ...((saved as Partial<Settings>).uiSounds ?? {}) };
  if (!((saved as Partial<Settings>).soundScale! >= 2)) {
    // The volume scale changed: take the new sound defaults once, keeping only the user's music folder.
    const { source, folder } = settings.music;
    settings.music = { ...DEFAULTS.music, source: folder ? source : "obelisk", folder };
    Object.assign(settings, { jingleVolume: 50, muteInactive: true, soundScale: 2 });
    settings.cues.volume = 50;
  }
  delete (settings.music as { focusOnly?: boolean }).focusOnly;
  if (!TRACKS.some((t) => t[0] === settings.music.track) && settings.music.track !== "all") settings.music.track = DEFAULTS.music.track;
  if (!JINGLES.some((j) => j[0] === settings.updateJingle) && !["random", "off"].includes(settings.updateJingle)) settings.updateJingle = DEFAULTS.updateJingle;
  if (!THEMES.some((t) => t.id === settings.theme)) settings.theme = "obelisk";
  applyTheme();
}

export async function saveSettings() {
  applyTheme();
  await invoke("set_settings", { settings: $state.snapshot(settings) });
}

export function applyTheme() {
  document.documentElement.dataset.theme = settings.theme;
  document.documentElement.dataset.tilefx = settings.tileFx ?? "none";
  document.documentElement.dataset.skin = isBlock(settings.theme) ? "block" : "flat";
  applyBlockTheme(settings.theme);
}
