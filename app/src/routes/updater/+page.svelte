<script lang="ts">
  // The small window a start opens first (like Discord's): checks for an update and installs it before Obelisk
  // opens. Obelisk starts again by itself after an update; otherwise its own window takes this one's place.
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  let status = $state("Checking for updates…");
  let done = $state(0);
  let total = $state(0);

  onMount(() => {
    const unFound = listen<{ version: string }>("self-update-found", (e) => (status = `Installing Obelisk ${e.payload.version}…`));
    const unProgress = listen<{ done: number; total: number }>("self-update-progress", (e) => { done = e.payload.done; total = e.payload.total; });
    (async () => {
      const installing = await invoke<boolean>("startup_update").catch(() => false);
      if (!installing) {
        status = "Starting…";
        await invoke("startup_done");
      }
    })();
    return () => { unFound.then((u) => u()); unProgress.then((u) => u()); };
  });
</script>

<main data-tauri-drag-region>
  <svg viewBox="0 0 64 96" width="56" height="84" aria-hidden="true" data-tauri-drag-region>
    <path d="M32 4 L44 22 L40 88 H24 L20 22 Z" fill="none" stroke="currentColor" stroke-width="4" stroke-linejoin="round" />
    <path d="M20 22 H44" stroke="currentColor" stroke-width="4" />
  </svg>
  <h1 data-tauri-drag-region>Obelisk</h1>
  <p data-tauri-drag-region>{status}</p>
  <div class="bar" class:busy={!total}><i style:width={total ? `${(done / total) * 100}%` : ""}></i></div>
</main>

<style>
  :global(html), :global(body) { margin: 0; height: 100%; background: #16171a; overflow: hidden; }
  main { height: 100%; box-sizing: border-box; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 14px; padding: 28px; color: #e9e9ec; font-family: "Segoe UI", system-ui, sans-serif; user-select: none; cursor: default; }
  svg { color: #9b87f5; }
  h1 { margin: 0; font-size: 22px; font-weight: 700; letter-spacing: 0.02em; }
  p { margin: 0; font-size: 13px; color: #9a9ba3; }
  .bar { width: 180px; height: 4px; border-radius: 2px; background: #2a2b31; overflow: hidden; }
  .bar i { display: block; height: 100%; background: #9b87f5; transition: width 0.2s; }
  .bar.busy i { width: 35%; animation: slide 1.1s ease-in-out infinite; }
  @keyframes slide { from { transform: translateX(-100%); } to { transform: translateX(300%); } }
</style>
