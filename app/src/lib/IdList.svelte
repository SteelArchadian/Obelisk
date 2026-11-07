<script lang="ts">
  import type { Value } from "./configmodel";

  /** A config list of mob, block or item ids: chips with real names, plus a searchable picker of everything in the pack. */
  let { value, kind, options, onchange }: {
    value: Value[];
    kind: "entity" | "block" | "item";
    options: [string, string][];
    onchange: (next: Value[]) => void;
  } = $props();

  let q = $state("");
  let open = $state(false);
  let hi = $state(0);
  let listEl = $state<HTMLDivElement>();

  const noun = $derived({ entity: "mobs", block: "blocks", item: "items" }[kind]);
  const names = $derived(new Map(options));
  const picked = $derived(new Set(value.filter((v): v is string => typeof v === "string")));

  let matches = $derived.by(() => {
    const words = q.trim().toLowerCase().split(/\s+/).filter(Boolean);
    const hits = options.filter(([id, name]) => words.every((w) => `${name} ${id}`.toLowerCase().includes(w)));
    const first = words[0] ?? "";
    // Names that start with what you typed go first, then vanilla before modded.
    const rank = ([id, name]: [string, string]) => (name.toLowerCase().startsWith(first) ? 0 : 2) + (id.startsWith("minecraft:") ? 0 : 1);
    return hits.sort((a, b) => rank(a) - rank(b) || a[1].localeCompare(b[1])).slice(0, 300);
  });

  $effect(() => { q; hi = 0; });

  /** "Zombie Horse", "zombie horse" or "zombie_horse" -> minecraft:zombie_horse; "#logs" -> #minecraft:logs. */
  function format(t: string): string {
    const byName = options.find(([, n]) => n.toLowerCase() === t.toLowerCase());
    if (byName) return byName[0];
    const tag = t.startsWith("#") ? "#" : "";
    let id = t.slice(tag.length).trim().toLowerCase().replace(/\s+/g, "_");
    if (!id.includes(":")) id = `minecraft:${id}`;
    return tag + id;
  }

  /** Leaving the box only adds what you typed if it names something real; half-typed searches aren't added. */
  const known = (t: string) => !!t.trim() && names.has(format(t.trim()).replace(/^#/, ""));

  function toggle(id: string) {
    onchange(picked.has(id) ? value.filter((v) => v !== id) : [...value, id]);
    q = "";
  }

  function addTyped() {
    const t = q.trim();
    if (!t) return;
    const id = format(t);
    if (!picked.has(id)) onchange([...value, id]);
    q = "";
  }

  function key(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      open = true;
      hi = Math.max(0, Math.min(matches.length - 1, hi + (e.key === "ArrowDown" ? 1 : -1)));
      listEl?.querySelector(`[data-i="${hi}"]`)?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (open && q.trim() && matches[hi]) toggle(matches[hi][0]);
      else addTyped();
    } else if (e.key === "Escape" && open) {
      e.stopPropagation();
      open = false;
    } else if (e.key === "Backspace" && !q && value.length) {
      onchange(value.slice(0, -1));
    }
  }

  const label = (v: Value) => (typeof v === "string" ? names.get(v.replace(/^#/, "")) : undefined);
  const ns = (v: Value) => (typeof v === "string" && !v.startsWith("minecraft:") && v.includes(":") ? v.split(":")[0].replace(/^#/, "") : "");
</script>

<div class="wrap">
  <div class="list">
    {#each value as item, i (i)}
      {@const name = label(item)}
      <span class="chip" class:unknown={!name && typeof item === "string" && !item.startsWith("#")} title={typeof item === "string" ? item : JSON.stringify(item)}>
        {name ?? (typeof item === "string" ? item : JSON.stringify(item))}{#if name && ns(item)}<small>{ns(item)}</small>{/if}
        <button aria-label="Remove {name ?? item}" onclick={() => onchange(value.filter((_, j) => j !== i))}>✕</button>
      </span>
    {/each}
    <input class="add" placeholder="Search {noun}…" bind:value={q} onfocus={() => (open = true)} oninput={() => (open = true)}
      onblur={() => { open = false; if (known(q)) addTyped(); }} onkeydown={key} role="combobox" aria-expanded={open} aria-controls="idl" aria-autocomplete="list" />
  </div>
  {#if open}
    <div class="drop" id="idl" role="listbox" bind:this={listEl}>
      {#each matches as [id, name], i (id)}
        <button data-i={i} class:hi={i === hi} class:on={picked.has(id)} role="option" aria-selected={picked.has(id)}
          onmousedown={(e) => { e.preventDefault(); toggle(id); }} onmouseenter={() => (hi = i)}>
          <span class="box">{picked.has(id) ? "✓" : ""}</span>
          <span class="nm">{name || id}</span>
          <small>{id}</small>
        </button>
      {:else}
        <p>{q.trim() ? `No ${noun} match. Press Enter to add ${format(q.trim())}.` : `No ${noun} found in this pack.`}</p>
      {/each}
    </div>
  {/if}
</div>

<style>
  .wrap { flex: 1; position: relative; min-width: 0; }
  .list { display: flex; flex-wrap: wrap; gap: 4px; background: var(--bg); border: 1px solid var(--line); border-radius: 5px; padding: 4px; min-width: 0; max-height: 120px; overflow-y: auto; }
  .list:focus-within { border-color: var(--accent); }
  .chip { display: inline-flex; align-items: center; gap: 4px; background: var(--panel); border-radius: 4px; padding: 2px 4px 2px 8px; font-size: 12px; max-width: 100%; overflow: hidden; }
  .chip small { color: var(--muted); font-size: 10.5px; }
  .chip.unknown { outline: 1px dashed var(--line); }
  .chip button { background: none; border: 0; color: var(--muted); padding: 0 3px; font-size: 11px; }
  .chip button:hover { color: var(--danger); }
  .add { flex: 1; min-width: 90px; background: none; border: 0; color: var(--text); padding: 3px 4px; font-size: 12px; outline: none; }
  .drop { position: absolute; z-index: 5; left: 0; right: 0; top: calc(100% + 4px); max-height: 260px; overflow-y: auto; background: var(--panel-2); border: 1px solid var(--line); border-radius: 6px; box-shadow: 0 8px 24px rgb(0 0 0 / 0.45); padding: 4px; }
  .drop button { width: 100%; display: flex; align-items: center; gap: 8px; background: none; border: 0; border-radius: 4px; padding: 5px 8px; text-align: left; color: var(--text-2); font-size: 12.5px; }
  .drop button.hi { background: var(--panel); color: var(--text); }
  .drop button.on .nm { color: var(--text); font-weight: 600; }
  .box { width: 14px; height: 14px; flex: none; border: 1px solid var(--line); border-radius: 3px; display: grid; place-items: center; font-size: 10px; color: var(--accent); }
  .on .box { border-color: var(--accent); }
  .nm { flex: none; }
  .drop small { margin-left: auto; color: var(--muted); font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .drop p { margin: 0; padding: 8px; color: var(--muted); font-size: 12px; }
</style>
