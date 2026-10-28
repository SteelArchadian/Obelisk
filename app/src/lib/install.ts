import { api, type Hit, type Installed, type Instance, type Kind, type ProjectType, type Tags } from "./api";
import { app, toast, pushOverlay, type PendingInstall } from "./state.svelte";
import { cfToHit, manualDownload } from "./cf";
import { recheck, updatesFor } from "./updates.svelte";

let tagsPromise: Promise<Tags> | null = null;
/** Modrinth tags (categories, loaders, versions) fetched once per session. */
export function tagsOnce() {
  tagsPromise ??= api.tags().catch((e) => { tagsPromise = null; throw e; });
  return tagsPromise;
}

const LOADERS = new Set(["neoforge", "forge", "fabric", "quilt", "liteloader", "rift", "modloader",
  "iris", "optifine", "canvas", "vanilla", "minecraft", "datapack", "bukkit", "paper", "spigot", "folia",
  "purpur", "sponge", "velocity", "waterfall", "bungeecord"]);

export const loadersOf = (h: Hit) => h.categories.filter((c) => LOADERS.has(c));

export type Compat =
  | { ok: true }
  | { ok: false; reason: string; connector: boolean };

/** Can this project go into that profile as-is? If not, would Sinytra Connector make it work? */
export function compat(h: Hit, inst: Instance | null): Compat {
  if (!inst || h.project_type === "modpack" || h.project_type === "datapack") return { ok: true };
  const mc = inst.minecraft;
  const loader = inst.loader.toLowerCase();
  const loaders = loadersOf(h);
  const mcOk = h.versions.length === 0 || h.versions.includes(mc);
  if (h.project_type === "mod") {
    const loaderOk = loaders.length === 0 || loaders.includes(loader);
    if (loaderOk && mcOk) return { ok: true };
    const connector = mcOk && loader === "neoforge" && loaders.includes("fabric");
    const reason = !mcOk ? `No ${mc} version` : `${loaders.map(cap).join(" / ")} only`;
    return { ok: false, reason, connector };
  }
  if (!mcOk && h.project_type === "shader") return { ok: false, reason: `No ${mc} version`, connector: false };
  return { ok: true };
}

export const cap = (s: string) => s.replace(/-/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());

/** Make `inst` the install target and load what it already contains. */
export async function setTarget(inst: Instance | null) {
  app.target = inst;
  if (inst) await refreshInstalled();
}

export async function refreshInstalled() {
  const inst = app.target;
  if (!inst) return;
  try {
    const list = await api.installed(inst.id);
    if (app.target?.id !== inst.id) return;
    app.installed = new Map(list.map((i) => [i.projectId, i]));
    app.installedFor = inst.id;
  } catch (e) {
    toast(`Couldn't check installed mods: ${e}`);
  }
  // CurseForge-only files are identified by fingerprint in the (slower) update check. Its results
  // can be hours old, so only count files that are still in the profile (not ones deleted since).
  updatesFor(inst.id).then(async (infos) => {
    const cf = infos.filter((i) => i.provider === "curseforge");
    const kinds = [...new Set(cf.map((i) => i.kind))];
    const lists = await Promise.all(kinds.map((k) => api.content(inst.id, k as Kind)));
    const present = new Set(lists.flatMap((l, n) => l.map((i) => `${kinds[n]}/${i.fileName}`)));
    if (app.target?.id !== inst.id) return;
    const m = new Map(app.installed);
    for (const i of cf) {
      if (present.has(`${i.kind}/${i.fileName}`)) m.set(`cf:${i.projectId}`, { projectId: `cf:${i.projectId}`, versionId: i.current, kind: i.kind, fileName: i.fileName });
    }
    app.installed = m;
  }).catch(() => {});
}

/** A mod's name from its file: "chat_heads-0.13.6-neoforge-1.21.1.jar" -> "chatheads". */
export function fileKey(file: string): string {
  const stem = file.replace(/\.disabled$/i, "").replace(/\.(jar|zip)$/i, "");
  const words: string[] = [];
  for (const w of stem.split(/[-_+\s]+/)) {
    if (/^v?\d/i.test(w) || /^(neo)?forge$|^fabric$|^quilt$|^mc\d|^minecraft$|^universal$|^all$/i.test(w)) break;
    words.push(w);
  }
  return nameKey(words.join(""));
}
export const nameKey = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, "");

