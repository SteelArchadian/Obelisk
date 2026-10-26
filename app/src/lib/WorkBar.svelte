<script lang="ts">
  // A small bar at the bottom of the window while something slow runs (copying a pack, a backup, a restore):
  // what it's doing and how far. Jobs that finish within half a second never show it.
  import { listen } from "@tauri-apps/api/event";

  type Work = { id: number; label: string; done: number; total: number; end: boolean };
  let jobs = $state<Record<number, Work & { since: number }>>({});
  let now = $state(Date.now());
  const shown = $derived(Object.values(jobs).filter((j) => now - j.since >= 500).at(-1));

  $effect(() => {
    const t = setInterval(() => (now = Date.now()), 250);
    const un = listen<Work>("work", (e) => {
      const w = e.payload;
      if (w.end) delete jobs[w.id];
      else jobs[w.id] = { ...w, since: jobs[w.id]?.since ?? Date.now() };
    });
    return () => { clearInterval(t); un.then((u) => u()); };
  });
</script>

{#if shown}
  <div class="work" role="status">
    <span class="label">{shown.label}…{#if shown.total} <b>{Math.floor((shown.done / shown.total) * 100)}%</b>{/if}</span>
    <div class="bar" class:busy={!shown.total}><i style:width={shown.total ? `${(shown.done / shown.total) * 100}%` : ""}></i></div>
  </div>
{/if}

<style>
  .work {
    position: fixed; left: 50%; bottom: 14px; transform: translateX(-50%); z-index: 60;
    width: min(360px, calc(100vw - 32px)); padding: 8px 12px; border-radius: 8px;
    background: var(--panel); border: 1px solid var(--line); box-shadow: 0 4px 16px rgb(0 0 0 / 0.35);
    display: grid; gap: 6px; font-size: 12px; pointer-events: none;
  }
  .label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; opacity: 0.85; }
  .label b { color: var(--text); font-weight: 600; }
  .bar { height: 4px; background: color-mix(in srgb, var(--line) 60%, transparent); border-radius: 2px; overflow: hidden; }
  .bar i { display: block; height: 100%; background: var(--accent); transition: width 0.2s; }
  .bar.busy i { width: 30%; animation: slide 1.2s ease-in-out infinite; }
  @keyframes slide { from { transform: translateX(-100%); } to { transform: translateX(340%); } }
</style>
