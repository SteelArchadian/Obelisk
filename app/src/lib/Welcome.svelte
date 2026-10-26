<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { settings, saveSettings } from "./settings.svelte";
  import { resetCfSearch } from "./cf";
  import { toast } from "./state.svelte";

  let key = $state("");
  let saving = $state(false);
  let error = $state("");

  async function finish() {
    settings.welcomed = true;
    await saveSettings();
  }

  async function save() {
    saving = true;
    error = "";
    try {
      await invoke("set_cf_key", { key: key.trim() });
      key = "";
      resetCfSearch();
      toast("CurseForge key saved to Windows Credential Manager.");
      await finish();
    } catch (e) {
      error = String(e);
    }
    saving = false;
  }
</script>

<div class="back">
  <div class="card" role="dialog" aria-label="Welcome to Obelisk">
    <h1>Welcome to Obelisk</h1>
    <p class="lead">
      Before you start: Obelisk searches <strong>Modrinth</strong> out of the box. To also see <strong>CurseForge</strong>
      mods, pages and updates, it needs a free CurseForge API key of your own. It takes about a minute.
    </p>

    <div class="cols">
      <ol class="steps">
        <li>
          <strong>Open the CurseForge console</strong>
          <span>Sign in (or create a free account) on CurseForge for Studios.</span>
          <button class="link" onclick={() => openUrl("https://console.curseforge.com/")}>console.curseforge.com ↗</button>
        </li>
        <li><strong>Go to “API keys”</strong><span>It's in the menu on the left.</span></li>
        <li><strong>Copy your key</strong><span>Use the copy button next to it.</span></li>
        <li><strong>Paste it here and save</strong><span>It's kept in Windows Credential Manager, never in a file.</span></li>
      </ol>

      <!-- A drawn sketch of the console page, not a screenshot of CurseForge's site. -->
      <svg class="sketch" viewBox="0 0 320 200" role="img" aria-label="Sketch: the API keys page with a copy button next to your key">
        <rect x="1" y="1" width="318" height="198" rx="10" class="frame" />
        <rect x="1" y="1" width="318" height="26" rx="10" class="bar" />
        <circle cx="16" cy="14" r="4" class="dot" /><circle cx="30" cy="14" r="4" class="dot" /><circle cx="44" cy="14" r="4" class="dot" />
        <rect x="12" y="40" width="80" height="12" rx="3" class="ghost" />
        <rect x="12" y="60" width="80" height="12" rx="3" class="ghost" />
        <rect x="8" y="78" width="90" height="20" rx="4" class="hl" />
        <text x="16" y="92" class="t strong">API keys</text>
        <text x="72" y="92" class="t num">2</text>
        <rect x="12" y="106" width="80" height="12" rx="3" class="ghost" />
        <text x="114" y="56" class="t strong big">API keys</text>
        <rect x="114" y="72" width="190" height="34" rx="6" class="field" />
        <text x="124" y="93" class="t mono">$2a$10$••••••••••••••</text>
        <rect x="268" y="77" width="28" height="24" rx="5" class="copy" />
        <rect x="276" y="83" width="9" height="11" rx="1.5" class="copyicon" /><rect x="279" y="86" width="9" height="11" rx="1.5" class="copyicon" />
        <text x="236" y="126" class="t num">3 copy</text>
        <path d="M262 120 Q276 116 280 104" class="arrow" />
        <text x="14" y="128" class="t num">← 2</text>
      </svg>
    </div>

    <div class="paste">
      <input type="password" placeholder="Paste your CurseForge API key" bind:value={key} autocomplete="off" spellcheck="false" />
      <button class="primary" disabled={!key.trim() || saving} onclick={save}>{saving ? "Saving…" : "Save and start"}</button>
    </div>
    {#if error}<p class="err">{error}</p>{/if}
    <p class="fine">
      Some personal keys can't use CurseForge's search. If yours can't, Obelisk still uses it for updates, project pages and
      installs, and searches Modrinth. You can change the key any time in Settings → CurseForge API.
    </p>
    <div class="foot"><button class="skip" onclick={finish}>Skip for now</button></div>
  </div>
</div>

<style>
  .back { position: fixed; inset: 0; z-index: 200; background: rgb(0 0 0 / 0.6); display: grid; place-items: center; padding: 16px; }
  .card { background: var(--panel); color: var(--text); border-radius: 12px; padding: 28px 32px 20px; width: min(820px, 100%); max-height: 100%; overflow: auto; box-shadow: 0 20px 60px rgb(0 0 0 / 0.5); }
  h1 { margin: 0 0 8px; font-size: 24px; }
  .lead { color: var(--text-2); margin: 0 0 20px; line-height: 1.5; }
  .cols { display: grid; grid-template-columns: 1fr 320px; gap: 24px; align-items: center; }
  @media (max-width: 760px) { .cols { grid-template-columns: 1fr; } }
  .steps { margin: 0; padding-left: 22px; display: grid; gap: 14px; }
  .steps li::marker { color: var(--accent); font-weight: 700; }
  .steps strong { display: block; }
  .steps span { color: var(--muted); font-size: 13px; }
  .link { background: none; border: 0; color: var(--accent); padding: 2px 0; display: block; }
  .link:hover { text-decoration: underline; }
  .sketch { width: 100%; height: auto; }
  .frame { fill: var(--bg); stroke: var(--line); }
  .bar { fill: var(--panel-2); }
  .dot { fill: var(--line); }
  .ghost { fill: var(--panel-2); }
  .hl { fill: color-mix(in srgb, var(--accent) 22%, transparent); stroke: var(--accent); }
  .field { fill: var(--panel); stroke: var(--line); }
  .copy { fill: var(--accent); }
  .copyicon { fill: none; stroke: #fff; stroke-width: 1.4; }
  .arrow { fill: none; stroke: var(--accent); stroke-width: 1.5; }
  .t { fill: var(--text); font-size: 11px; font-family: inherit; }
  .t.strong { font-weight: 700; }
  .t.big { font-size: 15px; }
  .t.mono { font-family: ui-monospace, monospace; fill: var(--muted); }
  .t.num { fill: var(--accent); font-weight: 700; }
  .paste { display: flex; gap: 10px; margin-top: 22px; }
  .paste input { flex: 1; min-width: 0; background: var(--bg); border: 1px solid var(--line); color: var(--text); border-radius: 6px; padding: 10px 12px; }
  .primary { background: var(--accent); color: #fff; border: 0; border-radius: 6px; padding: 10px 18px; font-weight: 600; white-space: nowrap; }
  .primary:hover:not(:disabled) { background: var(--accent-hover); }
  .primary:disabled { opacity: 0.5; }
  .err { color: var(--err-text); margin: 8px 0 0; }
  .fine { color: var(--muted); font-size: 12.5px; margin: 12px 0 0; line-height: 1.5; }
  .foot { display: flex; justify-content: flex-end; margin-top: 10px; }
  .skip { background: none; border: 0; color: var(--muted); padding: 6px 4px; }
  .skip:hover { color: var(--text); }
</style>
