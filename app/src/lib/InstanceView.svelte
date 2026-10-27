<script lang="ts">
  import { crisp } from "./crisp";
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { ago, api, initials, type FileInfo, type Item, type Kind, type World } from "./api";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { cached, recheck, updateState, updatesFor } from "./updates.svelte";
  import { settings } from "./settings.svelte";
  import { kill, play as startGame, running, starting } from "./play.svelte";
  import ConfigEditor from "./ConfigEditor.svelte";
  import Diagnosis from "./Diagnosis.svelte";
  import Icon from "./Icon.svelte";
  import { app, openBrowseTyping, pushOverlay, toast } from "./state.svelte";
  import { openProjectById, refreshInstalled, setTarget } from "./install";
  import { manualDownload } from "./cf";
  import { checkDupes, dupeState, flagged, ignoreDupes } from "./dupes.svelte";

  type Sub = Kind | "worlds";
  type SortKey = "name" | "date" | "authors" | "source" | "enabled" | "env" | "update";
  type TrashEntry = { id: string; kind: Kind; fileName: string; name: string; version: string; authors: string; source: Item["source"]; deletedMs: number };

  const KINDS: { kind: Kind; label: string }[] = [
    { kind: "mods", label: "Mods" },
    { kind: "resourcepacks", label: "Resource Packs" },
    { kind: "shaderpacks", label: "Shaders" },
  ];
  const SOURCE_LABEL = { curseforge: "CurseForge", modrinth: "Modrinth", local: "Local" };

  let current = $derived(app.current!);
  let content = $state<Record<Kind, Item[]>>({ mods: [], resourcepacks: [], shaderpacks: [] });
  let icons = $state<Record<string, string>>({});
  let worlds = $state<World[]>([]);
  let logs = $state<string[]>([]);
  let logName = $state("");
  let logText = $state("");
  let diagFile = $state("");
  /** The header's copy button: the open log, or the crash report/log shown in Diagnosis. */
  async function copyLog() {
    try {
      const text = page === "logs"
        ? (logName ? logText : await invoke<string>("read_report", { instanceId: current.id, file: "logs/latest.log" }))
        : await invoke<string>("read_report", { instanceId: current.id, file: diagFile || "logs/latest.log" });
      if (!text) return toast("Nothing to copy yet.");
      await navigator.clipboard.writeText(text);
      toast(`Copied ${page === "logs" && logName ? logName : (diagFile || "logs/latest.log").split("/").pop()} (${Math.round(text.length / 1024)} KB).`);
    } catch (e) { toast(String(e)); }
  }

  let page = $state<"content" | "logs" | "diagnosis">("content");
  /** A crash newer than the last time the Diagnosis tab was opened. */
  let newCrash = $state(false);
  /** The same mod installed twice (usually once from each site): the game won't start like that. */
  let dupes = $derived(dupeState[current.id]?.dupes ?? []);
  /** "Not now" hides the warning until the profile is opened again; Play stays blocked. */
  let dupesHiddenFor = $state("");
  let dupeBusy = $state(false);
  const SITE: Record<string, string> = { modrinth: "Modrinth", curseforge: "CurseForge", local: "added by hand" };
  const dupeItem = (file: string) => content.mods.find((i) => i.fileName === file);
  /** Move one copy to the profile's trash (restorable from there). */
  async function removeDupe(file: string) {
    const id = current.id;
    const it = dupeItem(file);
    dupeBusy = true;
    let failed = "";
    try {
      await invoke("delete_content", { instanceId: id, kind: "mods", fileName: file, name: it?.name ?? file, version: it?.version ?? "", authors: it?.authors ?? "", source: it?.source ?? "local" });
      toast(`Moved ${file} to the trash.`);
    } catch (e) {
      failed = String(e);
    }
    await load(id);
    await checkDupes(id);
    error = failed;
    dupeBusy = false;
  }
  /** Pinned content: Update all leaves it alone ("provider:projectId", or the file name). */
  let pins = $state<Set<string>>(new Set());
  // ---- hopper filters
  type Filters = { authors: string[]; sources: string[]; envs: string[]; status: string[]; noLibs?: boolean };
  const NO_FILTERS: Filters = { authors: [], sources: [], envs: [], status: [], noLibs: false };
  let filters = $state<Filters>({ ...NO_FILTERS });
  let showFilters = $state(false);
  // Optional columns: right-click the header to show or hide them (remembered). Hiding one also
  // drops its filters, so a filter can't silently hide rows you can't see the reason for.
  const COLUMNS = [["date", "Installed / Modified"], ["authors", "Author"], ["env", "Client / server"], ["source", "Source"], ["update", "Action"], ["enabled", "Active"]] as const;
  const WIDTH: Record<string, string> = { date: "120px", authors: "minmax(90px, 1fr)", env: "54px", source: "100px", update: "84px", enabled: "60px" };
  let hiddenCols = $state<string[]>((() => { try { return JSON.parse(localStorage.getItem("obelisk.hiddenCols") ?? "[]"); } catch { return []; } })());
  const shown = (c: string) => !hiddenCols.includes(c);
  let colMenu = $state<{ x: number; y: number } | null>(null);
  function toggleCol(c: string) {
    hiddenCols = shown(c) ? [...hiddenCols, c] : hiddenCols.filter((h) => h !== c);
    if (c === "env") filters = { ...filters, envs: [] };
    if (c === "source") filters = { ...filters, sources: [] };
    try { localStorage.setItem("obelisk.hiddenCols", JSON.stringify(hiddenCols)); } catch { /* ignore */ }
  }
  // A narrow window drops columns (least useful first) until the rest fit; they come back when it widens.
  const MIN_W: Record<string, number> = { date: 120, authors: 90, env: 54, source: 100, update: 84, enabled: 60 };
  const DROP_ORDER = ["authors", "date", "source", "env", "enabled"];
  let tableW = $state(0);
  let squeezed = $derived.by(() => {
    if (!tableW) return [] as string[];
    let need = 32 + 22 + 230 + COLUMNS.filter(([c]) => shown(c)).reduce((n, [c]) => n + MIN_W[c] + 14, 14);
    const out: string[] = [];
    for (const c of DROP_ORDER) {
      if (need <= tableW) break;
      if (shown(c)) { out.push(c); need -= MIN_W[c] + 14; }
    }
    return out;
  });
  const vis = (c: string) => shown(c) && !squeezed.includes(c);
  let colTemplate = $derived(["22px", "minmax(180px, 3fr)", ...COLUMNS.filter(([c]) => vis(c)).map(([c]) => WIDTH[c])].join(" "));
  let filterCount = $derived(filters.authors.length + filters.sources.length + filters.envs.length + filters.status.length + (filters.noLibs ? 1 : 0));
  let sub = $state<Sub>("mods");
  let query = $state("");
  let sortKey = $state<SortKey>("name");
  let sortAsc = $state(true);
  let selected = $state<Set<string>>(new Set());
  let confirmDelete = $state<Item[] | null>(null);
  let menu = $state<{ item: Item; x: number; y: number } | null>(null);
  let configsFor = $state<Item | null>(null);
  let trash = $state<TrashEntry[]>([]);
  /** Show this profile's trash in place of the content list. */
  let viewTrash = $state(false);
  /** Date column: when it was installed or last modified. */
  let dateMode = $state<"installed" | "modified">((localStorage.getItem("obelisk.dateMode") as any) ?? "installed");
  let dateMenu = $state(false);
  let error = $state("");
  /** Per-file source, popularity, sides and available update ("kind/fileName" -> info). */
  let infos = $state<Map<string, FileInfo>>(new Map());
  let checking = $state(false);
  let updating = $state<Set<string>>(new Set());
  const plain = (f: string) => f.replace(/\.disabled$/, "");
  const infoOf = (i: Item) => infos.get(`${kind}/${plain(i.fileName)}`);
  const ENV_LABEL: Record<string, string> = { client: "Client", server: "Server", both: "Client & server", "": "Unknown" };

  let kind = $derived<Kind>(sub === "worlds" ? "mods" : sub);
  let rows = $derived.by(() => {
    const q = query.trim().toLowerCase();
    let list: Item[] = viewTrash
      ? trash.filter((t) => t.kind === kind).map((t) => ({
          fileName: t.fileName, name: t.name, version: t.version, authors: t.authors, source: t.source || "local",
          enabled: true, projectId: "", installedMs: t.deletedMs, modifiedMs: t.deletedMs, trashId: t.id,
        }))
      : content[kind];
    if (q) list = list.filter((i) => `${i.name} ${i.fileName} ${i.authors}`.toLowerCase().includes(q));
    if (filterCount) list = list.filter(passes);
    const dir = sortAsc ? 1 : -1;
    return [...list].sort((a, b) => {
      const val = (i: Item): string | number | boolean => {
        const f = infoOf(i);
        if (sortKey === "env") return f?.env || "~";
        // Source: the one shown on the badge, with the chosen source first and the rest in a fixed order.
        if (sortKey === "source") {
          const src = i.source === "local" && f?.provider ? f.provider : i.source;
          return (SOURCE_ORDER.indexOf(src) - SOURCE_ORDER.indexOf(sourceFirst) + SOURCE_ORDER.length) % SOURCE_ORDER.length;
        }
        if (sortKey === "update") return f?.update ? 0 : 1;
        if (sortKey === "date") return dateMode === "modified" ? i.modifiedMs : i.installedMs;
        return i[sortKey];
      };
      const x = val(a), y = val(b);
      const c = typeof x === "boolean" ? Number(y) - Number(x) : typeof x === "number" ? x - (y as number)
        : String(x).localeCompare(String(y), undefined, { numeric: true, sensitivity: "base" });
      return (c || a.name.localeCompare(b.name)) * dir;
    });
  });
  let allChecked = $derived(rows.length > 0 && rows.every((r) => selected.has(r.fileName)));
  let pending = $derived([...infos.values()].filter((f) => f.update && !pins.has(pinKey(f))));
  let pendingHere = $derived(pending.filter((f) => f.kind === kind));
  let total = $derived(content.mods.length + content.resourcepacks.length + content.shaderpacks.length);

  let currentId = $derived(app.current!.id);
  $effect(() => {
    load(currentId);
  });

  // Opening a profile makes it where installs go until the user picks something else.
  $effect(() => {
    if (app.target?.id !== current.id) setTarget(current);
  });

  /** Open a row's project page: Prism metadata first, else the hash match against Modrinth. */
  function openItem(item: Item) {
    if (item.source === "modrinth" && item.projectId) return openProjectById(item.projectId);
    const f = infoOf(item);
    if (f?.provider === "modrinth") return openProjectById(f.projectId);
    if (f?.provider === "curseforge") return openProjectById(`cf:${f.projectId}`);
    for (const [id, inst] of app.installed) {
      if (inst.kind === kind && inst.fileName === item.fileName) return openProjectById(id);
    }
    toast(app.installedFor === current.id
      && !checking ? `${item.name} isn't on Modrinth or CurseForge (add a CurseForge key in Settings to check there).`
      : "Still matching your files. Try again in a moment.");
  }

  async function load(id: string) {
    query = "";
    error = "";
    selected = new Set();
    logName = "";
    logText = "";
    const results = await Promise.all(KINDS.map(({ kind }) => invoke<Item[]>("list_content", { instanceId: id, kind })));
    content = { mods: results[0], resourcepacks: results[1], shaderpacks: results[2] };
    dupesHiddenFor = "";
    checkDupes(id);
    invoke<string[]>("get_pins", { instanceId: id }).then((p) => { if (current.id === id) pins = new Set(p); }).catch(() => {});
    invoke<{ kind: string; timeMs: number }[]>("crash_reports", { instanceId: id }).then((r) => {
      const newest = Math.max(0, ...r.filter((x) => x.kind !== "log").map((x) => x.timeMs));
      let seen = 0;
      try { seen = Number(localStorage.getItem(`obelisk.seenCrash.${id}`) ?? 0); } catch { /* ignore */ }
      if (current.id === id) newCrash = newest > seen;
    }).catch(() => {});
    [worlds, logs, trash] = await Promise.all([
      invoke<World[]>("list_worlds", { instanceId: id }),
      invoke<string[]>("list_logs", { instanceId: id }),
      invoke<TrashEntry[]>("list_trash", { instanceId: id }),
    ]);
    // Icons can take a moment on first run (extracted from jars), so they stream in afterwards.
    const maps = await Promise.all(KINDS.map(({ kind }) =>
      invoke<Record<string, string>>("content_icons", { instanceId: id, kind })));
    const next: Record<string, string> = {};
    for (const m of maps) for (const [f, p] of Object.entries(m)) next[f] = convertFileSrc(p);
    if (current.id === id) icons = next;
    showUpdates(id);
  }

  /**
   * Show the last update check. Opening a profile never re-checks: that happens quietly in the
   * background (on launch, every 12 hours, after installs/updates/deletes, or Ctrl+R).
   */
  async function showUpdates(id: string) {
    const c = cached(id);
    if (c) return void (infos = toMap(c.infos));
    checking = true; // first time this profile is seen
    try {
      const list = await updatesFor(id);
      if (current.id === id) infos = toMap(list);
    } catch (e) {
      if (current.id === id) error = `Couldn't check for updates: ${e}`;
    }
    if (current.id === id) checking = false;
  }
  const toMap = (list: FileInfo[]) => new Map(list.map((f) => [`${f.kind}/${plain(f.fileName)}`, f]));

  // Background checks finished: pick up their results.
  $effect(() => {
    void updateState.tick;
    const c = cached(currentId);
    if (c) infos = toMap(c.infos);
  });

  /** Files changed here: refresh update info without showing a spinner. */
  const quietRecheck = (id: string) => void recheck(id).catch(() => {});

  /** Replace one file with its newest compatible build. Blocked CurseForge files go via the browser. */
  async function update(f: FileInfo, reload = true) {
    const key = `${f.kind}/${f.fileName}`;
    updating = new Set(updating).add(key);
    const id = current.id;
    try {
      const job = await api.applyUpdate(id, f);
      if (job) await manualDownload(id, job, () => { if (current.id === id) load(id); quietRecheck(id); });
      // Done: drop the button straight away; the background re-check fills in the rest.
      else infos = new Map([...infos].map(([k, v]) => [k, k === key ? { ...v, update: null } : v]));
    } catch (e) {
      toast(`${f.fileName}: ${e}`);
    }
    const next = new Set(updating);
    next.delete(key);
    updating = next;
    if (reload) {
      if (current.id === id) await load(id);
      quietRecheck(id);
    }
  }

  async function updateAll(list: FileInfo[]) {
    const id = current.id;
    // Updating many mods at once is easy to regret: a checkpoint first (kept like the ones you make yourself, not
    // pruned with the automatic backups), and no update at all if it can't be taken.
    try {
      await invoke("create_checkpoint", { instanceId: id, label: "Pre-Update all" });
    } catch (e) {
      toast(`Update all stopped: the checkpoint before it couldn't be made (${e}).`);
      return;
    }
    // Direct downloads first; files that need the browser are opened one at a time afterwards.
    list = list.filter((f) => !pins.has(pinKey(f)));
    const direct = list.filter((f) => !f.update!.blocked);
    const blocked = list.filter((f) => f.update!.blocked);
    for (const f of direct) await update(f, false);
    if (blocked.length) {
      toast(`${blocked.length} update${blocked.length > 1 ? "s" : ""} must be downloaded from curseforge.com. Opening the first one.`);
      await update(blocked[0], false);
    }
    if (current.id === id) {
      await load(id);
      refreshInstalled();
      quietRecheck(id);
      if (direct.length) toast(`Updated ${direct.length} file${direct.length === 1 ? "" : "s"}.`);
    }
  }

  function setSub(s: Sub) {
    sub = s;
    viewTrash = false;
    query = "";
    selected = new Set();
  }

  // Clicking Source again rotates which source comes first (Local, CurseForge, Modrinth) instead of
  // flipping the order, since "reverse" means little for three groups.
  const SOURCE_ORDER = ["local", "curseforge", "modrinth"];
  let sourceFirst = $state("local");
  function sortBy(key: SortKey) {
    if (key === "source") {
      if (sortKey === "source") sourceFirst = SOURCE_ORDER[(SOURCE_ORDER.indexOf(sourceFirst) + 1) % SOURCE_ORDER.length];
      else { sortKey = "source"; sourceFirst = "local"; }
      sortAsc = true;
      return;
    }
    if (sortKey === key) sortAsc = !sortAsc;
    else { sortKey = key; sortAsc = true; }
  }

  function toggleRow(file: string) {
    const s = new Set(selected);
    s.has(file) ? s.delete(file) : s.add(file);
    selected = s;
  }
  function toggleAll() {
    selected = allChecked ? new Set() : new Set(rows.map((r) => r.fileName));
  }

  /** The key a pin is stored under: the project when known (survives updates), else the file. */
  function pinKey(f: FileInfo | undefined, item?: Item) {
    if (f?.provider && f.projectId) return `${f.provider}:${f.projectId}`;
    return (item?.fileName ?? f?.fileName ?? "").replace(/\.disabled$/, "");
  }
  const isPinned = (item: Item) => pins.has(pinKey(infoOf(item), item));
  async function setPinned(items: Item[], want: boolean) {
    const next = new Set(pins);
    for (const it of items) want ? next.add(pinKey(infoOf(it), it)) : next.delete(pinKey(infoOf(it), it));
    pins = next;
    try {
      await invoke("set_pins", { instanceId: current.id, pins: [...next] });
      toast(want ? `Pinned ${items.length === 1 ? items[0].name : `${items.length} items`}. Update all will skip ${items.length === 1 ? "it" : "them"}.` : `Unpinned ${items.length === 1 ? items[0].name : `${items.length} items`}.`);
    } catch (e) {
      toast(String(e));
    }
  }

  // ---- hopper filters
  /** A library or API: as the site files it, else by name for mods neither site knows. */
  const isLibrary = (i: Item) => {
    const f = infoOf(i);
    return f?.provider ? !!f.library : /(lib|library)\b|\bapi\b/i.test(`${i.name} ${i.fileName.replace(/[-_.]/g, " ")}`);
  };

  function passes(i: Item) {
    const f = infoOf(i);
    if (filters.noLibs && kind === "mods" && isLibrary(i)) return false;
    if (filters.authors.length && !filters.authors.some((a) => authorsOf(i).includes(a))) return false;
    if (filters.sources.length) {
      const src = i.source === "local" && f?.provider ? f.provider : i.source;
      if (!filters.sources.includes(src)) return false;
    }
    if (filters.envs.length && !filters.envs.includes(f?.env || "unknown")) return false;
    if (filters.status.length) {
      const st = new Set<string>([i.enabled ? "enabled" : "disabled"]);
      if (f?.update && !isPinned(i)) st.add("update");
      if (isPinned(i)) st.add("pinned");
      if (!filters.status.some((s) => st.has(s))) return false;
    }
    return true;
  }
  const authorsOf = (i: Item) => i.authors.split(/,\s*|\s+and\s+/).map((a) => a.trim()).filter(Boolean);
  function toggleFilter(group: Exclude<keyof Filters, "noLibs">, value: string) {
    const cur = filters[group];
    filters = { ...filters, [group]: cur.includes(value) ? cur.filter((v) => v !== value) : [...cur, value] };
  }
  function sortFrom(v: string) {
    const [key, dir] = v.split(":");
    if (key === "installed" || key === "modified") { dateMode = key; sortKey = "date"; }
    else sortKey = key as SortKey;
    sortAsc = dir === "asc";
  }

  async function setEnabled(item: Item, want: boolean) {
    if (item.enabled === want) return;
    const old = item.fileName;
    item.enabled = want; // optimistic; reverted if the rename fails
    try {
      item.fileName = await invoke<string>("set_enabled", { instanceId: current.id, kind, fileName: old, enabled: want });
      if (icons[old]) icons[item.fileName] = icons[old];
      if (selected.has(old)) { selected.delete(old); selected.add(item.fileName); selected = new Set(selected); }
    } catch (e) {
      item.enabled = !want;
      await stale(e, item);
    }
  }

  async function bulkEnable(want: boolean) {
    for (const item of content[kind].filter((i) => selected.has(i.fileName))) await setEnabled(item, want);
  }

  /**
   * Re-read the content lists only (keeps search and selection). Files change behind Obelisk's back,
   * e.g. a newer jar dropped in by hand, so this runs when the window comes back into focus too.
   */
  async function refreshContent(id: string) {
    try {
      const results = await Promise.all(KINDS.map(({ kind }) => invoke<Item[]>("list_content", { instanceId: id, kind })));
      if (current.id !== id) return;
      content = { mods: results[0], resourcepacks: results[1], shaderpacks: results[2] };
      const names = new Set(results.flat().map((i) => i.fileName));
      selected = new Set([...selected].filter((f) => names.has(f)));
      checkDupes(id);
    } catch { /* keep what is shown */ }
  }
  $effect(() => {
    const id = current.id;
    const onFocus = () => refreshContent(id);
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  });
  /** A file in the list no longer exists: show what's really there. */
  async function stale(e: unknown, item: Item) {
    await refreshContent(current.id);
    error = String(e).includes("not found")
      ? `${item.fileName} isn't in the folder any more (it was probably replaced by another version). The list is refreshed; try again.`
      : String(e);
  }

  async function doDelete() {
    if (!confirmDelete) return;
    const doomed = confirmDelete;
    confirmDelete = null;
    for (const item of doomed) {
      try {
        await invoke("delete_content", { instanceId: current.id, kind, fileName: item.fileName, name: item.name, version: item.version, authors: item.authors, source: item.source });
        content[kind] = content[kind].filter((i) => i !== item);
        selected.delete(item.fileName);
      } catch (e) {
        await stale(e, item);
      }
    }
    selected = new Set(selected);
    trash = await invoke<TrashEntry[]>("list_trash", { instanceId: current.id });
    quietRecheck(current.id);
    toast(`Moved to this profile's trash. Restore it within ${trashLabel(settings.trashHours)}.`);
  }

  async function restoreIds(ids: string[]) {
    let ok = 0;
    for (const id of ids) {
      try {
        await invoke("restore_trash", { instanceId: current.id, id });
        ok++;
      } catch (e) {
        toast(String(e));
      }
    }
    if (ok) toast(`Restored ${ok} item${ok === 1 ? "" : "s"}.`);
    selected = new Set();
    await load(current.id);
    if (ok) quietRecheck(current.id);
  }

  async function removeIds(ids: string[]) {
    if (!confirm(`Delete ${ids.length} item${ids.length === 1 ? "" : "s"} for good? This can't be undone.`)) return;
    for (const id of ids) await invoke("remove_trash", { instanceId: current.id, id }).catch((e) => toast(String(e)));
    selected = new Set();
    trash = await invoke<TrashEntry[]>("list_trash", { instanceId: current.id });
  }

  const kindLabel = (k: Kind) => KINDS.find((x) => x.kind === k)?.label.replace(/s$/, "") ?? k;
  const trashLabel = (h: number) => (h % 24 === 0 ? `${h / 24} day${h === 24 ? "" : "s"}` : `${h} hours`);
  const until = (ms: number) => {
    const left = ms + settings.trashHours * 3_600_000 - Date.now();
    const h = Math.max(0, Math.round(left / 3_600_000));
    return h >= 48 ? `in ${Math.round(h / 24)} days` : `in ${h} hour${h === 1 ? "" : "s"}`;
  };

  /** The author's page: Modrinth owner inside Obelisk, CurseForge author in the browser. */
  async function openAuthor(item: Item) {
    const f = infoOf(item);
    try {
      if (f?.provider === "modrinth") {
        const m = await api.members(f.projectId);
        const owner = m.find((x) => x.role === "Owner") ?? m[0];
        if (owner) return pushOverlay({ kind: "author", name: owner.user.username });
      } else if (f?.provider === "curseforge") {
        const { project } = await api.cfProject(Number(f.projectId));
        const a = project.authors?.[0];
        if (a?.id) return pushOverlay({ kind: "author", name: a.name, cfId: a.id });
      }
      toast(`No author page found for ${item.name}.`);
    } catch (e) {
      toast(String(e));
    }
  }

  async function openLog(name: string) {
    logName = name;
    logText = await invoke<string>("read_log", { instanceId: current.id, name });
  }

  const play = () => startGame(current);
  // Stop asks once more, since a force-close loses anything since the last autosave.
  let confirmKill = $state(false);
  function stop() {
    if (!confirmKill) { confirmKill = true; setTimeout(() => (confirmKill = false), 4000); return; }
    confirmKill = false;
    kill(current);
  }

  const reveal = (subPath: string) => invoke("reveal", { instanceId: current.id, sub: subPath });

  function hours(s: number) {
    return s < 3600 ? `${Math.round(s / 60)} min` : `${Math.round(s / 3600)} hours`;
  }
