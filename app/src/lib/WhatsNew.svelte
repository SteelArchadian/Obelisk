<script lang="ts">
  // Once, right after an update: what the new version changed, in a few lines, and a link to the full notes.
  import { openUrl } from "@tauri-apps/plugin-opener";

  let { version, notes, close }: { version: string; notes: string; close: () => void } = $props();

  // Release notes are a few sentences or lines: one point each.
  const points = $derived(
    (notes.includes("\n") ? notes.split("\n") : notes.split(/(?<=\.)\s+(?=[A-Z])/))
      .map((l) => l.replace(/^[-*•]\s*/, "").trim()).filter(Boolean),
  );
  const full = () => openUrl(`https://github.com/SteelArchadian/Obelisk/releases/tag/v${version}`);
</script>

<div class="back" role="presentation" onclick={close}>
  <div class="dialog" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && close()}>
    <h3>What's new in Obelisk {version}</h3>
    {#if points.length}
      <ul>{#each points as p}<li>{p}</li>{/each}</ul>
    {/if}
    <div class="row">
      <button class="link" onclick={full}>Full release notes</button>
      <span class="spacer"></span>
      <button class="primary" onclick={close}>Got it</button>
    </div>
  </div>
</div>

<style>
  .back { position: fixed; inset: 0; background: rgb(0 0 0 / 0.5); display: grid; place-items: center; z-index: 50; }
  .dialog { width: min(440px, 92vw); max-height: 70vh; overflow-y: auto; background: var(--panel-2); border: 1px solid var(--line); border-radius: 10px; padding: 20px 22px; display: flex; flex-direction: column; gap: 12px; outline: none; }
  h3 { margin: 0; font-size: 16px; }
  ul { margin: 0; padding-left: 18px; display: flex; flex-direction: column; gap: 6px; color: var(--text-2); font-size: 13px; line-height: 1.45; }
  .row { display: flex; align-items: center; gap: 10px; }
  .spacer { flex: 1; }
  .link { background: none; border: 0; padding: 0; color: var(--accent); font-size: 13px; }
  .link:hover { text-decoration: underline; }
  .primary { color: #fff; background: var(--accent); border: 0; border-radius: 4px; padding: 7px 18px; font-weight: 600; }
</style>
