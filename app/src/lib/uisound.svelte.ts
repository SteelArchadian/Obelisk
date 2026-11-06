// Menu sounds (Settings → Sound → Menu sounds): a hover tick, a click, a rising "zoom in" when a modpack or project page
// opens, a falling one when it closes, and a two-note blip on tabs and pages. Small console-menu sounds in spirit (user,
// 2026-11-06: "like the DS / Wii menus"), all synthesized here with WebAudio, so every sound is original and nothing ships.
// Typing sounds too (user: "super light, minimal, and playful"), with their own switch and volume.
import { settings, soundGain } from "./settings.svelte";
import { app } from "./state.svelte";

export type UiSound = "hover" | "click" | "open" | "close" | "tab";

let ctx: AudioContext | null = null;
let lastHover: Element | null = null;
let hoverAt = 0;
let navAt = 0;

/** What counts as something to point at or press. */
const PRESSABLE = 'button, a[href], [role="tab"], [role="switch"], [role="menuitem"], select, input[type="checkbox"], input[type="radio"]';

/** One synthesized tone: a sine (or triangle) gliding from f0 to f1 Hz, optionally with an FM bell partial. */
type Tone = { at?: number; f0: number; f1?: number; dur: number; vol: number; wave?: OscillatorType; fm?: number };

/** Each set's five sounds, as tones. Levels are relative: hover is the faintest, open the fullest. */
const SETS: Record<string, Record<UiSound, Tone[]>> = {
  // Rounded sine blips, like a handheld's menu.
  soft: {
    hover: [{ f0: 1760, dur: 0.035, vol: 0.25 }],
    click: [{ f0: 1175, dur: 0.06, vol: 0.55 }, { at: 0.03, f0: 1760, dur: 0.07, vol: 0.4 }],
    open: [{ f0: 520, f1: 1560, dur: 0.14, vol: 0.45, wave: "triangle" }, { at: 0.12, f0: 2093, dur: 0.25, vol: 0.35 }],
    close: [{ f0: 1400, f1: 470, dur: 0.15, vol: 0.45, wave: "triangle" }],
    tab: [{ f0: 1319, dur: 0.05, vol: 0.4 }, { at: 0.045, f0: 1976, dur: 0.08, vol: 0.35 }],
  },
  // Little glass bells (two-operator FM), like a console's home menu.
  glass: {
    hover: [{ f0: 2637, dur: 0.06, vol: 0.18, fm: 3.5 }],
    click: [{ f0: 1568, dur: 0.18, vol: 0.4, fm: 3.5 }],
    open: [{ f0: 1047, dur: 0.3, vol: 0.3, fm: 3.5 }, { at: 0.06, f0: 1568, dur: 0.3, vol: 0.3, fm: 3.5 }, { at: 0.12, f0: 2093, dur: 0.45, vol: 0.32, fm: 3.5 }],
    close: [{ f0: 1568, dur: 0.25, vol: 0.3, fm: 3.5 }, { at: 0.07, f0: 1047, dur: 0.35, vol: 0.3, fm: 3.5 }],
    tab: [{ f0: 1760, dur: 0.15, vol: 0.3, fm: 3.5 }, { at: 0.05, f0: 2349, dur: 0.2, vol: 0.25, fm: 3.5 }],
  },
  // Water-drop pops: quick pitch bends, up for in, down for out.
  bubble: {
    hover: [{ f0: 900, f1: 1500, dur: 0.03, vol: 0.25 }],
    click: [{ f0: 500, f1: 1300, dur: 0.06, vol: 0.55 }],
    open: [{ f0: 400, f1: 1100, dur: 0.07, vol: 0.45 }, { at: 0.07, f0: 600, f1: 1700, dur: 0.08, vol: 0.45 }],
    close: [{ f0: 1500, f1: 600, dur: 0.07, vol: 0.45 }, { at: 0.07, f0: 1100, f1: 400, dur: 0.08, vol: 0.45 }],
    tab: [{ f0: 700, f1: 1400, dur: 0.05, vol: 0.45 }],
  },
};

/** Plays one menu sound, if menu sounds are on and the window is in front (or the user is previewing in Settings). */
export function uiSound(kind: UiSound, set = settings.uiSounds.set, force = false) {
  if (!force && (!settings.uiSounds.on || (settings.muteInactive && !document.hasFocus()))) return;
  // Menu sounds sit a little under the other sounds at the same slider position.
  playTones((SETS[set] ?? SETS.soft)[kind], soundGain(settings.uiSounds.volume) * 0.6);
}

function playTones(tones: Tone[], gain: number) {
  try {
    ctx ??= new AudioContext();
    if (ctx.state === "suspended") void ctx.resume();
    const out = ctx.createGain();
    out.gain.value = gain;
    out.connect(ctx.destination);
    const now = ctx.currentTime + 0.005;
    for (const t of tones) tone(ctx, out, now + (t.at ?? 0), t);
  } catch { /* no audio device: stay silent */ }
}

export type TypeSound = "key" | "space" | "erase" | "enter";

/** Pentatonic notes for letters: each key always gets the same one, so typing a word plays its own little tune. */
const KEY_NOTES = [1568, 1760, 1976, 2349, 2637];

