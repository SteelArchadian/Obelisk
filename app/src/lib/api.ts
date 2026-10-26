import { invoke } from "@tauri-apps/api/core";

export type Instance = {
  id: string; name: string; iconKey: string; minecraft: string;
  loader: string; loaderVersion: string; secondsPlayed: number; lastLaunchMs: number;
};
export type Item = {
  fileName: string; name: string; version: string; authors: string;
  enabled: boolean; source: "curseforge" | "modrinth" | "local"; projectId: string;
  installedMs: number; modifiedMs: number;
  /** Set on rows shown from the trash. */
  trashId?: string;
};
export type World = { folder: string; lastPlayedMs: number; icon: string | null; datapacks: string[] };
export type Kind = "mods" | "resourcepacks" | "shaderpacks";
export type ProjectType = "modpack" | "mod" | "resourcepack" | "shader" | "datapack";

/** A Modrinth search hit (subset of fields we use). */
export type Hit = {
  project_id: string; slug: string; title: string; description: string; author: string;
  icon_url: string | null; downloads: number; follows: number; date_modified: string; date_created: string;
  categories: string[]; display_categories?: string[]; versions: string[]; latest_version?: string;
  client_side: string; server_side: string; project_type: ProjectType;
  featured_gallery?: string | null; gallery?: string[]; color?: number | null;
  /** CurseForge project id when the project is (also) on CurseForge. */
  cf_id?: number;
  /** Which sites host it. */
  sources?: ("modrinth" | "curseforge")[];
};
export type FileInfo = {
  kind: Kind; fileName: string; provider: "modrinth" | "curseforge" | ""; projectId: string; current: string;
  downloads: number; env: "client" | "server" | "both" | ""; iconUrl: string; library?: boolean;
  update: { id: string; name: string; fileName: string; blocked: boolean } | null;
};
export type ManualDownload = {
  world?: string; name: string; page: string; fileName: string; sha1: string; size: number; folder: string; indexToml: string; replaces: string | null;
};
export type Version = {
  id: string; project_id: string; name: string; version_number: string; version_type: string;
  loaders: string[]; game_versions: string[]; date_published: string; downloads: number;
  changelog: string | null; files: { url: string; filename: string; primary: boolean; size: number }[];
  dependencies: { project_id: string | null; dependency_type: string }[];
};
export type Installed = { projectId: string; versionId: string; kind: Kind; fileName: string };
export type Tags = {
  categories: { name: string; project_type: string; header: string; icon: string }[];
  loaders: { name: string; supported_project_types: string[] }[];
  gameVersions: string[];
};
export type Query = {
  query: string; projectType: ProjectType; loaders: string[]; gameVersions: string[];
  categories: string[]; sort: string; offset: number; limit: number;
};

export const TYPES: { type: ProjectType; label: string }[] = [
  { type: "modpack", label: "Modpacks" },
  { type: "mod", label: "Mods" },
  { type: "resourcepack", label: "Resource Packs" },
  { type: "shader", label: "Shaders" },
  { type: "datapack", label: "Data Packs" },
];

export const api = {
  instances: () => invoke<Instance[]>("list_instances"),
  instanceIcon: (iconKey: string) => invoke<string | null>("instance_icon", { iconKey }),
  account: () => invoke<{ name: string; uuid: string } | null>("active_account"),
  search: (query: Query) => invoke<{ hits: Hit[]; total_hits: number }>("mr_search", { query }),
  tags: () => invoke<Tags>("mr_tags"),
  project: (id: string) => invoke<any>("mr_project", { id }),
  versions: (id: string, changelog = false) => invoke<Version[]>("mr_versions", { id, changelog }),
  installed: (instanceId: string) => invoke<Installed[]>("mr_installed", { instanceId }),
  install: (instanceId: string, req: { projectId: string; projectType: string; versionId?: string; loaderOverride?: string }, world?: string) =>
    invoke<string[]>("mr_install", { instanceId, req, world }),
  worlds: (instanceId: string) => invoke<World[]>("list_worlds", { instanceId }),
  user: (name: string) => invoke<{ user: any; projects: any[] }>("mr_user", { name }),
  members: (id: string) => invoke<any[]>("mr_members", { id }),
  content: (instanceId: string, kind: Kind) => invoke<Item[]>("list_content", { instanceId, kind }),
  checkUpdates: (instanceId: string) => invoke<FileInfo[]>("check_updates", { instanceId }),
  applyUpdate: (instanceId: string, info: FileInfo) => invoke<ManualDownload | null>("apply_update", { instanceId, info }),
  awaitDownload: (instanceId: string, job: ManualDownload) => invoke("await_download", { instanceId, job }),
  hasCfKey: () => invoke<boolean>("has_cf_key"),
  cfSearch: (query: object) => invoke<{ data: any[]; pagination: { totalCount: number } }>("cf_search", { query }),
  cfAuthor: (id: number) => invoke<any[]>("cf_author", { id }),
  cfIdBySlug: (slug: string) => invoke<number | null>("cf_id_by_slug", { slug }),
  cfProject: (id: number) => invoke<{ project: any; description: string }>("cf_project", { id }),
  cfFiles: (id: number) => invoke<any[]>("cf_files", { id }),
  cfChangelog: (modId: number, fileId: number) => invoke<string>("cf_changelog", { modId, fileId }),
  cfInstall: (instanceId: string, modId: number, fileId?: number, world?: string) =>
    invoke<{ status: "done"; names: string[] } | ({ status: "blocked"; names: string[] } & ManualDownload)>("cf_install", { instanceId, modId, fileId, world }),
  cfFeatured: () => invoke<any>("cf_featured"),
  importPack: (source: string) => invoke("import_pack", { source }),
};

export function compact(n: number) {
  if (n >= 1e9) return (n / 1e9).toFixed(1).replace(/\.0$/, "") + "B";
  if (n >= 1e6) return (n / 1e6).toFixed(1).replace(/\.0$/, "") + "M";
  if (n >= 1e3) return (n / 1e3).toFixed(1).replace(/\.0$/, "") + "K";
  return String(n);
}

export function ago(input: number | string) {
  const ms = typeof input === "string" ? Date.parse(input) : input;
  if (!ms) return "Never";
  const days = Math.floor((Date.now() - ms) / 86400000);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 30) return `${days} days ago`;
  return new Date(ms).toLocaleDateString();
}

export function initials(name: string) {
  const words = name.replace(/[^\p{L}\p{N}\s]/gu, " ").split(/\s+/).filter(Boolean);
  return words.map((w) => w[0]).join("").slice(0, 2).toUpperCase() || "?";
}

export function envLabel(client: string, server: string) {
  const c = client !== "unsupported", s = server !== "unsupported";
  return c && s ? "Client and Server" : c ? "Client" : s ? "Server" : "";
}

/** Newest game version first, highest-sorted label like "1.21.1 + 3". */
export function versionsLabel(versions: string[]) {
  const releases = versions.filter((v) => /^\d+(\.\d+)*$/.test(v));
  const list = releases.length ? releases : versions;
  if (!list.length) return "";
  const latest = list[list.length - 1];
  return list.length > 1 ? `${latest} + ${list.length - 1}` : latest;
}
