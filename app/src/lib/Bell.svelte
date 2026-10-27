<script lang="ts">
  // The bell on the left rail: Modrinth notifications (when signed in) in one tab, CurseForge in the other.
  import { onMount } from "svelte";
  import { mr, unread, loadAccount, loadNotes, markRead, openLink } from "./account.svelte";
  import { app, closeOverlays } from "./state.svelte";
  import { openProjectById } from "./install";
  import { ago } from "./api";

  let open = $state(false);
  let tab = $state<"modrinth" | "curseforge">("modrinth");
  const count = $derived(unread());

  onMount(() => {
    loadAccount();
    const t = setInterval(loadNotes, 10 * 60_000);
    const close = () => (open = false);
    window.addEventListener("click", close);
    return () => { clearInterval(t); window.removeEventListener("click", close); };
  });

  // A Modrinth link inside Obelisk when it's a project (version pages open the project), else the browser.
  function go(link: string) {
    const m = link.match(/^\/?(?:mod|plugin|datapack|shader|resourcepack|modpack|project)\/([^/]+)/);
    open = false;
    if (m) openProjectById(m[1]);
    else openLink(`https://modrinth.com${link.startsWith("/") ? "" : "/"}${link}`);
  }

  function settings() {
    open = false;
    closeOverlays();
    app.page = "settings";
    app.settingsSection = "modrinth";
  }
</script>

<div class="bell-wrap">
  <button class="bell" class:has={count > 0} title={count ? `${count} unread notification${count === 1 ? "" : "s"}` : "Notifications"}
    aria-label="Notifications" onclick={(e) => { e.stopPropagation(); open = !open; if (open) loadNotes(); }}>
    <svg viewBox="0 0 24 24" width="22" height="22"><path d="M6 16V11a6 6 0 0 1 12 0v5l1.5 2h-15zM10 20a2 2 0 0 0 4 0" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round" /></svg>
    {#if count}<span class="dot">{count > 99 ? "99+" : count}</span>{/if}
  </button>
  {#if open}
    <div class="pop" role="dialog" aria-label="Notifications" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && (open = false)}>
      <div class="tabs">
        <button class:on={tab === "modrinth"} onclick={() => (tab = "modrinth")}>Modrinth{#if count} ({count}){/if}</button>
        <button class:on={tab === "curseforge"} onclick={() => (tab = "curseforge")}>CurseForge</button>
      </div>
      {#if tab === "modrinth"}
        {#if !mr.account}
          <p class="muted">Sign in to Modrinth to see your notifications and follow projects.</p>
          <button class="primary" onclick={settings}>Sign in</button>
        {:else}
          {#if count}<button class="link all" onclick={() => markRead(mr.notes.filter((n) => !n.read).map((n) => n.id))}>Mark all read</button>{/if}
          <div class="list">
            {#each mr.notes.slice(0, 50) as n (n.id)}
              <button class="note" class:unread={!n.read} onclick={() => { markRead([n.id]); go(n.link); }}>
                <strong>{n.title}</strong>
                {#if n.text}<span>{n.text}</span>{/if}
                <small>{ago(n.created)}</small>
              </button>
            {:else}<p class="muted">No notifications.</p>{/each}
          </div>
        {/if}
      {:else}
        <p class="muted">CurseForge doesn't let other apps sign in, so its notifications and follows can't show here.</p>
        <button class="ghost" onclick={() => { open = false; openLink("https://www.curseforge.com/notifications"); }}>Open CurseForge notifications ↗</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .bell-wrap { position: relative; }
  .bell { position: relative; width: 44px; height: 44px; display: grid; place-items: center; border: 0; border-radius: 10px; background: none; color: var(--muted); }
  .bell:hover, .bell.has { color: var(--text); }
  .dot { position: absolute; top: 4px; right: 2px; min-width: 16px; height: 16px; padding: 0 4px; border-radius: 999px; background: var(--accent); color: #fff; font-size: 10px; display: grid; place-items: center; }
  .pop { position: absolute; left: calc(100% + 12px); bottom: -6px; z-index: 40; width: min(340px, calc(100vw - 90px)); max-height: min(480px, calc(100vh - 40px)); display: flex; flex-direction: column; gap: 10px;
    background: var(--panel-2); border: 1px solid var(--line); border-radius: 8px; padding: 10px; box-shadow: 0 12px 34px rgb(0 0 0 / 0.5); }
  .tabs { display: flex; gap: 4px; border-bottom: 1px solid var(--line); }
  .tabs button { flex: 1; background: none; border: 0; padding: 6px 8px 8px; color: var(--muted); border-bottom: 2px solid transparent; margin-bottom: -1px; }
  .tabs button.on { color: var(--text); border-bottom-color: var(--accent); font-weight: 600; }
  .list { overflow-y: auto; display: flex; flex-direction: column; gap: 4px; min-height: 0; }
  .note { display: flex; flex-direction: column; gap: 2px; text-align: left; background: none; border: 0; border-left: 3px solid transparent; padding: 7px 8px; border-radius: 4px; color: var(--text-2); }
  .note:hover { background: var(--panel); }
  .note.unread { border-left-color: var(--accent); color: var(--text); }
  .note span { font-size: 13px; color: var(--muted); }
  .note small { font-size: 11px; color: var(--muted); }
  .muted { color: var(--muted); margin: 0; font-size: 13px; line-height: 1.5; }
  .primary { background: var(--accent); color: #fff; border: 0; border-radius: 4px; padding: 7px 14px; font-weight: 600; align-self: flex-start; }
  .ghost { background: none; border: 1px solid var(--line); border-radius: 4px; padding: 7px 12px; color: inherit; align-self: flex-start; }
  .link { background: none; border: 0; color: var(--accent); padding: 0; }
  .all { align-self: flex-end; font-size: 12px; }
</style>