function typeTones(kind: TypeSound, key: string): Tone[] {
  switch (kind) {
    case "key": return [{ f0: KEY_NOTES[key.charCodeAt(0) % KEY_NOTES.length], dur: 0.022, vol: 0.3 }];
    case "space": return [{ f0: 880, dur: 0.03, vol: 0.28, wave: "triangle" }];
    case "erase": return [{ f0: 1400, f1: 900, dur: 0.03, vol: 0.25 }];
    case "enter": return [{ f0: 1319, dur: 0.035, vol: 0.28 }, { at: 0.04, f0: 1976, dur: 0.05, vol: 0.25 }];
  }
}

/** One typing sound, if typing sounds are on and the window is in front (or the user is previewing in Settings). */
export function typeSound(kind: TypeSound, key = "a", force = false) {
  if (!force && (!settings.typingSounds.on || (settings.muteInactive && !document.hasFocus()))) return;
  // Lighter still than the menu sounds: a keystroke is the most frequent sound there is.
  playTones(typeTones(kind, key), soundGain(settings.typingSounds.volume) * 0.4);
}

/** Where typing makes sound: text boxes, never password fields (their rhythm shouldn't be heard). */
function typingInto(el: EventTarget | null) {
  if (!(el instanceof HTMLElement)) return false;
  if (el instanceof HTMLTextAreaElement) return true;
  if (el instanceof HTMLInputElement) return ["text", "search", "email", "url", "number", "tel", ""].includes(el.type);
  return el.isContentEditable;
}

let typedAt = 0;

function tone(ctx: AudioContext, out: AudioNode, at: number, t: Tone) {
  const osc = ctx.createOscillator();
  const env = ctx.createGain();
  osc.type = t.wave ?? "sine";
  osc.frequency.setValueAtTime(t.f0, at);
  if (t.f1) osc.frequency.exponentialRampToValueAtTime(t.f1, at + t.dur * 0.8);
  // A 4 ms attack (no click), then an exponential ring-out over the length.
  env.gain.setValueAtTime(0.0001, at);
  env.gain.exponentialRampToValueAtTime(t.vol, at + 0.004);
  env.gain.exponentialRampToValueAtTime(0.0001, at + t.dur);
  osc.connect(env).connect(out);
  if (t.fm) {
    const mod = ctx.createOscillator();
    const depth = ctx.createGain();
    mod.frequency.value = t.f0 * t.fm;
    depth.gain.setValueAtTime(t.f0 * 1.2, at);
    depth.gain.exponentialRampToValueAtTime(1, at + t.dur);
    mod.connect(depth).connect(osc.frequency);
    mod.start(at);
    mod.stop(at + t.dur + 0.02);
  }
  osc.start(at);
  osc.stop(at + t.dur + 0.02);
}

/** Hover ticks, clicks on anything pressable and typing sounds, for the whole window. A click that opens or closes something plays
 *  that sound instead (navSound runs first, from the state change the click made). Returns the cleanup. */
export function watchUi() {
  const pressable = (e: Event) => {
    const el = (e.target as Element | null)?.closest?.(PRESSABLE);
    return el && !(el as HTMLButtonElement).disabled && el.getAttribute("aria-disabled") !== "true" ? el : null;
  };
  const over = (e: PointerEvent) => {
    const el = pressable(e);
    if (el === lastHover) return;
    lastHover = el;
    if (!el || el.matches("select, input") || performance.now() - hoverAt < 40) return;
    hoverAt = performance.now();
    uiSound("hover");
  };
  const click = (e: MouseEvent) => {
    // Buttons marked data-quiet play a sound of their own (previews in Settings → Sound).
    if (!pressable(e) || (e.target as Element).closest("[data-quiet]")) return;
    const at = performance.now();
    setTimeout(() => { if (navAt < at) uiSound("click"); }, 0);
  };
  const key = (e: KeyboardEvent) => {
    // A held key repeats without sound, shortcuts stay quiet, and fast typing gets at most one tick per 30 ms, so the
    // sounds never pile up.
    if (e.repeat || e.isComposing || e.ctrlKey || e.metaKey || e.altKey || !typingInto(e.target)) return;
    const kind: TypeSound | null = e.key === " " ? "space" : e.key === "Backspace" || e.key === "Delete" ? "erase"
      : e.key === "Enter" ? "enter" : e.key.length === 1 ? "key" : null;
    if (!kind || performance.now() - typedAt < 30) return;
    typedAt = performance.now();
    typeSound(kind, e.key);
  };
  document.addEventListener("pointerover", over, { passive: true });
  document.addEventListener("click", click, { capture: true, passive: true });
  document.addEventListener("keydown", key, { capture: true, passive: true });
  return () => {
    document.removeEventListener("pointerover", over);
    document.removeEventListener("click", click, { capture: true });
    document.removeEventListener("keydown", key, { capture: true });
  };
}

let seen: { page: string; stack: number; section: string } | null = null;

/** Call from an $effect: opening a modpack or a project page zooms in, leaving it zooms out, other pages and Settings
 *  sections blip like tabs. */
export function navSound() {
  const now = { page: app.page, stack: app.stack.length, section: app.settingsSection };
  const was = seen;
  seen = now;
  if (!was) return;
  let kind: UiSound | null = null;
  if (now.stack > was.stack || (now.page === "instance" && was.page !== "instance")) kind = "open";
  else if (now.stack < was.stack || (was.page === "instance" && now.page !== "instance")) kind = "close";
  else if (now.page !== was.page || now.section !== was.section) kind = "tab";
  if (!kind) return;
  navAt = performance.now();
  uiSound(kind);
}
