import type { Hit } from "./api";

/** The project's page on its site (CurseForge for a CurseForge-only hit). */
export function projectUrl(hit: Pick<Hit, "project_id" | "project_type" | "slug" | "cf_id">): string {
  if (hit.project_id.startsWith("cf:")) return `https://www.curseforge.com/projects/${hit.cf_id ?? hit.project_id.slice(3)}`;
  return `https://modrinth.com/${hit.project_type}/${hit.slug || hit.project_id}`;
}

/** The author's page on the same site as the project. */
export function authorUrl(hit: Pick<Hit, "project_id" | "author">): string {
  const name = encodeURIComponent(hit.author);
  return hit.project_id.startsWith("cf:") ? `https://www.curseforge.com/members/${name}/projects` : `https://modrinth.com/user/${name}`;
}