</script>

<svelte:window onclick={() => { menu = null; dateMenu = false; showFilters = false; colMenu = null; }} onkeydown={(e) => e.key === "Escape" && (menu = null)} onscroll={() => (menu = null)} />

<div class="instance">
  <section class="hero">
    <div class="hero-icon">
      {#if app.icons[current.id]}<img src={app.icons[current.id]} alt="" />{:else}<span>{initials(current.name)}</span>{/if}
    </div>
    <div class="hero-body">
      <div class="hero-title">
        <div>
          <h2>{current.name}</h2>
          <p class="sub">Prism instance · {current.id}</p>
        </div>
        <div class="hero-actions">
          <button class="ghost" title="Share this pack: a code for a friend, or a zip" onclick={() => (app.packShare = current)}>⇪ Share</button>
          <button class="ghost" title="Profile options" onclick={() => (app.profileDialog = { kind: "options", inst: current })}>⚙ Options</button>
          {#if running[current.id]}
            <button class="play stop" onclick={stop} title="Force-close the game (for when it's frozen)">{confirmKill ? "Click again to force-close" : "■ Stop"}</button>
          {:else}
            <button class="play" disabled={starting[current.id]} onclick={play}>{starting[current.id] ? "Starting…" : "Play"}</button>
          {/if}
        </div>
      </div>
      <div class="stats">
        <span>Last played: {ago(current.lastLaunchMs)}</span>
        <span>Time played: {hours(current.secondsPlayed)}</span>
        <span>{current.minecraft}</span>
        <span>{current.loader}{current.loaderVersion ? ` - ${current.loaderVersion}` : ""}</span>
      </div>
    </div>
  </section>

  <div class="tabs" role="tablist">
    <button role="tab" class:on={page === "content"} onclick={() => (page = "content")}><Icon name="cube" /> Content <span class="count">({total})</span></button>
    <button role="tab" class:on={page === "logs"} onclick={() => (page = "logs")}><Icon name="page" /> Logs</button>
    <button role="tab" class:on={page === "diagnosis"} onclick={() => { page = "diagnosis"; newCrash = false; }}>
      <Icon name="pulse" /> Diagnosis{#if newCrash}<span class="crash-dot" title="New crash"></span>{/if}
    </button>
    {#if page === "logs" || page === "diagnosis"}
      <button class="add-content copy-log" title={page === "logs" ? "Copy this log" : "Copy the crash report or log shown"} aria-label="Copy log" onclick={copyLog}>
        <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><rect x="8" y="8" width="12" height="13" rx="2" fill="none" stroke="currentColor" stroke-width="2" /><path d="M16 8V5a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h2" fill="none" stroke="currentColor" stroke-width="2" /></svg>
      </button>
    {:else}
    <button class="add-content" onclick={() => openBrowseTyping(kind === "resourcepacks" ? "resourcepack" : kind === "shaderpacks" ? "shader" : "mod")}>
      <svg viewBox="0 0 24 24" width="16" height="16"><path d="M12 5v14M5 12h14" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg> Add Content
    </button>
    {/if}
  </div>

  {#if error}<div class="error" role="alert">{error} <button class="link" onclick={() => (error = "")}>Dismiss</button></div>{/if}

  {#if flagged(current.id) && dupesHiddenFor !== current.id}
    <div class="dupes-back" role="presentation">
      <div class="dupes" role="alertdialog" aria-labelledby="dupes-title">
        <div class="dupes-title" id="dupes-title">⚠ You have {dupes.length === 1 ? "a duplicate mod" : `${dupes.length} duplicate mods`}. Remove one copy.</div>
        <p>The same mod is installed twice, usually once from Modrinth and once from CurseForge. The game won't start like this, so Play is blocked. Which copy would you like to remove?</p>
        {#each dupes as d (d.modId)}
          <div class="dupe">
            <strong>{dupeItem(d.files[0])?.name || d.modId}</strong>
            {#each d.files as f (f)}
              {@const it = dupeItem(f)}
              <div class="dupe-file">
                <span><code>{f}</code>{#if it} <small>· {SITE[it.source] ?? it.source}{it.version ? ` · ${it.version}` : ""}</small>{/if}</span>
                <button class="btn danger" disabled={dupeBusy} onclick={() => removeDupe(f)}>Remove this one</button>
              </div>
            {/each}
          </div>
        {/each}
        <div class="dupe-actions">
          <button class="link" onclick={() => (dupesHiddenFor = current.id)}>Not now</button>
          <button class="btn ghost" title="Allow Play with these duplicates. You'll be warned again if they change." onclick={() => ignoreDupes(current.id, true).catch((e) => (error = String(e)))}>Ignore (let it start anyway)</button>
        </div>
      </div>
    </div>
  {:else if dupes.length}
    <div class="error" role="status">
      {#if dupeState[current.id]?.ignored}Ignoring {dupes.length === 1 ? "a duplicate mod" : `${dupes.length} duplicate mods`} ({dupes.map((d) => d.modId).join(", ")}); the game may not start.
        <button class="link" onclick={() => ignoreDupes(current.id, false)}>Stop ignoring</button>
      {:else}{dupes.length === 1 ? "A mod is" : `${dupes.length} mods are`} installed twice, so Play is blocked.
        <button class="link" onclick={() => (dupesHiddenFor = "")}>Choose which to remove</button>
      {/if}
    </div>
  {/if}

  {#if page === "content"}
    <div class="subtabs" role="tablist">
      {#each KINDS as k (k.kind)}
        <button role="tab" class:on={sub === k.kind} onclick={() => setSub(k.kind)}>{k.label} ({content[k.kind].length})</button>
      {/each}
      <button role="tab" class:on={sub === "worlds"} onclick={() => setSub("worlds")}>Worlds ({worlds.length})</button>
    </div>

    {#if sub === "worlds"}
      <div class="grid-wrap">
        <div class="worlds">
          {#each worlds as w (w.folder)}
            <button class="world" onclick={() => reveal(`saves/${w.folder}`)} title="Open folder">
              {#if w.icon}<img src={convertFileSrc(w.icon)} alt="" loading="lazy" />{:else}<div class="thumb big">{initials(w.folder)}</div>{/if}
              <div class="text"><strong>{w.folder}</strong><small>Last played {ago(w.lastPlayedMs).toLowerCase()}</small>
                <small title={w.datapacks.join("\n")}>{w.datapacks.length ? `${w.datapacks.length} data pack${w.datapacks.length === 1 ? "" : "s"}` : "No data packs"}</small></div>
            </button>
          {:else}
            <div class="empty small">No worlds yet.</div>
          {/each}
        </div>
      </div>
    {:else}
      <div class="toolbar">
        {#if viewTrash}
          <span class="sel">Trash · {trash.filter((t) => t.kind === kind).length} item{trash.filter((t) => t.kind === kind).length === 1 ? "" : "s"}</span>
          {#if selected.size}
            <button class="ghost" onclick={() => restoreIds([...selected])}>Restore</button>
            <button class="ghost danger" onclick={() => removeIds([...selected])}>Delete now</button>
            <button class="link" onclick={() => (selected = new Set())}>Clear</button>
          {/if}
          <span class="muted small">Items are deleted for good {trashLabel(settings.trashHours)} after you remove them.</span>
        {:else if selected.size}
          <span class="sel">{selected.size} selected</span>
          <button class="ghost" onclick={() => bulkEnable(true)}>Enable</button>
          <button class="ghost" onclick={() => bulkEnable(false)}>Disable</button>
          <button class="ghost" onclick={() => setPinned(content[kind].filter((i) => selected.has(i.fileName)), true)}>Pin</button>
          <button class="ghost" onclick={() => setPinned(content[kind].filter((i) => selected.has(i.fileName)), false)}>Unpin</button>
          <button class="ghost danger" onclick={() => (confirmDelete = content[kind].filter((i) => selected.has(i.fileName)))}>Delete</button>
          <button class="link" onclick={() => (selected = new Set())}>Clear</button>
        {:else}
          <button class="ghost" disabled={checking || updating.size > 0 || !pendingHere.length}
            title={checking ? "Checking for updates…" : pendingHere.length ? `${pendingHere.length} update(s) available` : "Everything is up to date"}
            onclick={() => updateAll(pendingHere)}>
            <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true" class:spin={checking || updating.size > 0}><path d="M20 12a8 8 0 1 1-2.3-5.7M20 4v5h-5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>
            {checking ? "Checking…" : updating.size ? "Updating…" : "Update all"}{#if pendingHere.length && !checking}<span class="count">{pendingHere.length}</span>{/if}
          </button>
          <button class="ghost trash-btn" onclick={() => { viewTrash = true; selected = new Set(); }} title="Trash: recently deleted content" aria-label="Trash">
            <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>
            {#if trash.length}<span class="fcount">{trash.length}</span>{/if}
          </button>
        {/if}
        <div class="hopper-wrap">
          <button class="hopper" class:active={filterCount > 0} title="Filter and sort" aria-label="Filter and sort"
            onclick={(e) => { e.stopPropagation(); showFilters = !showFilters; }}>
            <svg viewBox="0 0 16 16" width="17" height="17" aria-hidden="true" shape-rendering="crispEdges"><path d="M1 2h14v2h-1v3h-2v2h-2v3H9v2H7v-2H6V9H4V7H2V4H1z" fill="currentColor" opacity="0.9" /><path d="M3 4h10v2H3z" fill="var(--bg)" opacity="0.55" /></svg>{#if filterCount}<span class="fcount">{filterCount}</span>{/if}
          </button>
          {#if showFilters}
            <div class="filters" role="dialog" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && (showFilters = false)}>
              <div class="fhead"><strong>Filter and sort</strong>{#if filterCount}<button class="link" onclick={() => (filters = { ...NO_FILTERS })}>Reset</button>{/if}</div>
              <label class="fsort">Sort by
                <select value={sortKey === "date" ? `${dateMode}:${sortAsc ? "asc" : "desc"}` : `${sortKey}:${sortAsc ? "asc" : "desc"}`} onchange={(e) => sortFrom(e.currentTarget.value)}>
                  <option value="name:asc">Name (A–Z)</option>
                  <option value="name:desc">Name (Z–A)</option>
                  <option value="installed:desc">Installed (newest first)</option>
                  <option value="installed:asc">Installed (oldest first)</option>
                  <option value="modified:desc">Last modified (newest first)</option>
                  <option value="modified:asc">Last modified (oldest first)</option>
                  <option value="authors:asc">Author</option>
                  <option value="update:asc">Updates first</option>
                </select>
              </label>
              <div class="fgroups">
                <section>
                  <h5>Status</h5>
                  {#each [["enabled", "Enabled"], ["disabled", "Disabled"], ["update", "Has an update"], ["pinned", "Pinned"]] as [v, l] (v)}
                    <label class="fopt"><input type="checkbox" checked={filters.status.includes(v)} onchange={() => toggleFilter("status", v)} /> {l}</label>
                  {/each}
                  {#if kind === "mods"}
                    <label class="fopt"><input type="checkbox" checked={!!filters.noLibs} onchange={() => (filters = { ...filters, noLibs: !filters.noLibs })} /> Hide libraries</label>
                  {/if}
                </section>
                {#if shown("source")}
                <section>
                  <h5>Source</h5>
                  {#each [["curseforge", "CurseForge"], ["modrinth", "Modrinth"], ["local", "Local"]] as [v, l] (v)}
                    <label class="fopt"><input type="checkbox" checked={filters.sources.includes(v)} onchange={() => toggleFilter("sources", v)} /> {l}</label>
                  {/each}
                </section>
                {/if}
                {#if shown("env")}
                <section>
                  <h5>Environment</h5>
                  {#each [["client", "Client only"], ["server", "Server only"], ["both", "Client & server"], ["unknown", "Unknown"]] as [v, l] (v)}
                    <label class="fopt"><input type="checkbox" checked={filters.envs.includes(v)} onchange={() => toggleFilter("envs", v)} /> {l}</label>
                  {/each}
                </section>
                {/if}
              </div>
            </div>
          {/if}
        </div>
        <input class="search" placeholder="Search {KINDS.find((k) => k.kind === kind)?.label.toLowerCase()} or authors…" bind:value={query} />
      </div>

      <div class="table" role="table" style:--cols={colTemplate} bind:clientWidth={tableW}>
        <div class="thead" role="row" tabindex="-1" title="Right-click to choose columns"
          oncontextmenu={(e) => { e.preventDefault(); e.stopPropagation(); colMenu = { x: Math.min(e.clientX, innerWidth - 230), y: e.clientY }; }}>
          <span><input type="checkbox" checked={allChecked} onchange={toggleAll} aria-label="Select all" /></span>
          {#each [["name", "Name"], ["date", ""], ["authors", "Author"], ["env", "Env"], ["source", "Source"], ["update", viewTrash ? "" : "Action"], ["enabled", viewTrash ? "" : "Active"]].filter(([k]) => k === "name" || vis(k)) as [key, label] (key)}
            {#if key === "date"}
              <div class="pop">
                <button class="th" onclick={(e) => { e.stopPropagation(); if (viewTrash) sortBy("date"); else dateMenu = !dateMenu; }}>
                  {viewTrash ? "Deleted" : dateMode === "modified" ? "Modified" : "Installed"}{#if sortKey === "date"}<span class="arrow">{sortAsc ? "▲" : "▼"}</span>{/if} {#if !viewTrash}<span class="caret">▾</span>{/if}
                </button>
                {#if dateMenu}
                  <div class="ctx inline" role="menu" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
                    {#each [["installed", false, "Installed · newest first"], ["installed", true, "Installed · oldest first"], ["modified", false, "Modified · newest first"], ["modified", true, "Modified · oldest first"]] as [mode, asc, text] (text)}
                      <button class:on={sortKey === "date" && dateMode === mode && sortAsc === asc}
                        onclick={() => { dateMode = mode as typeof dateMode; sortKey = "date"; sortAsc = asc as boolean; dateMenu = false; try { localStorage.setItem("obelisk.dateMode", dateMode); } catch {} }}>{text}</button>
                    {/each}
                  </div>
                {/if}
              </div>
            {:else if label}
              <button class="th" class:center={key === "enabled" || key === "env"} onclick={() => sortBy(key as SortKey)}
                title={key === "source" ? "Click to change which source comes first" : undefined}>
                {label}{#if sortKey === key && key === "source"}<span class="arrow" title="{SOURCE_LABEL[sourceFirst as keyof typeof SOURCE_LABEL]} first">· {({ local: "Local", curseforge: "CF", modrinth: "MR" } as Record<string, string>)[sourceFirst]}</span>{:else if sortKey === key}<span class="arrow">{sortAsc ? "▲" : "▼"}</span>{/if}
              </button>
            {:else}<span></span>{/if}
          {/each}
        </div>
        {#if colMenu}
          <div class="ctx colmenu" style:left="{colMenu.x}px" style:top="{colMenu.y}px" role="menu" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && (colMenu = null)}>
            <div class="cm-title">Columns</div>
            <button disabled><span class="tick">✓</span> Name</button>
            {#each COLUMNS as [c, l] (c)}
              <button onclick={() => toggleCol(c)}><span class="tick">{shown(c) ? "✓" : ""}</span> {l}</button>
            {/each}
          </div>
        {/if}
        <div class="tbody">
          {#each rows as item (item.trashId ?? item.fileName)}
            {@const f = infoOf(item)}
            {@const src = item.source === "local" && f?.provider ? f.provider : item.source}
            {@const when = viewTrash || dateMode === "installed" ? item.installedMs : item.modifiedMs}
            <div class="row" class:off={!item.enabled} class:picked={selected.has(item.fileName)} role="row" tabindex="-1" oncontextmenu={(e) => { e.preventDefault(); e.stopPropagation(); dateMenu = false; menu = { item, x: e.clientX, y: e.clientY }; }}>
              <span><input type="checkbox" checked={selected.has(item.trashId ?? item.fileName)} onchange={() => toggleRow(item.trashId ?? item.fileName)} aria-label="Select {item.name}" /></span>
              <div class="name">
                {#if icons[item.fileName] || f?.iconUrl}<img class="thumb" use:crisp src={icons[item.fileName] || f?.iconUrl} alt="" loading="lazy" decoding="async" />{:else}<div class="thumb">{initials(item.name)}</div>{/if}
                <button class="text linkish" onclick={() => openItem(item)} title="Open project page"><strong>{item.name}</strong><small>{item.fileName}</small>{#if item.version}<small class="ver">{item.version}</small>{/if}</button>
              </div>
              {#if vis("date")}<span class="muted small" title={when ? new Date(when).toLocaleString() : ""}>{when ? new Date(when).toLocaleDateString(undefined, { dateStyle: "medium" }) : "–"}</span>{/if}
              {#if vis("authors")}<span class="ellipsis">{#if item.authors && f?.provider}<button class="author" title="Open the author's page" onclick={() => openAuthor(item)}>{item.authors}</button>{:else}<span class="muted" title={item.authors}>{item.authors}</span>{/if}</span>{/if}
              {#if vis("env")}<span class="center env" title={ENV_LABEL[f?.env ?? ""]}>
                {#if f?.env === "client" || f?.env === "both"}<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><rect x="3" y="4" width="18" height="12" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.8" /><path d="M8 20h8M12 16v4" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" /></svg>{/if}
                {#if f?.env === "server" || f?.env === "both"}<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><rect x="4" y="3" width="16" height="7" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.8" /><rect x="4" y="14" width="16" height="7" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.8" /><circle cx="8" cy="6.5" r="1" fill="currentColor" /><circle cx="8" cy="17.5" r="1" fill="currentColor" /></svg>{/if}
                {#if !f?.env}<span class="muted">–</span>{/if}
              </span>{/if}
              {#if vis("source")}<span><span class="badge {src}">{SOURCE_LABEL[src as keyof typeof SOURCE_LABEL]}</span></span>{/if}
              {#if vis("update")}<span>
                {#if item.trashId}
                  <button class="upd" onclick={() => restoreIds([item.trashId!])}>Restore</button>
                {:else if isPinned(item)}
                  <button class="pinned" title={f?.update ? `Pinned. ${f.update.name || "A newer version"} is available; unpin to update.` : "Pinned: Update all skips this. Click to unpin."}
                    onclick={() => setPinned([item], false)}>
                    <svg viewBox="0 0 24 24" width="13" height="13" aria-hidden="true"><path d="M15 3l6 6-3 1-4 4 1 5-2 2-4-4-5 5-1-1 5-5-4-4 2-2 5 1 4-4z" fill="currentColor" /></svg>
                    Pinned{#if f?.update}<i class="pdot"></i>{/if}
                  </button>
                {:else if f?.update}
                  <button class="upd" disabled={updating.has(`${f.kind}/${f.fileName}`)} onclick={() => update(f)}
                    title="{f.update.name || f.update.fileName}{f.update.blocked ? ' (download from curseforge.com)' : ''}">{updating.has(`${f.kind}/${f.fileName}`) ? "…" : "Update"}</button>
                {:else if checking}
                  <span class="muted small">…</span>
                {:else}
                  <span class="muted small">Up to date</span>
                {/if}
              </span>{/if}
              {#if vis("enabled")}<span class="center">
                {#if !item.trashId}<button class="switch" class:on={item.enabled} role="switch" aria-checked={item.enabled}
                  aria-label="Toggle {item.name}" onclick={() => setEnabled(item, !item.enabled)}><i></i></button>{/if}
              </span>{/if}
            </div>
          {:else}
            <div class="empty small">{query ? "Nothing matches your search." : viewTrash ? `Nothing deleted here in the last ${trashLabel(settings.trashHours)}.` : "Nothing here yet."}</div>
          {/each}
        </div>
      </div>
    {/if}
  {:else if page === "diagnosis"}
    <Diagnosis bind:file={diagFile} instanceId={current.id} mods={content.mods} {icons} infoOf={(i) => infos.get(`mods/${plain(i.fileName)}`)}
      onToggle={(i, want) => setEnabled(i, want)} onOpen={(i) => openItem(i)} onShaders={() => { page = "content"; setSub("shaderpacks"); }} />
  {:else if page === "logs"}
    <div class="logs">
      <div class="log-list">
        {#each logs as name (name)}
          <button class:on={logName === name} onclick={() => openLog(name)}>{name}</button>
        {:else}
          <div class="empty small">No logs yet. They appear after the first launch.</div>
        {/each}
        <button class="ghost wide" onclick={() => reveal("logs")}>Open logs folder</button>
      </div>
      <pre class="log-view">{logName ? logText : "Pick a log on the left."}</pre>
    </div>
  {/if}
</div>

{#if confirmDelete}
  <div class="modal-back" role="presentation" onclick={() => (confirmDelete = null)}>
    <div class="modal" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && (confirmDelete = null)}>
      <h3>Move {confirmDelete.length === 1 ? confirmDelete[0].name : `${confirmDelete.length} items`} to the Recycle Bin?</h3>
      <p class="muted">You can restore {confirmDelete.length === 1 ? "it" : "them"} from the Recycle Bin.</p>
      <div class="modal-actions">
        <button class="ghost" onclick={() => (confirmDelete = null)}>Cancel</button>
        <button class="play danger-fill" onclick={doDelete}>Delete</button>
      </div>
    </div>
  </div>
{/if}

{#if configsFor}
  <ConfigEditor instanceId={current.id} fileName={configsFor.fileName} name={configsFor.name} onclose={() => (configsFor = null)} />
{/if}

{#if menu}
  {@const m = menu}
  <div class="ctx" style:left="{Math.min(m.x, innerWidth - 220)}px" style:top="{Math.min(m.y, innerHeight - 220)}px" role="menu" tabindex="-1"
    onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
    <button onclick={() => { const it = m.item; menu = null; openItem(it); }}>Open project page</button>
    {#if kind === "mods"}<button onclick={() => { const it = m.item; menu = null; configsFor = it; }}>Edit configs</button>{/if}
    <button onclick={() => { const it = m.item; menu = null; setEnabled(it, !it.enabled); }}>{m.item.enabled ? "Disable" : "Enable"}</button>
    {#if !m.item.trashId}<button onclick={() => { const it = m.item; menu = null; setPinned([it], !isPinned(it)); }}>{isPinned(m.item) ? "Unpin version" : "Pin this version"}</button>{/if}
    <button onclick={() => { const it = m.item; menu = null; invoke("reveal_file", { instanceId: current.id, kind, fileName: it.fileName }).catch((e) => toast(String(e))); refreshContent(current.id); }}>Show in folder</button>
    <button class="danger" onclick={() => { const it = m.item; menu = null; confirmDelete = selected.has(it.fileName) && selected.size > 1 ? content[kind].filter((i) => selected.has(i.fileName)) : [it]; }}>Delete</button>
  </div>
{/if}

{#if viewTrash && page === "content" && sub !== "worlds"}
  <button class="back-float" onclick={() => { viewTrash = false; selected = new Set(); }}>‹ Back to {KINDS.find((k) => k.kind === kind)?.label.toLowerCase()}</button>
{/if}


<style>
  button { font: inherit; color: inherit; cursor: pointer; }
  button:disabled { cursor: default; }
  input[type="checkbox"] { accent-color: var(--accent); width: 15px; height: 15px; margin: 0; cursor: pointer; }

  .hero-icon span { font-weight: 700; color: var(--muted); }


  .hero { display: flex; gap: 18px; margin-top: 16px; padding: 14px; background: var(--panel); border-radius: 10px; flex: none; }
  .hero-icon { width: 104px; height: 104px; flex: none; border-radius: 8px; overflow: hidden; background: var(--panel-2); display: grid; place-items: center; font-size: 28px; }
  .hero-icon img { width: 100%; height: 100%; object-fit: cover; }
  .hero-body { flex: 1; display: flex; flex-direction: column; justify-content: space-between; min-width: 0; }
  .instance { display: flex; flex-direction: column; flex: 1; min-height: 0; }
  .hero-actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 10px; align-items: center; }
  .hero-title { display: flex; flex-wrap: wrap; justify-content: space-between; align-items: flex-start; gap: 10px 16px; }
  h2 { margin: 2px 0; font-size: 20px; font-weight: 600; }
  .sub { margin: 0; color: var(--muted); font-size: 13px; }
  .play { background: var(--accent); border: 0; border-radius: 6px; padding: 9px 30px; font-weight: 600; font-size: 15px; color: #fff; transition: background 0.15s; }
  .play:hover:not(:disabled) { background: var(--accent-hover); }
  .play:disabled { opacity: 0.7; }
  .play.stop { background: var(--danger); }
  .play.stop:hover { background: color-mix(in srgb, var(--danger) 85%, #fff); }
  .stats { display: flex; flex-wrap: wrap; gap: 6px 20px; justify-content: flex-end; border-top: 1px solid var(--line); padding-top: 10px; color: var(--muted); font-size: 12.5px; }

  .tabs, .subtabs { display: flex; flex-wrap: wrap; flex: none; }
  .tabs { gap: 6px 26px; margin-top: 16px; border-bottom: 1px solid var(--line); }
  .tabs button { background: none; border: 0; padding: 8px 0 10px; color: var(--muted); border-bottom: 2px solid transparent; margin-bottom: -1px; transition: color 0.15s; }
  .tabs button:hover, .subtabs button:hover { color: var(--text); }
  .tabs .add-content { margin: 0 0 6px auto; display: inline-flex; align-items: center; gap: 8px; background: var(--panel-2); color: var(--text); border: 1px solid var(--line); border-radius: 4px; padding: 7px 14px; font-size: 15px; }
  .tabs .copy-log { padding: 7px 11px; }
  .tabs .add-content:hover { border-color: var(--muted); }
  .tabs button.on { color: var(--text); border-bottom-color: var(--accent); }
  .count { opacity: 0.8; }
  .subtabs { gap: 4px; margin-top: 12px; }
  .subtabs button { background: none; border: 0; padding: 5px 12px; border-radius: 6px; color: var(--muted); }
  .subtabs button.on { color: var(--accent); background: color-mix(in srgb, var(--accent) 12%, transparent); }

  .toolbar { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; padding: 12px 0; flex: none; min-height: 58px; }
  .sel { font-weight: 600; margin-right: 4px; }
  .ghost { display: inline-flex; align-items: center; gap: 8px; background: none; border: 1px solid var(--line); border-radius: 6px; padding: 7px 12px; transition: border-color 0.15s; }
  .ghost:hover:not(:disabled) { border-color: var(--muted); }
  .ghost:disabled { color: var(--muted); }
  .ghost.danger { color: var(--danger); }
  .ghost.wide { width: 100%; justify-content: center; margin-top: 8px; }
  .link { background: none; border: 0; color: var(--muted); text-decoration: underline; padding: 4px; }
  .search { margin-left: auto; width: 260px; background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 6px; padding: 7px 10px; font: inherit; outline: none; }
  .search:focus { border-color: var(--accent); }
  .error { background: var(--err-bg); color: var(--err-text); padding: 8px 12px; border-radius: 6px; margin-top: 10px; user-select: text; }

  .table { flex: 1; min-width: 0; display: flex; flex-direction: column; min-height: 0; background: var(--panel); border-radius: 10px 10px 0 0; }
  .thead, .row { display: grid; grid-template-columns: var(--cols); gap: 14px; align-items: center; padding: 0 16px; }
  .thead { height: 38px; border-bottom: 1px solid var(--line); flex: none; }
  .th { background: none; border: 0; padding: 0; text-align: left; font-size: 12.5px; font-weight: 600; color: var(--muted); }
  .th:hover { color: var(--text); }
  .arrow { font-size: 9px; margin-left: 5px; color: var(--accent); }
  .tbody { overflow-y: auto; flex: 1; scrollbar-gutter: stable; }
  .row { height: 68px; border-bottom: 1px solid var(--row-line); content-visibility: auto; contain-intrinsic-size: auto 68px; }
  .row:hover { background: var(--panel-2); }
  .row.picked { background: color-mix(in srgb, var(--accent) 8%, var(--panel)); }
  .row.off .text, .row.off .thumb { opacity: 0.45; }
  .name { display: flex; align-items: center; gap: 12px; min-width: 0; }
  .thumb { width: 36px; height: 36px; flex: none; border-radius: 6px; background: var(--panel-2); display: grid; place-items: center; font-size: 11px; font-weight: 700; color: var(--muted); object-fit: cover; }
  .thumb:global(.px) { image-rendering: pixelated; }
  .thumb.big { width: 64px; height: 64px; font-size: 18px; }
  .text { display: flex; flex-direction: column; min-width: 0; text-align: left; }
  .linkish { background: none; border: 0; padding: 0; }
  .linkish:hover strong { color: var(--accent); }
  .text strong, .text small, .ellipsis { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .text strong { font-weight: 600; }
  .text small { color: var(--muted); font-size: 12px; }
  .muted { color: var(--muted); font-size: 13px; }
  .center { text-align: center; justify-self: center; }
  .text small.ver { font-size: 11.5px; opacity: 0.8; }
  .pop { position: relative; }
  .caret { font-size: 10px; opacity: 0.7; }
  .colmenu { min-width: 220px; }
  .cm-title { padding: 6px 12px 4px; font-size: 11.5px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
  .tick { display: inline-block; width: 18px; color: var(--accent); }
  .ctx.inline { position: absolute; top: 28px; left: 0; z-index: 30; }
  .ctx button.on { color: var(--accent); font-weight: 600; }
  .back-float { position: fixed; right: 28px; bottom: 24px; z-index: 30; background: var(--accent); color: #fff; border: 0; border-radius: 999px; padding: 10px 18px; font-weight: 600; box-shadow: 0 8px 24px rgb(0 0 0 / 0.4); }
  .back-float:hover { background: var(--accent-hover); }
  .crash-dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; background: #f2555a; margin-left: 6px; vertical-align: middle; box-shadow: 0 0 0 3px rgb(242 85 90 / 0.25); }
  .pinned { display: inline-flex; align-items: center; gap: 4px; background: none; border: 1px solid var(--line); color: var(--text-2); border-radius: 4px; padding: 4px 9px; font-size: 12px; font-weight: 600; position: relative; }
  .pinned:hover { border-color: var(--accent); color: var(--accent); }
  .pdot { width: 6px; height: 6px; border-radius: 50%; background: var(--accent); position: absolute; top: -3px; right: -3px; }
  .hopper-wrap { position: relative; margin-left: auto; }
  .hopper-wrap + .search { margin-left: 0; }
  .hopper { position: relative; background: var(--panel); border: 1px solid var(--line); border-radius: 6px; width: 38px; height: 36px; display: grid; place-items: center; color: var(--text-2); }
  .hopper svg, .trash-btn svg { display: block; }
  .trash-btn { position: relative; width: 38px; height: 36px; padding: 0 !important; display: grid !important; place-items: center; }
  .hopper:hover, .hopper.active { color: var(--accent); border-color: var(--accent); }
  .fcount { position: absolute; top: -6px; right: -6px; background: var(--accent); color: #fff; border-radius: 999px; font-size: 10.5px; min-width: 17px; height: 17px; display: grid; place-items: center; padding: 0 4px; }
  .filters { position: absolute; right: 0; top: 42px; z-index: 40; width: min(460px, calc(100vw - 110px)); background: var(--panel-2); border: 1px solid var(--line); border-radius: 10px; padding: 12px 14px; box-shadow: 0 14px 40px rgb(0 0 0 / 0.45); display: flex; flex-direction: column; gap: 10px; }
  .fhead { display: flex; justify-content: space-between; align-items: center; }
  .fsort { display: flex; align-items: center; gap: 10px; font-size: 13px; color: var(--text-2); }
  .fsort select { flex: 1; background: var(--bg); border: 1px solid var(--line); color: var(--text); border-radius: 5px; padding: 6px 8px; }
  .fgroups { display: grid; grid-template-columns: repeat(auto-fit, minmax(130px, 1fr)); gap: 14px; }
  .fgroups h5 { margin: 8px 0 4px; font-size: 11.5px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); display: flex; justify-content: space-between; }
  .fopt { display: flex; align-items: center; gap: 8px; font-size: 13px; padding: 3px 0; cursor: pointer; }
  .fopt input { accent-color: var(--accent); }
  .fsearch { width: 100%; background: var(--bg); border: 1px solid var(--line); color: var(--text); border-radius: 5px; padding: 6px 8px; margin-bottom: 4px; }
  .author { background: none; border: 0; padding: 0; color: var(--text-2); max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .author:hover { color: var(--accent); text-decoration: underline; }
  .ctx { position: fixed; z-index: 50; min-width: 200px; background: var(--panel-2); border: 1px solid var(--line); border-radius: 8px; padding: 5px; box-shadow: 0 10px 30px rgb(0 0 0 / 0.4); display: flex; flex-direction: column; }
  .ctx button { text-align: left; background: none; border: 0; padding: 8px 12px; border-radius: 5px; }
  .ctx button:hover { background: var(--panel); }
  .ctx .danger { color: var(--danger); }
  .muted-count { background: var(--panel-2); color: var(--muted); }
  .env { display: flex; justify-content: center; gap: 3px; color: var(--muted); }
  .small { font-size: 12.5px; }
  .upd { background: var(--accent); color: #fff; border: 0; border-radius: 4px; padding: 5px 12px; font-weight: 600; font-size: 12.5px; }
  .upd:hover:not(:disabled) { background: var(--accent-hover); }
  .upd:disabled { opacity: 0.6; }
  .count { background: var(--accent); color: #fff; border-radius: 999px; font-size: 11px; padding: 1px 7px; margin-left: 6px; }
  .spin { animation: spin 1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .badge { font-size: 11.5px; padding: 2px 8px; border-radius: 999px; border: 1px solid var(--line); color: var(--muted); }
  .badge.curseforge { color: var(--cf); border-color: color-mix(in srgb, var(--cf) 40%, transparent); }
  .badge.modrinth { color: var(--mr); border-color: color-mix(in srgb, var(--mr) 40%, transparent); }
  .switch { width: 40px; height: 22px; border-radius: 999px; border: 0; padding: 0; position: relative; background: var(--switch-off); transition: background 0.15s; }
  .switch i { position: absolute; top: 3px; left: 3px; width: 16px; height: 16px; border-radius: 50%; background: #fff; transition: transform 0.15s; }
  .switch.on { background: var(--accent); }
  .switch.on i { transform: translateX(18px); }

  .grid-wrap { flex: 1; overflow-y: auto; min-height: 0; padding-bottom: 20px; }
  .worlds { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 12px; padding-top: 14px; }
  .world { display: flex; align-items: center; gap: 14px; padding: 12px; background: var(--panel); border: 1px solid transparent; border-radius: 10px; transition: border-color 0.15s; }
  .world:hover { border-color: var(--line); }
  .world img { width: 64px; height: 64px; border-radius: 6px; image-rendering: pixelated; }

  .logs { flex: 1; display: grid; grid-template-columns: 220px 1fr; gap: 14px; min-height: 0; padding: 14px 0 20px; }
  .log-list { display: flex; flex-direction: column; gap: 2px; overflow-y: auto; }
  .log-list > button:not(.ghost) { background: none; border: 0; text-align: left; padding: 7px 10px; border-radius: 6px; color: var(--muted); }
  .log-list > button.on { background: var(--panel-2); color: var(--text); }
  .log-view { margin: 0; overflow: auto; background: var(--panel); border-radius: 10px; padding: 12px 14px; font: 12px/1.5 "Cascadia Mono", Consolas, monospace; color: var(--text-2); user-select: text; white-space: pre-wrap; word-break: break-all; }

  .modal-back { position: fixed; inset: 0; background: rgb(0 0 0 / 0.6); display: grid; place-items: center; z-index: 10; }
  .modal { background: var(--panel); border: 1px solid var(--line); border-radius: 12px; padding: 20px 22px; width: min(440px, 90vw); outline: none; }
  .modal h3 { margin: 0 0 6px; font-size: 16px; }
  .modal-actions { display: flex; justify-content: flex-end; gap: 10px; margin-top: 18px; }
  .danger-fill { background: var(--danger); padding: 8px 22px; font-size: 14px; }
  .danger-fill:hover { background: #f2555a !important; }

  .empty { color: var(--muted); padding: 40px; text-align: center; }
  .empty.small { padding: 24px; }
  :focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .dupes-back { position: fixed; inset: 0; z-index: 60; display: grid; place-items: center; background: rgb(0 0 0 / 0.45); padding: 24px; }
  .dupes { width: min(720px, 100%); max-height: 80vh; overflow: auto; background: #7a1010; color: #fff; border: 3px solid #ff4b4b; border-radius: 12px;
    padding: 20px 24px; box-shadow: 0 20px 60px rgb(0 0 0 / 0.6); user-select: text; }
  .dupes p { margin: 8px 0 12px; opacity: 0.95; }
  .dupes .dupes-title { font-size: 1.5em; font-weight: 800; }
  .dupes .dupe { margin-top: 10px; padding: 10px 12px; background: rgb(0 0 0 / 0.25); border-radius: 8px; }
  .dupes .dupe-file { display: flex; justify-content: space-between; align-items: center; gap: 12px; margin-top: 6px; }
  .dupes .dupe-file code { word-break: break-all; }
  .dupes .dupe-file small { opacity: 0.8; }
  .dupes .btn { border-radius: 6px; padding: 7px 14px; font-weight: 600; cursor: pointer; }
  .dupes .btn:disabled { opacity: 0.6; cursor: default; }
  .dupes .btn.danger { background: #ff4b4b; color: #fff; border: 0; white-space: nowrap; }
  .dupes .dupe-actions { display: flex; justify-content: flex-end; align-items: center; gap: 16px; margin-top: 16px; }
  .dupes .link, .dupes .btn.ghost { color: #fff; }
  .dupes .btn.ghost { background: transparent; border: 1px solid rgb(255 255 255 / 0.5); }
</style>
