import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { settings, saveSettings } from "./settings.svelte";
import { syncInstances } from "./install";
import { toast } from "./state.svelte";

/** Commands that only look: no toast for these. */
const READS = /^(get_|list_|read_|has_|cf_search|mr_search|ui\.settings\.get$)|(_get|_installed|_running|_status|_defaults)$/;

/**
 * The assistant's hands (control API "app", see bridge.rs): run an app command here, as a click
 * would, so this window's lists and settings stay in step, and send the answer back.
 */
export function startAssistantBridge() {
  listen<{ id: number; command: string; args: Record<string, unknown> }>("control-invoke", async (e) => {
    const { id, command, args } = e.payload;
    let ok = true;
    let value: unknown = null;
    try {
      if (command === "ui.settings.get") value = $state.snapshot(settings);
      else if (command === "ui.settings.set") {
        Object.assign(settings, args);
        await saveSettings();
        value = $state.snapshot(settings);
      } else if (command === "ui.refresh") {
        await syncInstances();
        value = true;
      }
      else value = await invoke(command, args);
      if (!READS.test(command)) toast(`Assistant ran ${command}.`);
    } catch (err) {
      ok = false;
      value = String(err);
    }
    await invoke("control_reply", { id, ok, value: value ?? null }).catch(() => {});
  }).catch(() => {});
}
