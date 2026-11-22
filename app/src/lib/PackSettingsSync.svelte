<script lang="ts">
  // Settings kept in step between the play packs (cfgsync.rs): a setting changed in one pack's game reaches the
  // others. A change made while that pack's game was closed (a chat or a tool) waits here until it is approved.
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { toast, toastError } from "./state.svelte";

  type Pending = { id: number; pack: string; file: string; key: string; old: string | null; new: string; seen: string };
  type Status = { packs: string[]; paused: boolean; pending: Pending[]; waiting_for_game_to_close: Record<string, Record<string, string>>; mod_config_files_tracked: number; log: string[] };
  let st = $state<Status | null>(null);
  let busy = $state(false);
  let showLog = $state(false);

  const call = (args: Record<string, unknown>) => invoke<Status>("config_sync", { args });
  const load = () => call({}).then((s) => (st = s)).catch(() => (st = null));
  onMount(() => { load(); const t = setInterval(load, 20000); return () => clearInterval(t); });

  async function act(action: "approve" | "reject", p?: Pending) {
    busy = true;
    try {
      st = await call(p ? { action, id: p.id } : { action, all: true });
      toast(action === "approve" ? "Approved: it goes to the other packs (each one once its game is closed)." : "Rejected: it stays in that pack only.");
    } catch (e) { toastError(e); }
    busy = false;
  }
  const where = (p: Pending) => (p.file === "options.txt" ? "game options" : p.file.replace(/^config\//, ""));
  const waiting = $derived(st ? Object.keys(st.waiting_for_game_to_close).map((k) => k.split("|")[0]) : []);
</script>

{#if st && st.packs.length > 1}
  <h3>Settings shared between packs</h3>
  <p class="muted small">
    A setting you change in {st.packs.join(", ")} (game options and {st.mod_config_files_tracked} mod config files) reaches the others.
    Each file is backed up before it's written, and a pack's files are never changed while its game is open.
    {#if st.paused}<strong>Paused right now.</strong>{/if}
  </p>
  {#if st.pending.length}
    <div class="pending">
      <div class="head">
        <strong>{st.pending.length} change{st.pending.length === 1 ? "" : "s"} made while the game was closed</strong>
        <span class="muted small">Probably a chat or a tool, not you in game. Approve to share it with the other packs.</span>
        {#if st.pending.length > 1}
          <span class="all"><button class="ghost" disabled={busy} onclick={() => act("approve")}>Approve all</button>
          <button class="ghost" disabled={busy} onclick={() => act("reject")}>Reject all</button></span>
        {/if}
      </div>
      {#each st.pending as p (p.id)}
        <div class="row">
          <span><strong>{p.pack}</strong> · {where(p)} · <code>{p.key}</code>:
            <code>{p.old ?? "(none)"}</code> → <code>{p.new}</code> <small class="muted">{p.seen}</small></span>
          <button disabled={busy} onclick={() => act("approve", p)}>Approve</button>
          <button class="ghost" disabled={busy} onclick={() => act("reject", p)}>Keep in {p.pack} only</button>
        </div>
      {/each}
    </div>
  {:else}
    <p class="small">Nothing waiting for approval.</p>
  {/if}
  {#if waiting.length}<p class="small muted">Waiting for {[...new Set(waiting)].join(", ")} to close before writing.</p>{/if}
  <button class="link" onclick={() => (showLog = !showLog)}>{showLog ? "Hide" : "Show"} recent activity</button>
  {#if showLog}<pre class="log">{st.log.join("\n") || "Nothing yet."}</pre>{/if}
{/if}

<style>
  .pending { border: 1px solid var(--border, #444); border-radius: 8px; padding: 10px; margin: 8px 0; display: grid; gap: 8px; }
  .head { display: flex; flex-wrap: wrap; gap: 8px; align-items: baseline; }
  .all { margin-left: auto; display: flex; gap: 6px; }
  .row { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .row > span { flex: 1 1 320px; overflow-wrap: anywhere; }
  .log { max-height: 220px; overflow: auto; font-size: 12px; white-space: pre-wrap; }
</style>
