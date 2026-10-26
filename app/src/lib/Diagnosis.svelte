<script lang="ts">
  import { crisp } from "./crisp";
  import { invoke } from "@tauri-apps/api/core";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { ago, initials, type FileInfo, type Item } from "./api";
  import { app, toast } from "./state.svelte";
  import { settings } from "./settings.svelte";
  import Icon from "./Icon.svelte";

  type Info = { file: string; kind: "crash" | "jvm" | "log"; timeMs: number };
  type Finding = { severity: "critical" | "warning" | "info"; title: string; detail: string; fix: string; action: string; target: string; lines?: string[] };
  type Suspect = { modId: string; fileName: string; reason: string; score: number };
  type Step = { seconds: number; phase: string; who: string; file: string; note: string; before: string; after: string };
  type Timeline = { startupS: number; worldS: number; steps: Step[]; byMod: { who: string; file: string; seconds: number }[] };
  type Group = { who: string; file: string; count: number; sample: string };
  type Noise = { label: string; count: number; from: string[] };
  type Diag = { file: string; kind: string; timeMs: number; headline: string; error: string; findings: Finding[]; suspects: Suspect[]; excerpt: string[]; maxMemoryMb: number;
    timeline: Timeline | null; errorGroups: Group[]; noise: Noise[] };
  type Phase = { key: string; label: string; ms: number; idle: boolean };
  type Times = { clickMs: number; source: string; label: string; speedloader: boolean; phases: Phase[] };
  type Run = { id: string; label: string; startedMs: number; whole: any; findings: any[]; contenders: any[]; phases: string[]; html: string };

  let { instanceId, mods, icons, infoOf, onToggle, onOpen, onShaders, file = $bindable("") }: {
    instanceId: string; mods: Item[]; icons: Record<string, string>; infoOf: (i: Item) => FileInfo | undefined;
    onToggle: (i: Item, want: boolean) => Promise<void>; onOpen: (i: Item) => void; onShaders: () => void;
    /** The crash report or log being shown (for the header's Copy button). */
    file?: string;
  } = $props();

  let reports = $state<Info[]>([]);
  let runs = $state<Run[]>([]);
  let lightningInstalled = $state(false);
  let pick = $state<{ type: "report"; file: string } | { type: "run"; id: string } | { type: "times"; at: number } | null>(null);
  let times = $state<Times[]>([]);
  let diag = $state<Diag | null>(null);
  let loading = $state(false);

  const KIND_LABEL = { crash: "Crash report", jvm: "Java crash", log: "Last session" };
  let useLightning = $derived(settings.plugins?.lightning === true);

  /** Read the list of crashes and logs. `force` re-reads the open one even if nothing changed;
   *  otherwise only a new or changed file reloads the page. */
  async function load(id: string, force: boolean) {
    const r = await invoke<Info[]>("crash_reports", { instanceId: id }).catch(() => [] as Info[]);
    if (id !== instanceId) return;
    const changed = JSON.stringify(r) !== JSON.stringify(reports);
    if (!changed && !force) return;
    const had = pick?.type === "report" ? pick.file : null;
    const wasNewest = had && reports.find((x) => x.kind !== "log")?.file === had;
    reports = r;
    // Stay on the open file; jump to the newest crash only at first or when you were on the newest one.
    const first = r.find((x) => x.kind !== "log") ?? r[0];
    const keep = had && !wasNewest && r.some((x) => x.file === had) ? had : first?.file;
    if (keep && pick?.type !== "run") select({ type: "report", file: keep });
    try { if (first) localStorage.setItem(`obelisk.seenCrash.${id}`, String(Math.max(...r.filter((x) => x.kind !== "log").map((x) => x.timeMs), 0))); } catch { /* ignore */ }
  }
  $effect(() => {
    const id = instanceId;
    reports = [];
    runs = [];
    diag = null;
    pick = null;
    load(id, true);
    // Coming back to the window only reloads when a crash report or log was written meanwhile.
    const onFocus = () => load(id, false);
    const onRefresh = () => load(id, true);
    window.addEventListener("focus", onFocus);
    window.addEventListener("obelisk-refresh", onRefresh);
    return () => { window.removeEventListener("focus", onFocus); window.removeEventListener("obelisk-refresh", onRefresh); };
  });
  $effect(() => {
    const id = instanceId;
    invoke<Times[]>("load_times", { instanceId: id }).then((t) => { if (id === instanceId) times = t; }).catch(() => (times = []));
  });
  $effect(() => {
    if (!useLightning) return void (runs = []);
    invoke<{ installed: boolean; runs: Run[] }>("lightning_runs", { instanceId }).then((r) => {
      lightningInstalled = r.installed;
      runs = r.runs;
    });
  });

  async function select(p: NonNullable<typeof pick>) {
    pick = p;
    if (p.type !== "report") return;
    file = p.file;
    loading = true;
    try {
      diag = await invoke<Diag>("diagnose_crash", { instanceId, file: p.file });
    } catch (e) {
      toast(String(e));
    }
    loading = false;
  }

  let sess = $derived(pick?.type === "times" ? times.find((t) => t.clickMs === (pick as any).at) ?? null : null);
  /** Loading time up to a phase key (not counting time spent in the menu or playing). */
  const until = (t: Times, key: string) => {
    let sum = 0;
    for (const p of t.phases) { if (!p.idle) sum += p.ms; if (p.key === key) return sum; }
    return null;
  };
  const msecs = (ms: number | null | undefined) => (ms == null ? "–" : `${(ms / 1000).toFixed(1)} s`);
  /** Same phase in the other sessions, averaged, for "faster or slower than usual". */
  const usual = (label: string, not: Times) => {
    const v = times.filter((t) => t !== not).map((t) => t.phases.find((p) => p.label === label)?.ms).filter((x): x is number => x != null);
    return v.length ? v.reduce((a, b) => a + b, 0) / v.length : null;
  };
  const PHASE_COLOR: Record<string, string> = { launcher: "#8a7cf0", jvm: "#5aa2f2", mods: "#e5c84a", resources: "#f0a35e", join: "#4cc38a", leave: "#c47be0", quit: "#f2555a" };
  let run = $derived(pick?.type === "run" ? runs.find((r) => r.id === (pick as any).id) ?? null : null);
  const itemFor = (file: string) => mods.find((m) => m.fileName === file);
  const DRIVERS: Record<string, string> = {
    NVIDIA: "https://www.nvidia.com/en-us/drivers/",
    AMD: "https://www.amd.com/en/support/download/drivers.html",
    Intel: "https://www.intel.com/content/www/us/en/support/detect.html",
  };

  async function act(f: Finding) {
    const inst = app.instances.find((i) => i.id === instanceId);
    if (f.action === "memory" && inst) app.profileDialog = { kind: "options", inst };
    else if (f.action === "driver") openUrl(DRIVERS[f.target] ?? "https://www.google.com/search?q=graphics+driver+download");
    else if (f.action === "java") invoke("open_prism");
    else if (f.action === "shaders") onShaders();
    else if ((f.action === "disable" || f.action === "page") && f.target) {
      const it = itemFor(f.target);
      if (!it) return toast("That mod isn't in this profile any more.");
      if (f.action === "page") onOpen(it);
      else { await onToggle(it, false); toast(`Disabled ${it.name}. Launch again to check.`); }
    }
  }
  const RANK = { critical: 0, warning: 1, info: 2 };
  const secs = (s: number) => (s >= 60 ? `${Math.floor(s / 60)} min ${Math.round(s % 60)} s` : `${Math.round(s)} s`);
  const nameOf = (s: Suspect) => itemFor(s.fileName)?.name ?? s.modId;

  /** The diagnosis in plain text, with the log lines behind it, ready to paste into a bug report or chat. */
  function synopsis(d: Diag): string {
    const inst = app.instances.find((i) => i.id === instanceId);
    const out: string[] = [];
    out.push(`Obelisk diagnosis: ${inst?.name ?? instanceId}`);
    out.push(`${KIND_LABEL[d.kind as keyof typeof KIND_LABEL]} · ${when(d.timeMs)} · ${d.file}`, "", d.headline);
    if (d.error) out.push(`Error: ${d.error}`);
    const fs = [...d.findings].sort((a, b) => RANK[a.severity] - RANK[b.severity]);
    if (fs.length) {
      out.push("", "What Obelisk found:");
      for (const f of fs) {
        out.push(`- [${f.severity}] ${f.title}`);
        if (f.detail) out.push(...f.detail.split("\n").map((l) => `    ${l}`));
        if (f.fix) out.push(`    What to do: ${f.fix}`);
      }
    }
    if (d.suspects.length) {
      out.push("", "Mods involved (most likely first):");
      for (const s of d.suspects) out.push(`- ${nameOf(s)}${s.fileName ? ` [${s.fileName}]` : ""}: ${s.reason}`);
    }
    const t = d.timeline;
    if (t && (t.startupS || t.worldS || t.byMod.length)) {
      out.push("", `Where the time went: starting the game ${t.startupS ? secs(t.startupS) : "?"}, opening the world ${t.worldS ? secs(t.worldS) : "?"}`);
      for (const m of t.byMod.slice(0, 6)) out.push(`- ${m.who}: ${secs(m.seconds)}`);
      for (const m of t.byMod) {
        const s = t.steps.find((x) => x.who === m.who && x.note);
        if (s) out.push(`    Why ${m.who}: ${s.note}`);
      }
    }
    if (d.errorGroups?.length) {
      out.push("", "Errors by mod:");
      for (const g of d.errorGroups.slice(0, 8)) out.push(`- ${g.who} x${g.count}: ${g.sample}`);
    }
    if (d.noise?.length) out.push("", `Known harmless: ${d.noise.map((n) => `${n.label} (${n.count})`).join("; ")}`);
    if (d.excerpt.length) out.push("", "Relevant lines from the file:", ...d.excerpt.slice(0, 50));
    return out.join("\n");
  }
  /** One issue on its own: what it is, what to do, and the lines behind it. */
  async function copyOne(parts: { title: string; detail?: string; fix?: string; lines?: string[] }) {
    const inst = app.instances.find((i) => i.id === instanceId);
    const out = [`${parts.title}  (${inst?.name ?? instanceId}, ${diag?.file ?? ""})`];
    if (parts.detail) out.push(parts.detail);
    if (parts.fix) out.push(`What to do: ${parts.fix}`);
    if (parts.lines?.length) out.push("", "From the log:", ...parts.lines);
    await navigator.clipboard.writeText(out.join("\n"));
    toast("Copied this issue with its log lines.");
  }
  async function copySynopsis() {
    if (!diag) return;
    await navigator.clipboard.writeText(synopsis(diag));
    toast("Diagnosis copied, with the log lines behind it. Paste it anywhere.");
  }
  const ACTION_LABEL: Record<string, string> = { memory: "Change memory", driver: "Get the driver", java: "Open Prism", shaders: "Shaders", page: "Open mod page" };

  /** Lightning's per-mod CPU shares, worst first, with a plain-word verdict. */
  const tier = (pct: number) => (pct >= 30 ? "Very heavy" : pct >= 15 ? "Heavy" : pct >= 5 ? "Noticeable" : "Light");
  const fmt = (n: any, d = 0) => (typeof n === "number" ? n.toFixed(d) : "–");
  const when = (ms: number) => (ms ? `${new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })}` : "");
