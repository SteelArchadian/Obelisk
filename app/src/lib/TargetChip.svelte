<script lang="ts">
  import { initials, type Hit } from "./api";
  import { app } from "./state.svelte";
  import { setTarget } from "./install";

  let { hit = null }: { hit?: Hit | null } = $props();
  let open = $state(false);
</script>

<svelte:window onclick={() => (open = false)} />

{#if hit?.project_type !== "modpack"}
  <div class="chip">
    <button class="cur" onclick={(e) => { e.stopPropagation(); open = !open; }} title="Where Install puts things">
      {#if app.target}
        <span class="pic">{#if app.icons[app.target.id]}<img src={app.icons[app.target.id]} alt="" />{:else}{initials(app.target.name)}{/if}</span>
        Installing to <strong>{app.target.name}</strong>
      {:else}
        Installing to <strong>…choose a profile</strong>
      {/if}
      <span class="caret">▾</span>
    </button>
    {#if open}
      <div class="menu" role="menu" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
        {#each app.instances as inst (inst.id)}
          <button class:on={app.target?.id === inst.id} onclick={() => { setTarget(inst); open = false; }}>
            <span class="pic">{#if app.icons[inst.id]}<img src={app.icons[inst.id]} alt="" />{:else}{initials(inst.name)}{/if}</span>
            <span>{inst.name}<small>{inst.loader} {inst.minecraft}</small></span>
          </button>
        {/each}
        <button onclick={() => { setTarget(null); open = false; }}><span class="pic">?</span><span>Ask each time</span></button>
      </div>
    {/if}
  </div>
{/if}

<style>
  .chip { position: relative; }
  .cur { display: flex; align-items: center; gap: 8px; background: var(--panel); border: 1px solid var(--line); border-radius: 6px; padding: 4px 10px 4px 4px; color: var(--muted); font-size: 13px; }
  .cur:hover { border-color: var(--muted); }
  .cur strong { color: var(--text); }
  .caret { font-size: 10px; }
  .pic { width: 24px; height: 24px; border-radius: 4px; overflow: hidden; background: var(--panel-2); display: grid; place-items: center; font-size: 10px; font-weight: 700; flex: none; }
  .pic img { width: 100%; height: 100%; object-fit: cover; }
  .menu { position: absolute; top: calc(100% + 4px); left: 0; z-index: 25; min-width: 240px; background: var(--panel-2); border: 1px solid var(--line); border-radius: 8px; padding: 4px; box-shadow: 0 10px 30px rgb(0 0 0 / 0.45); }
  .menu button { display: flex; align-items: center; gap: 10px; width: 100%; background: none; border: 0; padding: 6px 8px; border-radius: 5px; text-align: left; }
  .menu button:hover, .menu button.on { background: var(--panel); }
  .menu small { display: block; color: var(--muted); font-size: 11.5px; }
</style>
