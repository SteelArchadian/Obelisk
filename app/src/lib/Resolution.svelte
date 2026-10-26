<script lang="ts">
  let { width = $bindable(), height = $bindable(), maximized = $bindable() }: { width: number; height: number; maximized: boolean } = $props();

  const PRESETS: [number, number, string][] = [
    [854, 480, "854 × 480 (Minecraft default)"],
    [1280, 720, "1280 × 720 (HD)"],
    [1366, 768, "1366 × 768"],
    [1600, 900, "1600 × 900"],
    [1920, 1080, "1920 × 1080 (Full HD)"],
    [2560, 1440, "2560 × 1440 (QHD)"],
    [3440, 1440, "3440 × 1440 (Ultrawide)"],
    [3840, 2160, "3840 × 2160 (4K)"],
  ];

  let choice = $derived(maximized ? "max" : PRESETS.find(([w, h]) => w === width && h === height) ? `${width}x${height}` : "custom");
  let custom = $state(false);

  function pick(v: string) {
    custom = v === "custom";
    maximized = v === "max";
    const p = PRESETS.find(([w, h]) => `${w}x${h}` === v);
    if (p) [width, height] = [p[0], p[1]];
  }
</script>

<div class="res">
  <select class="select" value={custom ? "custom" : choice} onchange={(e) => pick(e.currentTarget.value)} aria-label="Game window size">
    <option value="max">Maximized</option>
    {#each PRESETS as [w, h, label] (label)}<option value="{w}x{h}">{label}</option>{/each}
    <option value="custom">Custom…</option>
  </select>
  {#if (custom || choice === "custom") && !maximized}
    <input class="num" type="number" min="320" bind:value={width} aria-label="Width" /> ×
    <input class="num" type="number" min="240" bind:value={height} aria-label="Height" />
  {/if}
</div>

<style>
  .res { display: flex; align-items: center; gap: 8px; font-weight: 400; }
  .num { width: 86px; background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 5px; padding: 5px 6px; font: inherit; }
</style>