</script>

<div class="diag">
  <nav class="list">
    <div class="lhead">
      <h4>Crashes and logs</h4>
      <span class="spacer"></span>
      <button class="copy" title="Re-read crashes and logs (Ctrl+R)" aria-label="Refresh" onclick={() => load(instanceId, true)}>
        <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path d="M20 12a8 8 0 1 1-2.34-5.66M20 4v5h-5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" /></svg>
      </button>
      <button class="copy" title="Copy this diagnosis (summary, findings and the log lines behind them)" aria-label="Copy diagnosis" disabled={!diag || pick?.type !== "report"} onclick={copySynopsis}>
        <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><rect x="8" y="8" width="12" height="13" rx="2" fill="none" stroke="currentColor" stroke-width="2" /><path d="M16 8V5a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h2" fill="none" stroke="currentColor" stroke-width="2" /></svg>
      </button>
    </div>
    {#each reports as r (r.file)}
      <button class:on={pick?.type === "report" && pick.file === r.file} onclick={() => select({ type: "report", file: r.file })}>
        <span class="dot {r.kind}"></span>
        <span class="txt"><strong>{KIND_LABEL[r.kind]}</strong><small>{ago(r.timeMs)} · {r.file.split("/").pop()}</small></span>
      </button>
    {:else}
      <p class="muted small pad">No crashes yet. If the game crashes, the report shows up here, explained.</p>
    {/each}
    <h4 class="bolt">Load times</h4>
    {#each times as t (t.clickMs)}
      <button class:on={pick?.type === "times" && pick.at === t.clickMs} onclick={() => (pick = { type: "times", at: t.clickMs })}>
        <span class="dot times"></span>
        <span class="txt"><strong>Menu {msecs(until(t, "resources"))}{#if until(t, "join") != null} · world {msecs(t.phases.find((p) => p.key === "join")?.ms)}{/if}</strong>
          <small>{ago(t.clickMs)}{t.source === "test" ? " · test run" : ""}</small></span>
      </button>
    {:else}
      <p class="muted small pad">Play once from Obelisk and how long each loading step took shows up here.</p>
    {/each}
    {#if useLightning}
      <h4 class="bolt">Lightning runs</h4>
      {#if !lightningInstalled}
        <p class="muted small pad">Lightning isn't installed in this profile.</p>
      {/if}
      {#each runs as r (r.id)}
        <button class:on={pick?.type === "run" && pick.id === r.id} onclick={() => select({ type: "run", id: r.id })}>
          <span class="dot run"></span>
          <span class="txt"><strong>{r.label || "Capture"} · {fmt(r.whole.fps)} FPS</strong><small>{ago(r.startedMs)}</small></span>
        </button>
      {:else}
        {#if lightningInstalled}<p class="muted small pad">No captures yet. Record one in game with Lightning.</p>{/if}
      {/each}
    {/if}
  </nav>

  <section class="detail">
    {#if pick?.type === "report"}
      {#if loading && !diag}<p class="muted">Reading…</p>{/if}
      {#if diag}
        <header class="hero" class:ok={!diag.findings.some((f) => f.severity === "critical")}>
          <div>
            <small>{KIND_LABEL[diag.kind as keyof typeof KIND_LABEL]} · {when(diag.timeMs)}</small>
            <h2>{diag.headline}</h2>
            {#if diag.error}<code class="err-line">{diag.error}</code>{/if}
          </div>
          <button class="ghost" onclick={() => invoke("open_game_file", { instanceId, rel: diag!.file }).catch((e) => toast(String(e)))}>Open file</button>
        </header>

        {#each [...diag.findings].sort((a, b) => RANK[a.severity] - RANK[b.severity]) as f, i (i)}
          <article class="finding {f.severity}">
            <div class="ftext">
              <strong>{f.title}</strong>
              {#if f.detail}<p>{f.detail}</p>{/if}
              {#if f.fix}<p class="fix"><b>What to do:</b> {f.fix}</p>{/if}
            </div>
            {#if f.action && (f.action !== "disable" || itemFor(f.target)?.enabled)}
              <button class="primary" onclick={() => act(f)}>{f.action === "disable" ? `Disable ${itemFor(f.target)?.name ?? "mod"}` : ACTION_LABEL[f.action]}</button>
            {/if}
            <button class="copy one" title="Copy just this issue, with its log lines" aria-label="Copy this issue" onclick={() => copyOne(f)}><svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><rect x="8" y="8" width="12" height="13" rx="2" fill="none" stroke="currentColor" stroke-width="2" /><path d="M16 8V5a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h2" fill="none" stroke="currentColor" stroke-width="2" /></svg></button>
          </article>
        {/each}

        {#if diag.suspects.length}
          <h3>Mods involved</h3>
          <div class="suspects">
            {#each diag.suspects as s, i (s.modId)}
              {@const it = itemFor(s.fileName)}
              {@const info = it ? infoOf(it) : undefined}
              <div class="suspect">
                {#if it && (icons[it.fileName] || info?.iconUrl)}<img use:crisp src={icons[it.fileName] || info?.iconUrl} alt="" />{:else}<div class="ph">{initials(it?.name ?? s.modId)}</div>{/if}
                <div class="stext">
                  <strong>{it?.name ?? s.modId}{#if i === 0}<span class="tag">Most likely</span>{/if}{#if it && !it.enabled}<span class="tag off">Disabled</span>{/if}</strong>
                  <small>{s.reason}{#if !it} Not installed now.{/if}</small>
                  <div class="bar"><i style:width="{Math.min(100, (s.score / diag.suspects[0].score) * 100)}%"></i></div>
                </div>
                {#if it}
                  {#if info?.update}<span class="tag upd">Update available</span>{/if}
                  <button class="ghost" onclick={() => onOpen(it)}>Page</button>
                  <button class="ghost" onclick={() => onToggle(it, !it.enabled)}>{it.enabled ? "Disable" : "Enable"}</button>
                {/if}
              </div>
            {/each}
          </div>
        {/if}

        {#if diag.timeline && (diag.timeline.startupS || diag.timeline.worldS || diag.timeline.byMod.length)}
          {@const t = diag.timeline}
          <h3>Where the time went</h3>
          <div class="stats">
            {#if t.startupS}<div><span class="sicon"><Icon name="timer" size={22} /></span><span class="sv"><b>{secs(t.startupS)}</b><small>Starting the game</small></span></div>{/if}
            {#if t.worldS}<div><span class="sicon"><Icon name="cube" size={22} /></span><span class="sv"><b>{secs(t.worldS)}</b><small>Opening the world</small></span></div>{/if}
          </div>
          {#if t.byMod.length}
            <p class="muted small">Time the game spent waiting on each mod while starting up, opening the world and closing (time in menus and in game isn't counted).</p>
            <div class="mods-cost">
              {#each t.byMod as m (m.who)}
                {@const it = m.file ? itemFor(m.file) : undefined}
                <div class="cost {m.seconds >= 20 ? 'very-heavy' : m.seconds >= 8 ? 'heavy' : m.seconds >= 4 ? 'noticeable' : 'light'}">
                  <span class="mname">{it?.name ?? m.who}</span>
                  <div class="bar"><i style:width="{Math.min(100, (m.seconds / t.byMod[0].seconds) * 100)}%"></i></div>
                  <span class="pct">{secs(m.seconds)}</span>
                  {#if it}<button class="ghost tiny" onclick={() => onOpen(it)}>Page</button>{:else}<span></span>{/if}
                </div>
              {/each}
            </div>
            {#each t.byMod as m (m.who)}
              {@const s = t.steps.find((x) => x.who === m.who && x.note)}
              {#if s}
                <article class="finding info"><div class="ftext"><strong>Why {m.who} took {secs(m.seconds)}</strong><p>{s.note}</p></div><button class="copy one" title="Copy just this issue, with its log lines" aria-label="Copy this issue" onclick={() => copyOne({ title: `Why ${m.who} took ${secs(m.seconds)}`, detail: s.note, lines: t.steps.filter((x) => x.who === m.who).flatMap((x) => [`${secs(x.seconds)} wait:`, x.before, x.after]) })}><svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><rect x="8" y="8" width="12" height="13" rx="2" fill="none" stroke="currentColor" stroke-width="2" /><path d="M16 8V5a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h2" fill="none" stroke="currentColor" stroke-width="2" /></svg></button></article>
              {/if}
            {/each}
            <details class="raw">
              <summary>Longest waits, with the log lines around them</summary>
              <pre>{t.steps.map((s) => `${secs(s.seconds)} · ${s.who} (${s.phase})\n  ${s.before}\n  ${s.after}`).join("\n\n")}</pre>
            </details>
          {/if}
        {/if}

        {#if diag.errorGroups?.length}
          <h3>Errors by mod</h3>
          <div class="groups">
            {#each diag.errorGroups.slice(0, 12) as g (g.who + g.file)}
              {@const it = g.file ? itemFor(g.file) : undefined}
              <div class="group">
                <div class="stext"><strong>{it?.name ?? g.who} <span class="muted">×{g.count}</span></strong><small class="sample">{g.sample}</small></div>
                {#if it}<button class="ghost" onclick={() => onOpen(it)}>Page</button>{/if}
                <button class="copy one" title="Copy just this issue, with its log lines" aria-label="Copy this issue" onclick={() => copyOne({ title: `${it?.name ?? g.who}: ${g.count} error line${g.count === 1 ? "" : "s"}`, lines: [g.sample, ...diag!.excerpt.filter((l) => l.includes(g.who)).slice(0, 10)] })}><svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><rect x="8" y="8" width="12" height="13" rx="2" fill="none" stroke="currentColor" stroke-width="2" /><path d="M16 8V5a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h2" fill="none" stroke="currentColor" stroke-width="2" /></svg></button>
              </div>
            {/each}
          </div>
        {/if}

        {#if diag.noise?.length}
          <details class="raw">
            <summary>{diag.noise.reduce((n, x) => n + x.count, 0)} harmless messages Obelisk recognised</summary>
            <ul class="noise">
              {#each diag.noise as n (n.label)}<li><b>{n.count}×</b> {n.label}{#if n.from.length}<span class="muted"> · from {n.from.join(", ")}</span>{/if}</li>{/each}
            </ul>
          </details>
        {/if}

        {#if diag.excerpt.length}
          <details class="raw">
            <summary>{diag.kind === "log" ? "Error lines" : "Technical details"} ({diag.excerpt.length} lines)</summary>
            <pre>{diag.excerpt.join("\n")}</pre>
          </details>
        {/if}
      {/if}
    {:else if run}
      {@const w = run.whole}
      <header class="hero ok">
        <div>
          <small>Lightning capture · {when(run.startedMs)} · {fmt(w.seconds)} s</small>
          <h2>{run.label || "Performance capture"}</h2>
        </div>
        {#if run.html}<button class="ghost" onclick={() => invoke("open_game_file", { instanceId, rel: run!.html })}>Full report</button>{/if}
      </header>
      <div class="stats">
        <div><span class="sicon"><Icon name="cube" size={22} /></span><span class="sv"><b>{fmt(w.fps)}</b><small>Average FPS</small></span></div>
        <div><span class="sicon"><Icon name="bars" size={22} /></span><span class="sv"><b>{fmt(w.low1PctFps)}</b><small>1% low FPS</small></span></div>
        <div><span class="sicon"><Icon name="timer" size={22} /></span><span class="sv"><b>{fmt(w.frameP99, 1)} ms</b><small>Slow frames (P99)</small></span></div>
        <div><span class="sicon"><Icon name="tick" size={22} /></span><span class="sv"><b>{fmt(w.tickP50, 1)} ms</b><small>Server tick (50 ms budget)</small></span></div>
        <div><span class="sicon"><Icon name="memory" size={22} /></span><span class="sv"><b>{w.liveSetMBmax ? `${(w.liveSetMBmax / 1024).toFixed(1)} / ${(w.heapMaxMB / 1024).toFixed(1)} GB` : "–"}</b><small>Memory used / allowed</small></span></div>
      </div>
      {#if w.liveSetMBmax && w.heapMaxMB && w.liveSetMBmax / w.heapMaxMB > 0.85}
        <article class="finding critical"><div class="ftext"><strong>Memory is nearly full</strong><p>The game peaked at {(w.liveSetMBmax / 1024).toFixed(1)} GB of {(w.heapMaxMB / 1024).toFixed(1)} GB. Expect stutters and possibly an out-of-memory crash.</p><p class="fix"><b>What to do:</b> give it about 2 GB more in Profile options.</p></div>
          <button class="primary" onclick={() => act({ severity: "critical", title: "", detail: "", fix: "", action: "memory", target: "" })}>Change memory</button></article>
      {/if}
      {#if w.topModsCpu?.length}
        <h3>Which mods cost the most</h3>
        <p class="muted small">Share of the game thread's CPU samples each mod showed up in (a mod calling another counts for both).</p>
        <div class="mods-cost">
          {#each w.topModsCpu.slice(0, 10) as m (m.mod)}
            <div class="cost {tier(m.sharePct).toLowerCase().replace(' ', '-')}">
              <span class="mname">{m.mod}</span>
              <div class="bar"><i style:width="{Math.min(100, m.sharePct)}%"></i></div>
              <span class="pct">{fmt(m.sharePct)}%</span>
              <span class="tier">{tier(m.sharePct)}</span>
            </div>
          {/each}
        </div>
        {@const worst = w.topModsCpu[0]}
        {#if worst && worst.sharePct >= 30}
          <article class="finding warning"><div class="ftext"><strong>{worst.mod} is the heaviest mod here</strong><p>It showed up in {fmt(worst.sharePct)}% of the game thread's work during this capture. If the game feels slow, try it without {worst.mod}, or check its settings.</p></div></article>
        {/if}
      {/if}
      {#if run.contenders?.length}
        <h3>What Lightning noticed</h3>
        {#each run.contenders as c, i (i)}
          <article class="finding info"><div class="ftext"><strong>{c.title}</strong>{#if c.size}<p>{c.size}</p>{/if}</div></article>
        {/each}
      {/if}
      {#if runs.length > 1}
        <h3>History</h3>
        <table class="hist">
          <thead><tr><th>When</th><th>Label</th><th>FPS</th><th>1% low</th><th>Heaviest mod</th></tr></thead>
          <tbody>
            {#each runs as r (r.id)}
              <tr class:on={r.id === run.id} onclick={() => select({ type: "run", id: r.id })}>
                <td>{ago(r.startedMs)}</td><td>{r.label || "–"}</td><td>{fmt(r.whole.fps)}</td><td>{fmt(r.whole.low1PctFps)}</td>
                <td>{r.whole.topModsCpu?.[0] ? `${r.whole.topModsCpu[0].mod} (${fmt(r.whole.topModsCpu[0].sharePct)}%)` : "–"}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    {:else if sess}
      {@const load = sess.phases.filter((p) => !p.idle)}
      {@const total = load.reduce((a, p) => a + p.ms, 0) || 1}
      <header class="hero ok">
        <div>
          <small>Load times · {ago(sess.clickMs)}{sess.label ? ` · ${sess.label}` : ""}</small>
          <h2>Menu in {msecs(until(sess, "resources"))}{#if until(sess, "join") != null}, in the world after {msecs(until(sess, "join"))} of loading{/if}</h2>
          <p class="muted small">Counts only loading, not time you spent in the menu or playing.{sess.speedloader ? " Includes Optimal's exact title-screen and terrain marks." : ""}</p>
        </div>
      </header>
      <div class="tbar">
        {#each load as p, i (i)}<i style:width="{(100 * p.ms) / total}%" style:background={PHASE_COLOR[p.key] ?? "var(--muted)"} title="{p.label}: {msecs(p.ms)}"></i>{/each}
      </div>
      <table class="hist">
        <thead><tr><th>Step</th><th>This time</th><th>Usually</th><th></th></tr></thead>
        <tbody>
          {#each sess.phases as p, i (i)}
            {@const u = usual(p.label, sess)}
            <tr class:idle={p.idle}>
              <td><span class="sw" style:background={p.idle ? "transparent" : PHASE_COLOR[p.key] ?? "var(--muted)"}></span>{p.label}</td>
              <td>{msecs(p.ms)}</td><td>{p.idle ? "" : msecs(u)}</td>
              <td class:slow={!p.idle && u != null && p.ms > u * 1.15 && p.ms - u > 1000} class:fast={!p.idle && u != null && p.ms < u * 0.87 && u - p.ms > 1000}>
                {#if !p.idle && u != null && Math.abs(p.ms - u) > 1000}{p.ms > u ? "+" : "−"}{msecs(Math.abs(p.ms - u))}{/if}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="muted">Pick a crash or log on the left.</p>
    {/if}
  </section>
</div>

<style>
  .diag { flex: 1; display: grid; grid-template-columns: 250px 1fr; gap: 14px; min-height: 0; padding: 14px 0 20px; }
  .list { overflow-y: auto; background: var(--panel); border-radius: 8px; padding: 8px; display: flex; flex-direction: column; gap: 2px; }
  .list h4 { margin: 6px 8px; font-size: 11.5px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
  .list h4.bolt { margin-top: 16px; }
  .lhead { display: flex; align-items: center; gap: 2px; padding-right: 4px; }
  .lhead .spacer { flex: 1; }
  .list .copy { display: grid; place-items: center; width: 30px; height: 30px; padding: 0; border-radius: 6px; color: var(--text-2); }
  .list .copy:hover:not(:disabled) { color: var(--text); background: var(--panel-2); }
  .list .copy:disabled { opacity: 0.35; }
  .copy.one { display: grid; place-items: center; width: 32px; height: 32px; padding: 0; flex: none; border-radius: 6px; background: none; border: 1px solid var(--line); color: var(--text-2); }
  .copy.one:hover { color: var(--text); background: var(--panel-2); }
  .groups { display: flex; flex-direction: column; gap: 6px; }
  .group { display: flex; align-items: center; gap: 12px; background: var(--panel); border-radius: 8px; padding: 10px 14px; }
  .sample { font-family: ui-monospace, Consolas, monospace; font-size: 11.5px; overflow: hidden; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; word-break: break-word; }
  .noise { margin: 4px 0 0; padding-left: 18px; display: flex; flex-direction: column; gap: 4px; font-size: 13px; }
  .ghost.tiny { padding: 3px 8px; font-size: 12px; }
  .list button { display: flex; align-items: center; gap: 10px; text-align: left; background: none; border: 0; padding: 8px; border-radius: 6px; }
  .list button:hover { background: var(--panel-2); }
  .list button.on { background: var(--panel-2); box-shadow: inset 3px 0 0 var(--accent); }
  .txt { display: flex; flex-direction: column; min-width: 0; }
  .txt small { color: var(--muted); font-size: 11.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dot { width: 9px; height: 9px; border-radius: 50%; flex: none; background: var(--muted); }
  .dot.crash { background: #f2555a; }
  .dot.jvm { background: #f0a35e; }
  .dot.run { background: #e5c84a; }
  .dot.times { background: #4cc38a; }
  .tbar { display: flex; height: 14px; border-radius: 7px; overflow: hidden; background: var(--panel-2); }
  .tbar i { display: block; height: 100%; min-width: 2px; }
  .sw { display: inline-block; width: 9px; height: 9px; border-radius: 2px; margin-right: 8px; vertical-align: middle; }
  tr.idle td { color: var(--muted); font-style: italic; }
  td.slow { color: #f26b6b; }
  td.fast { color: #4cc38a; }
  .pad { padding: 4px 8px; margin: 0; }
  .detail { overflow-y: auto; padding-right: 6px; display: flex; flex-direction: column; gap: 10px; }
  .hero { display: flex; justify-content: space-between; gap: 14px; align-items: flex-start; background: var(--panel); border-radius: 8px; padding: 16px 18px; border-left: 4px solid #f2555a; }
  .hero.ok { border-left-color: var(--accent); }
  .hero small { color: var(--muted); }
  .hero h2 { margin: 4px 0 8px; font-size: 20px; }
  .err-line { display: block; font-size: 12px; color: var(--text-2); background: var(--bg); padding: 6px 8px; border-radius: 5px; word-break: break-word; }
  .finding { display: flex; gap: 14px; align-items: center; background: var(--panel); border-radius: 8px; padding: 12px 16px; border-left: 4px solid var(--muted); }
  .finding.critical { border-left-color: #f2555a; }
  .finding.warning { border-left-color: #f0a35e; }
  .finding.info { border-left-color: #5aa2f2; }
  .ftext { flex: 1; min-width: 0; }
  .ftext p { margin: 4px 0 0; color: var(--text-2); white-space: pre-line; line-height: 1.45; }
  .ftext .fix { color: var(--text); }
  h3 { margin: 10px 0 0; font-size: 15px; }
  .suspects { display: flex; flex-direction: column; gap: 6px; }
  .suspect { display: flex; align-items: center; gap: 12px; background: var(--panel); border-radius: 8px; padding: 10px 14px; }
  .suspect img, .ph { width: 36px; height: 36px; border-radius: 6px; object-fit: cover; flex: none; }
  .suspect img:global(.px) { image-rendering: pixelated; }
  .ph { background: var(--panel-2); display: grid; place-items: center; font-size: 11px; font-weight: 700; color: var(--muted); }
  .stext { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .stext small { color: var(--muted); }
  .tag { font-size: 11px; font-weight: 600; margin-left: 8px; padding: 1px 7px; border-radius: 999px; background: rgb(242 85 90 / 0.15); color: #f26b6b; }
  .tag.off { background: var(--panel-2); color: var(--muted); }
  .tag.upd { margin: 0; background: color-mix(in srgb, var(--accent) 18%, transparent); color: var(--accent); }
  .bar { height: 5px; background: var(--panel-2); border-radius: 3px; overflow: hidden; }
  .bar i { display: block; height: 100%; background: #f2555a; border-radius: 3px; }
  .raw summary { cursor: pointer; color: var(--muted); padding: 6px 0; }
  .raw pre { background: var(--bg); border: 1px solid var(--line); border-radius: 6px; padding: 10px; font-size: 11.5px; overflow-x: auto; max-height: 360px; }
  .stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(155px, 1fr)); gap: 8px; }
  .stats > div { background: var(--panel); border-radius: 8px; padding: 12px 14px; display: flex; align-items: center; gap: 12px; }
  .sicon { width: 38px; height: 38px; border-radius: 6px; display: grid; place-items: center; color: var(--accent); background: color-mix(in srgb, var(--accent) 14%, transparent); flex: none; }
  .sv { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .stats b { font-size: 20px; }
  .stats small { color: var(--muted); }
  .mods-cost { display: flex; flex-direction: column; gap: 6px; background: var(--panel); border-radius: 8px; padding: 12px 14px; }
  .cost { display: grid; grid-template-columns: 200px 1fr 70px 90px; gap: 10px; align-items: center; font-size: 13px; }
  .mname { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .pct { text-align: right; color: var(--text-2); }
  .tier { font-size: 11.5px; font-weight: 700; }
  .cost.very-heavy .bar i { background: #f2555a; } .cost.very-heavy .tier { color: #f26b6b; }
  .cost.heavy .bar i { background: #f0a35e; } .cost.heavy .tier { color: #f0a35e; }
  .cost.noticeable .bar i { background: #e5c84a; } .cost.noticeable .tier { color: #e5c84a; }
  .cost.light .bar i { background: #3ccf7f; } .cost.light .tier { color: #3ccf7f; }
  .hist { width: 100%; border-collapse: collapse; background: var(--panel); border-radius: 8px; overflow: hidden; font-size: 13px; }
  .hist th, .hist td { text-align: left; padding: 8px 12px; border-bottom: 1px solid var(--row-line); }
  .hist th { color: var(--muted); font-weight: 600; font-size: 12px; }
  .hist tr { cursor: pointer; }
  .hist tbody tr:hover, .hist tr.on { background: var(--panel-2); }
  .ghost { background: none; border: 1px solid var(--line); border-radius: 5px; padding: 7px 12px; white-space: nowrap; }
  .primary { background: var(--accent); color: #fff; border: 0; border-radius: 5px; padding: 8px 16px; font-weight: 600; white-space: nowrap; }
  .muted { color: var(--muted); }
  .small { font-size: 12.5px; }
</style>
