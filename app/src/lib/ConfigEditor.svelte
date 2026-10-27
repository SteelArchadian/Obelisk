<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { toast } from "./state.svelte";
  import { parse, check, same, label, type Setting, type Value } from "./configmodel";

  let { instanceId, fileName, name, onclose }: { instanceId: string; fileName: string; name: string; onclose: () => void } = $props();

  let files = $state<{ path: string; size: number }[] | null>(null);
  let path = $state("");
  let text = $state("");
  let saved = $state("");
  let defaults = $state<string | null>(null);
  let raw = $state(false);
  let filter = $state("");
  let busy = $state(false);
  let error = $state("");
  /** Folders the user opened (Configured-style: closed until you click in). */
  let open = $state<Set<string>>(new Set());
  /** Rejected input per setting id. */
  let invalid = $state<Map<string, string>>(new Map());
  /** List settings with their "add" box text. */
  let adding = $state<Record<string, string>>({});

  let dirty = $derived(text !== saved);
  let doc = $derived(path ? parse(path, text, defaults) : null);
  let formOk = $derived(!!doc);

  $effect(() => {
    invoke<{ path: string; size: number }[]>("mod_configs", { instanceId, fileName })
      .then((f) => { files = f; if (f[0]) openFile(f[0].path); })
      .catch((e) => (error = String(e)));
  });

  async function openFile(p: string) {
    if (dirty && !confirm("You have unsaved changes. Discard them?")) return;
    error = "";
    try {
      text = saved = await invoke<string>("read_config", { instanceId, path: p });
      defaults = await invoke<string | null>("config_default", { instanceId, path: p });
      path = p;
      raw = false;
      invalid = new Map();
      open = new Set();
      filter = "";
    } catch (e) {
      error = String(e);
    }
  }

  async function save() {
    if (invalid.size) return toast("Fix the highlighted values first.");
    busy = true;
    try {
      await invoke("write_config", { instanceId, path, text });
      saved = text;
      toast("Saved. Changes apply the next time the game starts.");
    } catch (e) {
      error = String(e);
    }
    busy = false;
  }

  async function restoreDefault() {
    if (!confirm("Put this whole file back to how it was before Obelisk first opened it? Your current version is kept in the backups.")) return;
    try {
      text = saved = await invoke<string>("restore_config_default", { instanceId, path });
      invalid = new Map();
      toast("Restored the default for this file.");
    } catch (e) {
      error = String(e);
    }
  }

  function setValue(s: Setting, v: Value) {
    const err = check(s, v);
    const next = new Map(invalid);
    if (err) {
      next.set(s.id, err);
      invalid = next;
      return;
    }
    next.delete(s.id);
    invalid = next;
    text = doc!.set(text, s, v);
  }

  function numberInput(s: Setting, input: string) {
    const v = input.trim();
    setValue(s, v === "" ? NaN : Number(v));
  }

  function rawInput(s: Setting, input: string) {
    if (doc?.format === "json") {
      try {
        setValue(s, JSON.parse(input));
      } catch {
        invalid = new Map(invalid).set(s.id, "Not valid JSON");
      }
    } else {
      setValue(s, input.trim());
    }
  }

  function addItem(s: Setting) {
    const t = (adding[s.id] ?? "").trim();
    if (!t) return;
    const list = (s.value as Value[]) ?? [];
    const numeric = list.length > 0 && list.every((x) => typeof x === "number");
    setValue(s, [...list, numeric && !Number.isNaN(Number(t)) ? Number(t) : t]);
    adding[s.id] = "";
  }

  const show = (v: Value) => (typeof v === "string" ? v : JSON.stringify(v));

  /** Matching settings grouped by folder ("" = top level). */
  let groups = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    const list = (doc?.settings ?? []).filter((s) => !q || `${s.path.join(" ")} ${s.key} ${label(s.key)} ${s.help}`.toLowerCase().includes(q));
    const m = new Map<string, Setting[]>();
    for (const s of list) {
      const k = s.path.join(" › ");
      m.set(k, [...(m.get(k) ?? []), s]);
    }
    return [...m.entries()];
  });

  function toggle(k: string) {
    const n = new Set(open);
    n.has(k) ? n.delete(k) : n.add(k);
    open = n;
  }

  /** "create-client.toml" -> "Client", like Configured's file list. */
  function fileLabel(p: string) {
    const f = p.split("/").pop()!.replace(/\.[^.]+$/, "");
    const side = /[-_](client|common|server|startup)$/i.exec(f)?.[1];
    const world = /^saves\/([^/]+)\/serverconfig\//.exec(p)?.[1];
    if (world) return `Server · ${world}`;
    if (p.startsWith("defaultconfigs/")) return `${side ? label(side.toLowerCase()) : f} (new worlds)`;
    return side ? label(side.toLowerCase()) : label(f);
  }

  function tryClose() {
    if (!dirty || confirm("You have unsaved changes. Close anyway?")) onclose();
  }