let byName: { from: Map<string, Installed>; keys: Map<string, Installed> } | null = null;

/**
 * Is this project already in the target profile? By either site's id, or else by name, so a mod
 * installed from Modrinth also counts for its CurseForge listing and the other way round.
 */
export function isInstalled(h: Hit): Installed | undefined {
  const hit = app.installed.get(h.project_id) ?? (h.cf_id ? app.installed.get(`cf:${h.cf_id}`) : undefined);
  if (hit) return hit;
  if (byName?.from !== app.installed) {
    const keys = new Map<string, Installed>();
    for (const i of app.installed.values()) {
      const k = fileKey(i.fileName);
      if (k.length >= 3 && !keys.has(k)) keys.set(k, i);
    }
    byName = { from: app.installed, keys };
  }
  return byName.keys.get(nameKey(h.slug)) ?? byName.keys.get(nameKey(h.title));
}

/**
 * Install a project. Modpacks go to Prism's importer. Everything else goes to the remembered
 * target, or asks for a profile first if there isn't one.
 */
export async function install(p: PendingInstall): Promise<boolean> {
  const { hit } = p;
  try {
    if (hit.project_type === "modpack") {
      const versions = await api.versions(hit.project_id);
      const v = p.versionId ? versions.find((x) => x.id === p.versionId) : versions.find((x) => x.version_type === "release") ?? versions[0];
      const file = v?.files.find((f) => f.primary) ?? v?.files[0];
      if (!file) throw new Error("This modpack has no downloadable version.");
      await api.importPack(file.url);
      toast(`Opened ${hit.title} in Prism's importer.`);
      return true;
    }
    if (!app.target) {
      app.picker = p;
      return false;
    }
    const target = app.target;
    // Data packs live inside a world, so ask which one.
    if (hit.project_type === "datapack" && !p.world) {
      app.worldPicker = p;
      return false;
    }
    const into = p.world ? `${p.world} (${target.name})` : target.name;
    // CurseForge-only projects (and CurseForge versions picked on a CurseForge page).
    if (hit.project_id.startsWith("cf:")) {
      const r = await api.cfInstall(target.id, hit.cf_id!, p.versionId ? Number(p.versionId) : undefined, p.world, p.loaderOverride);
      if (r.status === "blocked") {
        if (r.names.length) toast(`Installed ${r.names.join(", ")} into ${into}.`);
        await manualDownload(target.id, r, refreshInstalled);
      } else {
        const extra = r.names.length > 1 ? ` (+ required: ${r.names.slice(1).join(", ")})` : "";
        toast(`Installed ${r.names[0]} into ${into}${extra}.`);
      }
      recheck(target.id).catch(() => {});
      await refreshInstalled();
      return true;
    }
    const names = await api.install(app.target.id, {
      projectId: hit.project_id, projectType: hit.project_type, versionId: p.versionId, loaderOverride: p.loaderOverride,
    }, p.world);
    const extra = names.length > 1 ? ` (+ required: ${names.slice(1).join(", ")})` : "";
    toast(`Installed ${names[0]} into ${into}${extra}.`);
    recheck(target.id).catch(() => {});
    await refreshInstalled();
    return true;
  } catch (e) {
    toast(String(e));
    return false;
  }
}

/** Search-hit shape from a full project (for pages opened by id). */
export function projectToHit(p: any, author = ""): Hit {
  return {
    project_id: p.id, slug: p.slug, title: p.title, description: p.description, author,
    icon_url: p.icon_url, downloads: p.downloads, follows: p.followers ?? 0,
    date_modified: p.updated, date_created: p.published,
    categories: [...(p.categories ?? []), ...(p.loaders ?? [])], display_categories: p.categories,
    versions: p.game_versions ?? [], client_side: p.client_side, server_side: p.server_side,
    project_type: p.project_type, gallery: (p.gallery ?? []).map((g: any) => g.url),
    featured_gallery: p.gallery?.find((g: any) => g.featured)?.url ?? null,
  };
}

/** Where a link in project text goes inside Obelisk; null means the browser. */
export type LinkTarget =
  | { project: string } // Modrinth id/slug or "cf:<id>" (openProjectById)
  | { author: string; cf?: true } // Modrinth user, or a CurseForge member by name
  | { search: ProjectType; query: string };

