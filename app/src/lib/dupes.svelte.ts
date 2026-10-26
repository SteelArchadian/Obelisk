import { invoke } from "@tauri-apps/api/core";

/** The same mod installed twice (usually once from each site): the game won't start like that. */
export type Dupe = { modId: string; files: string[] };
export type DupeState = { dupes: Dupe[]; ignored: boolean };

/** Each profile's duplicates, as last checked (home tiles and the profile view share it). */
export const dupeState = $state<Record<string, DupeState>>({});

/** Whether a profile needs the big warning: duplicates the user hasn't chosen to ignore. */
export const flagged = (id: string) => !!dupeState[id]?.dupes.length && !dupeState[id].ignored;

export async function checkDupes(id: string) {
  try {
    dupeState[id] = await invoke<DupeState>("duplicate_mods", { instanceId: id });
  } catch { /* keep what is shown */ }
}

/** One at a time: a big library shouldn't open every mods folder at once. */
export async function checkAllDupes(ids: string[]) {
  for (const id of ids) await checkDupes(id);
}

/** Let the profile start anyway (`on`), or warn again. Lasts until the duplicates change. */
export async function ignoreDupes(id: string, on: boolean) {
  await invoke("ignore_duplicates", { instanceId: id, on });
  await checkDupes(id);
}
