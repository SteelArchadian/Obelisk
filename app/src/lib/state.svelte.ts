import type { Instance, ProjectType, Hit, Installed } from "./api";

export type Page = "library" | "instance" | "discover" | "browse" | "settings";
export type ProfileDialog = { kind: "options" | "clone" | "delete" | "purge"; inst: Instance; tab?: "options" | "backups" };
export type Overlay = { kind: "project"; hit: Hit } | { kind: "author"; name: string; cfId?: number };
/** `world` is the world folder a data pack goes into. */
export type PendingInstall = { hit: Hit; versionId?: string; loaderOverride?: string; world?: string };

/** App-wide navigation and selection state. */
export const app = $state({
  page: "library" as Page,
  instances: [] as Instance[],
  icons: {} as Record<string, string | null>,
  /** Instance shown on the instance page. */
  current: null as Instance | null,
  /**
   * Where installs go. Set by the path the user took (opening a profile, Add Content, or the
   * picker) and kept while they wander through project and author pages.
   */
  target: null as Instance | null,
  /** Modrinth projects in the target, keyed by project id (index metadata or file hash). */
  installed: new Map<string, Installed>(),
  installedFor: "",
  browseType: "mod" as ProjectType,
  browseQuery: "",
  /** Project/author pages opened on top of the current page; Back pops one. */
  stack: [] as Overlay[],
  /** An install waiting for the user to pick a profile (or confirm Connector). */
  picker: null as PendingInstall | null,
  /** Bumped to put the cursor in the top search bar. */
  focusSearch: 0,
  /** A data pack waiting for the user to pick a world. */
  worldPicker: null as PendingInstall | null,
  connector: null as PendingInstall | null,
  profileDialog: null as ProfileDialog | null,
  /** The modpack right-click menu (library tiles and the side rail share it). */
  packMenu: null as { x: number; y: number; inst: Instance } | null,
  /** Profile being turned into server + client packs. */
  serverPack: null as Instance | null,
  /** Profile whose Share dialog is open. */
  sharePack: null as Instance | null,
  /** Profile whose Share dialog (code or zip) is open. */
  packShare: null as Instance | null,
  /** Import-by-code dialog: the code typed so far, or null when closed. */
  packImport: null as string | null,
  /** Live share codes per profile, so reopening Share shows the running one. */
  packCodes: {} as Record<string, { code: string; expiresMs: number; files: number; custom: number; private?: number }>,
  /** Incoming changes from friends, per shared profile (for badges). */
  shareIncoming: {} as Record<string, number>,
  settingsSection: "general",
  toast: "",
  /** The toast is an error: tinted red, stays longer, closes with its ✕. */
  toastError: false,
});

let toastTimer: ReturnType<typeof setTimeout> | undefined;
export function toast(msg: string) {
  app.toast = msg;
  app.toastError = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (app.toast = ""), 5000);
}

/** Something went wrong: shown tinted red, long enough to read (20 s, or until its ✕). */
export function toastError(e: unknown) {
  app.toast = String(e);
  app.toastError = true;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (app.toast = ""), 20000);
}

export function closeToast() {
  clearTimeout(toastTimer);
  app.toast = "";
}

export function openBrowse(type: ProjectType, query = "") {
  app.stack = [];
  app.browseType = type;
  app.browseQuery = query;
  app.page = "browse";
}

/** Browse with the cursor already in the top search bar ("Add Content"). */
export function openBrowseTyping(type: ProjectType) {
  openBrowse(type);
  app.focusSearch++;
}

export const pushOverlay = (o: Overlay) => (app.stack = [...app.stack, o]);
export const popOverlay = () => (app.stack = app.stack.slice(0, -1));
export const closeOverlays = () => (app.stack = []);
