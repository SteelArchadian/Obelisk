import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, type Hit, type ManualDownload, type ProjectType, type Version } from "./api";
import { toast } from "./state.svelte";

const LOADER_BY_TYPE: Record<number, string> = { 1: "forge", 4: "fabric", 5: "quilt", 6: "neoforge" };
const TYPE_BY_CLASS: Record<number, ProjectType> = { 6: "mod", 4471: "modpack", 12: "resourcepack", 6552: "shader", 6945: "datapack" };
const RELEASE: Record<number, string> = { 1: "release", 2: "beta", 3: "alpha" };
const LOADER_NAMES = new Set(["neoforge", "forge", "fabric", "quilt"]);

/** A CurseForge mod in the same shape as a Modrinth search hit. */
export function cfToHit(m: any): Hit {
  const idx: any[] = m.latestFilesIndexes ?? [];
  const loaders = [...new Set(idx.map((i) => LOADER_BY_TYPE[i.modLoader]).filter(Boolean))];
  const versions = [...new Set(idx.map((i) => i.gameVersion as string))]
    .sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
  return {
    project_id: `cf:${m.id}`, cf_id: m.id, sources: ["curseforge"], slug: m.slug, title: m.name,
    description: m.summary ?? "", author: m.authors?.[0]?.name ?? "",
    icon_url: m.logo?.thumbnailUrl ?? m.logo?.url ?? null, downloads: Math.round(m.downloadCount ?? 0),
    follows: m.thumbsUpCount ?? 0, date_modified: m.dateModified, date_created: m.dateReleased,
    categories: [...(m.categories ?? []).map((c: any) => c.slug), ...loaders],
    display_categories: (m.categories ?? []).map((c: any) => c.slug),
    versions, client_side: "", server_side: "", project_type: TYPE_BY_CLASS[m.classId] ?? "mod",
    gallery: (m.screenshots ?? []).map((s: any) => s.url),
    featured_gallery: m.screenshots?.[0]?.url ?? null,
  };
}

/** A CurseForge file in the same shape as a Modrinth version (changelog is fetched on demand). */
export function cfFileToVersion(f: any, modId: number): Version {
  const gv: string[] = f.gameVersions ?? [];
  return {
    id: String(f.id), project_id: `cf:${modId}`, name: f.displayName, version_number: f.fileName,
    version_type: RELEASE[f.releaseType] ?? "release",
    loaders: gv.map((v) => v.toLowerCase()).filter((v) => LOADER_NAMES.has(v)),
    game_versions: gv.filter((v) => /^\d/.test(v)), date_published: f.fileDate, downloads: f.downloadCount ?? 0,
    changelog: null, files: [{ url: f.downloadUrl ?? "", filename: f.fileName, primary: true, size: f.fileLength }],
    dependencies: (f.dependencies ?? []).filter((d: any) => d.relationType === 3).map((d: any) => ({ project_id: `cf:${d.modId}`, dependency_type: "required" })),
  };
}

/** Normalised title for matching the same project across both sites. */
export const sameKey = (title: string) => title.toLowerCase().replace(/\[.*?\]|\(.*?\)/g, "").replace(/[^a-z0-9]/g, "");

/** Merge CurseForge hits into Modrinth hits: one card per project, tagged with every site that hosts it. */
export function mergeHits(mr: Hit[], cf: Hit[]): Hit[] {
  const byKey = new Map(mr.map((h) => [sameKey(h.title), { ...h, sources: ["modrinth"] as Hit["sources"] }]));
  const extra: Hit[] = [];
  for (const c of cf) {
    const m = byKey.get(sameKey(c.title)) ?? [...byKey.values()].find((h) => h.slug === c.slug);
    if (m) {
      m.cf_id = c.cf_id;
      m.sources = ["modrinth", "curseforge"] as Hit["sources"];
      m.downloads = Math.max(m.downloads, c.downloads);
    } else {
      extra.push(c);
    }
  }
  return [...byKey.values(), ...extra];
}

let cfSearchOk: boolean | null = null;
/** Whether the saved key may use CurseForge search (some keys can't). Probed once per session. */
export async function cfSearchAvailable() {
  if (cfSearchOk !== null) return cfSearchOk;
  if (!(await api.hasCfKey())) return false;
  try {
    await api.cfSearch({ query: "", projectType: "mod", loader: "", gameVersion: "", sort: "", offset: 0, limit: 1 });
    cfSearchOk = true;
  } catch {
    cfSearchOk = false;
  }
  return cfSearchOk;
}
export const resetCfSearch = () => (cfSearchOk = null);

let listening = false;
/**
 * The author only allows downloads on curseforge.com: open the file page in the browser and
 * let Obelisk pick the file up from Downloads when it lands.
 */
export async function manualDownload(instanceId: string, job: ManualDownload, onDone?: () => void) {
  if (!listening) {
    listening = true;
    await listen<{ name: string; ok: boolean; error: string | null }>("manual-download", (e) => {
      toast(e.payload.ok ? `${e.payload.name} downloaded and installed.` : `${e.payload.name}: ${e.payload.error}`);
      doneCallbacks.splice(0).forEach((f) => f());
    });
  }
  if (onDone) doneCallbacks.push(onDone);
  await api.awaitDownload(instanceId, job);
  await openUrl(job.page);
  toast(`${job.name}'s author only allows downloads on curseforge.com. Click Download there and Obelisk will finish the install.`);
}
const doneCallbacks: (() => void)[] = [];