</script>

<svelte:window onkeydown={(e) => {
  if (e.key === "Escape") tryClose();
  if (e.key === "s" && e.ctrlKey && dirty) { e.preventDefault(); save(); }
}} />

<div class="back" role="presentation">
  <div class="dialog" role="dialog" aria-label="Configs for {name}">
    <header>
      <h3>{name}</h3>
      <button class="x" onclick={tryClose} aria-label="Close">✕</button>
    </header>
    <div class="body">
      <nav class="files">
        {#each files ?? [] as f (f.path)}
          <button class:on={path === f.path} onclick={() => openFile(f.path)} title={f.path}><strong>{fileLabel(f.path)}</strong><small>{f.path.split("/").pop()}</small></button>
        {:else}
          <p class="muted pad">{files ? "This mod has no config files yet. Most mods create them the first time the game starts." : "Looking…"}</p>
        {/each}
      </nav>
      <section class="main">
        {#if path}
          <div class="bar">
            {#if formOk && !raw}
              <input class="search" placeholder="Search settings…" bind:value={filter} />
            {:else}<span class="muted grow">{path}</span>{/if}
            {#if formOk}<button class="ghost" onclick={() => (raw = !raw)}>{raw ? "Back to settings" : "Edit as text"}</button>{/if}
          </div>
          {#if raw || !formOk}
            {#if !formOk}<p class="muted small">This file's format can't be shown as settings, so it opens as text.</p>{/if}
            <textarea spellcheck="false" bind:value={text}></textarea>
          {:else}
            <div class="form">
              {#each groups as [folder, list] (folder)}
                {@const expanded = !folder || open.has(folder) || !!filter.trim()}
                {#if folder}
                  <button class="folder" class:open={expanded} onclick={() => toggle(folder)} aria-expanded={expanded}>
                    <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" fill="currentColor" opacity="0.85" /></svg>
                    <strong>{folder.split(" › ").map(label).join(" › ")}</strong>
                    <span class="n">{list.length}</span>
                    <span class="chev">›</span>
                  </button>
                {/if}
                {#if expanded}
                  <div class="group" class:nested={!!folder}>
                    {#each list as s (s.id)}
                      {@const bad = invalid.get(s.id)}
                      {@const changedFromDefault = s.def !== undefined && !same(s.def, s.value)}
                      <div class="field" class:bad={!!bad}>
                        <div class="lbl">
                          <strong>{label(s.key)}</strong>
                          {#if s.help}<small>{s.help}</small>{/if}
                          {#if s.min !== undefined || s.max !== undefined}
                            <small class="rule">{s.min !== undefined && s.max !== undefined ? `${s.min} to ${s.max}` : s.min !== undefined ? `At least ${s.min}` : `At most ${s.max}`}</small>
                          {/if}
                          {#if bad}<small class="err-msg">{bad}</small>{/if}
                        </div>
                        <div class="ctl">
                          {#if s.kind === "bool"}
                            <button class="tf" class:on={s.value === true} onclick={() => setValue(s, !s.value)}>{s.value ? "True" : "False"}</button>
                          {:else if s.kind === "enum"}
                            <select class="val" value={String(s.value)} onchange={(e) => setValue(s, e.currentTarget.value)}>
                              {#each s.options ?? [] as o (o)}<option value={o}>{label(o.toLowerCase())}</option>{/each}
                            </select>
                          {:else if s.kind === "int" || s.kind === "float"}
                            {#if s.min !== undefined && s.max !== undefined && s.max - s.min <= 10000}
                              <input class="slider" type="range" min={s.min} max={s.max} step={s.kind === "int" ? 1 : (s.max - s.min) / 100}
                                value={Number(s.value)} oninput={(e) => setValue(s, s.kind === "int" ? Math.round(Number(e.currentTarget.value)) : Math.round(Number(e.currentTarget.value) * 1000) / 1000)} />
                            {/if}
                            <input class="val num" inputmode="decimal" value={String(s.value)} onchange={(e) => numberInput(s, e.currentTarget.value)} />
                          {:else if s.kind === "string"}
                            <input class="val" value={String(s.value)} onchange={(e) => setValue(s, e.currentTarget.value)} />
                          {:else if s.kind === "list"}
                            <div class="list">
                              {#each (s.value as Value[]) as item, i (i)}
                                <span class="chip">{show(item)}<button aria-label="Remove {show(item)}" onclick={() => setValue(s, (s.value as Value[]).filter((_, j) => j !== i))}>✕</button></span>
                              {/each}
                              <input class="add" placeholder="Add…" bind:value={adding[s.id]} onkeydown={(e) => e.key === "Enter" && addItem(s)} onblur={() => addItem(s)} />
                            </div>
                          {:else}
                            <input class="val mono" value={show(s.value)} onchange={(e) => rawInput(s, e.currentTarget.value)} />
                          {/if}
                          <button class="reset" title={s.def !== undefined ? `Reset to default: ${show(s.def)}` : "No default known"} disabled={!changedFromDefault}
                            onclick={() => setValue(s, s.def!)} aria-label="Reset {s.key}">
                            <svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true"><path d="M4 12a8 8 0 1 0 2.3-5.7M4 4v5h5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>
                          </button>
                        </div>
                      </div>
                    {/each}
                  </div>
                {/if}
              {:else}
                <p class="muted">{filter ? "No settings match your search." : "No settings in this file."}</p>
              {/each}
            </div>
          {/if}
        {/if}
        {#if error}<p class="err">{error}</p>{/if}
      </section>
    </div>
    <footer>
      <button class="ghost" disabled={!path || defaults === null || defaults === text} onclick={restoreDefault}
        title="Back to how this file was before Obelisk first opened it">Restore default</button>
      <span class="muted small grow">{invalid.size ? "" : dirty ? "Unsaved changes" : "Changes apply the next time the game starts."}</span>
      {#if invalid.size}<span class="err small grow">{invalid.size} invalid value{invalid.size === 1 ? "" : "s"}</span>{/if}
      <button class="ghost" disabled={!dirty} onclick={() => { text = saved; invalid = new Map(); }}>Revert</button>
      <button class="primary" disabled={!dirty || busy || invalid.size > 0} onclick={save}>{busy ? "Saving…" : "Save"}</button>
    </footer>
  </div>
</div>

<style>
  .back { position: fixed; inset: 0; background: rgb(0 0 0 / 0.6); display: grid; place-items: center; z-index: 60; padding: 16px; }
  .dialog { width: min(1100px, 100%); height: min(820px, 100%); background: var(--panel-2); border: 1px solid var(--line); border-radius: 10px; display: flex; flex-direction: column; overflow: hidden; }
  header { display: flex; align-items: center; padding: 14px 18px; border-bottom: 1px solid var(--line); }
  h3 { margin: 0; flex: 1; font-size: 17px; }
  .x { background: none; border: 0; color: var(--muted); font-size: 15px; }
  .body { flex: 1; display: grid; grid-template-columns: 220px 1fr; min-height: 0; }
  .files { overflow-y: auto; border-right: 1px solid var(--line); padding: 8px; display: flex; flex-direction: column; gap: 2px; }
  .files button { text-align: left; background: none; border: 0; padding: 8px 10px; border-radius: 6px; display: flex; flex-direction: column; min-width: 0; }
  .files button small { color: var(--muted); font-size: 11.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .files button strong { font-weight: 500; color: var(--text-2); }
  .files button:hover { background: var(--panel); }
  .files button.on { background: var(--panel); box-shadow: inset 3px 0 0 var(--accent); }
  .files button.on strong { color: var(--text); font-weight: 600; }
  .pad { padding: 8px; }
  .main { display: flex; flex-direction: column; min-height: 0; padding: 12px 16px; gap: 10px; }
  .bar { display: flex; gap: 8px; align-items: center; }
  .grow { flex: 1; }
  .search { flex: 1; background: var(--bg); border: 1px solid var(--line); color: var(--text); border-radius: 6px; padding: 9px 12px; font-size: 14px; }
  .ghost { background: none; border: 1px solid var(--line); border-radius: 5px; padding: 7px 12px; white-space: nowrap; }
  .ghost:disabled { opacity: 0.45; }
  textarea { flex: 1; resize: none; background: var(--bg); color: var(--text); border: 1px solid var(--line); border-radius: 6px; padding: 10px; font: 12.5px/1.5 ui-monospace, Consolas, monospace; }
  .form { flex: 1; overflow-y: auto; padding-right: 6px; display: flex; flex-direction: column; gap: 6px; }
  .folder { width: 100%; display: flex; align-items: center; gap: 10px; background: var(--panel); border: 1px solid var(--line); border-radius: 6px; padding: 10px 14px; text-align: left; color: var(--text); }
  .folder:hover { border-color: var(--accent); }
  .folder svg { color: #e5b55a; flex: none; }
  .folder .n { margin-left: auto; color: var(--muted); font-size: 12px; }
  .chev { color: var(--muted); transition: transform 0.15s; font-size: 18px; line-height: 1; }
  .folder.open .chev { transform: rotate(90deg); }
  .group.nested { padding-left: 12px; border-left: 2px solid var(--line); margin: 0 0 6px 14px; }
  .field { display: grid; grid-template-columns: minmax(0, 1fr) minmax(200px, 360px); gap: 16px; align-items: center; padding: 9px 4px; border-bottom: 1px solid var(--row-line); }
  .field.bad .val { border-color: var(--danger); }
  .lbl { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .lbl small { color: var(--muted); white-space: pre-line; font-size: 12px; }
  .lbl .rule { color: var(--text-2); }
  .err-msg { color: var(--danger) !important; }
  .ctl { display: flex; gap: 8px; align-items: center; justify-content: flex-end; }
  .val { background: var(--bg); border: 1px solid var(--line); color: var(--text); border-radius: 5px; padding: 6px 9px; width: 100%; min-width: 0; }
  .num { width: 100px; flex: none; }
  .slider { flex: 1; min-width: 0; accent-color: var(--accent); }
  .mono { font-family: ui-monospace, Consolas, monospace; font-size: 12px; }
  .tf { width: 84px; padding: 6px 0; border-radius: 5px; border: 1px solid rgb(240 80 80 / 0.5); background: rgb(240 80 80 / 0.14); color: #f26b6b; font-weight: 700; }
  .tf.on { border-color: rgb(60 200 120 / 0.55); background: rgb(60 200 120 / 0.15); color: #3ccf7f; }
  .list { flex: 1; display: flex; flex-wrap: wrap; gap: 4px; background: var(--bg); border: 1px solid var(--line); border-radius: 5px; padding: 4px; min-width: 0; max-height: 120px; overflow-y: auto; }
  .chip { display: inline-flex; align-items: center; gap: 4px; background: var(--panel); border-radius: 4px; padding: 2px 4px 2px 8px; font-size: 12px; max-width: 100%; overflow: hidden; }
  .chip button { background: none; border: 0; color: var(--muted); padding: 0 3px; font-size: 11px; }
  .chip button:hover { color: var(--danger); }
  .add { flex: 1; min-width: 70px; background: none; border: 0; color: var(--text); padding: 3px 4px; font-size: 12px; outline: none; }
  .reset { background: none; border: 1px solid var(--line); border-radius: 5px; width: 30px; height: 30px; flex: none; color: var(--text-2); display: grid; place-items: center; }
  .reset:not(:disabled):hover { color: var(--accent); border-color: var(--accent); }
  .reset:disabled { opacity: 0.25; }
  footer { display: flex; gap: 10px; align-items: center; padding: 12px 18px; border-top: 1px solid var(--line); }
  .small { font-size: 12.5px; }
  .primary { background: var(--accent); color: #fff; border: 0; border-radius: 5px; padding: 8px 22px; font-weight: 600; }
  .primary:disabled { opacity: 0.5; }
  .muted { color: var(--muted); }
  .err { color: var(--err-text); margin: 0; }
</style>
