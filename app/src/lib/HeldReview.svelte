<script lang="ts">
  // Files Obelisk blocked in a pack (they act like programs that steal accounts or passwords), shown right after a
  // join, import or friend's change brings one, and from the pack's page. Allowing one opens a Windows box that
  // only the user can answer (share_allow_held); Keep it blocked stops the asking.
  import { invoke } from "@tauri-apps/api/core";
  import { app, toast, toastError } from "./state.svelte";

  type Held = { path: string; size: number; from: string; reasons: string[]; kept: boolean };
  let { id, onclose }: { id: string; onclose: () => void } = $props();
  let list = $state<Held[]>([]);
  let busy = $state("");
  const name = $derived(app.instances.find((i) => i.id === id)?.name ?? id);
  const file = (h: Held) => h.path.split("/").pop();

  const load = () => invoke<Held[]>("held_list", { instanceId: id }).then((l) => { list = l; if (!l.length) onclose(); }).catch((e) => toastError(e));
  $effect(() => { void id; load(); });

  async function allow(h: Held) {
    busy = h.path;
    try {
      if (await invoke<boolean>("share_allow_held", { instanceId: id, path: h.path })) {
        toast(`${file(h)} is in ${name} now. Undo anytime from Backups.`);
        window.dispatchEvent(new Event("share-changed"));
        await load();
      }
    } catch (e) { toastError(e); }
    busy = "";
  }
  async function keep(h: Held) {
    busy = h.path;
    try { await invoke("held_keep", { instanceId: id, path: h.path }); await load(); } catch (e) { toastError(e); }
    busy = "";
  }
</script>

<div class="back" role="presentation" onclick={onclose}>
  <div class="dialog" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && onclose()}>
    <button class="x" onclick={onclose} aria-label="Close">✕</button>
    <h3>Obelisk blocked {list.length === 1 ? "a file" : `${list.length} files`} in {name}</h3>
    <p class="muted small">They act like programs that steal accounts or passwords, so they weren't installed. Some real mods do this
      for their own sign-in features: allow one only if you know it and trust where it came from.</p>
    <ul>
      {#each list as h (h.path)}
        <li>
          <div class="what"><b>{file(h)}</b> <span class="muted small">{h.path} · from {h.from || "the person who shared the pack"}</span>
            <span class="small why">It {h.reasons.join(", and it ")}.</span></div>
          <div class="acts">
            {#if h.kept}<span class="muted small">Kept blocked</span>{:else}<button class="ghost" disabled={!!busy} onclick={() => keep(h)}>Keep it blocked</button>{/if}
            <button class="primary" disabled={!!busy} onclick={() => allow(h)} title="Windows asks you to confirm">Review & allow…</button>
          </div>
        </li>
      {/each}
    </ul>
  </div>
</div>

<style>
  .back { position: fixed; inset: 0; background: rgb(0 0 0 / 0.6); display: grid; place-items: center; z-index: 45; }
  .dialog { position: relative; width: min(600px, 94vw); max-height: 90vh; overflow-y: auto; background: var(--panel-2); border: 1px solid var(--line); border-radius: 10px; padding: 22px 26px; display: flex; flex-direction: column; gap: 12px; outline: none; }
  .x { position: absolute; top: 10px; right: 10px; background: none; border: 0; color: var(--muted); padding: 4px 8px; }
  h3 { margin: 0; font-size: 17px; padding-right: 24px; }
  p { margin: 0; }
  .muted { color: var(--muted); }
  .small { font-size: 12px; }
  ul { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 10px; }
  li { display: flex; gap: 12px; align-items: center; border: 1px solid var(--line); border-radius: 8px; padding: 10px 12px; flex-wrap: wrap; }
  .what { flex: 1; min-width: 220px; display: flex; flex-direction: column; gap: 3px; overflow-wrap: anywhere; }
  .why { color: var(--err-text); }
  .acts { display: flex; gap: 8px; align-items: center; }
</style>