const MR_TYPES: Record<string, ProjectType> = { mod: "mod", mods: "mod", modpack: "modpack", modpacks: "modpack", resourcepack: "resourcepack",
  resourcepacks: "resourcepack", shader: "shader", shaders: "shader", datapack: "datapack", datapacks: "datapack", plugin: "mod", plugins: "mod" };
const CF_SECTIONS: Record<string, ProjectType> = { "mc-mods": "mod", modpacks: "modpack", "texture-packs": "resourcepack",
  shaders: "shader", "data-packs": "datapack" };

/**
 * Modrinth and CurseForge mod, author and search pages map to Obelisk's own pages; anything else is null.
 * CurseForge's linkout redirects are unwrapped first.
 */
export async function linkTarget(href: string): Promise<LinkTarget | null> {
  let u: URL;
  try { u = new URL(href); } catch { return null; }
  const host = u.hostname.replace(/^www\.|^legacy\./, "");
  const parts = u.pathname.split("/").filter(Boolean);
  if (host === "curseforge.com" && parts[0] === "linkout") {
    const inner = u.searchParams.get("remoteUrl");
    return inner ? linkTarget(decodeURIComponent(inner)) : null;
  }
  if (host === "modrinth.com") {
    if (parts[0] === "user" && parts[1]) return { author: parts[1] };
    const type = MR_TYPES[parts[0]];
    if (type && parts[1] && !parts[0].endsWith("s")) return { project: parts[1] };
    if (parts[0] === "project" && parts[1]) return { project: parts[1] };
    if (type && !parts[1]) return { search: type, query: u.searchParams.get("q") ?? "" };
  }
  if (host === "curseforge.com") {
    if (parts[0] === "projects" && /^\d+$/.test(parts[1] ?? "")) return { project: `cf:${parts[1]}` };
    if (parts[0] === "members" && parts[1]) return { author: decodeURIComponent(parts[1]), cf: true };
    if (parts[0] !== "minecraft") return null;
    const query = u.searchParams.get("search") ?? "";
    if (parts[1] === "search") return { search: CF_SECTIONS[u.searchParams.get("class") ?? ""] ?? "mod", query };
    const type = CF_SECTIONS[parts[1] ?? ""];
    if (type && !parts[2]) return { search: type, query };
    if (type && parts[2] && parts[2] !== "search" && parts[2] !== "category") {
      const id = await api.cfIdBySlug(parts[2]).catch(() => null);
      return id ? { project: `cf:${id}` } : null;
    }
  }
  return null;
}

export async function openProjectById(id: string) {
  try {
    if (id.startsWith("cf:")) {
      const { project } = await api.cfProject(Number(id.slice(3)));
      pushOverlay({ kind: "project", hit: cfToHit(project) });
      return;
    }
    pushOverlay({ kind: "project", hit: projectToHit(await api.project(id)) });
  } catch (e) {
    toast(String(e));
  }
}

/** Re-read Prism's instances (picks up packs imported in Prism) and their icons. */
export async function syncInstances() {
  const list = await api.instances();
  app.instances = list;
  for (const inst of list) api.instanceIcon(inst.iconKey).then((u) => (app.icons[inst.id] = u));
  // Keep the same object when nothing changed, so open pages don't reload on every window focus.
  const keep = (i: Instance | null) => {
    const fresh = i ? list.find((x) => x.id === i.id) ?? null : null;
    return fresh && i && JSON.stringify(fresh) === JSON.stringify(i) ? i : fresh;
  };
  app.current = keep(app.current) ?? list[0] ?? null;
  app.target = keep(app.target);
  return list.length;
}

/** Does the target already have what Connector needs? */
export async function connectorStatus(inst: Instance) {
  const mods = await api.content(inst.id, "mods");
  const has = (re: RegExp) => mods.some((m) => m.enabled && (re.test(m.name) || re.test(m.fileName)));
  return {
    connector: has(/sinytra connector|^connector[-_ ]|connector-\d/i),
    ffapi: has(/forgified fabric api|forgified-fabric-api/i),
    extras: has(/connector extras|connectorextras/i),
  };
}
