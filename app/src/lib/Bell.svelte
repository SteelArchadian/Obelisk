<script lang="ts">
  // The bell on the left rail: CurseForge notifications (when signed in) in the first tab, Modrinth in the other.
  import { onMount } from "svelte";
  import { mr, cf, unread, cfUnread, loadAccount, loadNotes, markRead, loadCf, loadCfNotes, cfMarkRead, cfSignIn, openLink, type CfNotification } from "./account.svelte";
  import { app, closeOverlays } from "./state.svelte";
  import { openProjectById } from "./install";
  import { ago, api } from "./api";

  let open = $state(false);
  let tab = $state<"modrinth" | "curseforge">("curseforge");
  const count = $derived(unread());
  const cfCount = $derived(cfUnread());
  const mrCount = $derived(count - cfCount);

  onMount(() => {
    loadAccount();
    loadCf();
    const t = setInterval(() => { loadNotes(); loadCfNotes(); }, 10 * 60_000);
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

  // A CurseForge notification opens its project in Obelisk (on its comments or files when that's what it's about);
  // a private message opens on legacy.curseforge.com.
  async function cfGo(n: CfNotification) {
    cfMarkRead([n.id]);
    open = false;
    const id = n.projectId ?? (n.slug ? await api.cfIdBySlug(n.slug).catch(() => null) : null);
    if (id && n.kind !== "message") openProjectById(`cf:${id}`, n.tab ?? (n.kind ? undefined : "comments"));
    else if (n.url) openLink(n.url);
  }

  /** Which little icon a notification gets: comments and replies a speech bubble, moderation (status) an alert. */
  type Icon = "comment" | "alert" | "message" | "invite" | "file" | "other";
  function cfIcon(n: CfNotification): Icon {
    if (n.kind === "comment" || n.kind === "reply") return "comment";
    if (n.kind === "status" || (n.kind === "file" && n.label)) return "alert";
    if (n.kind === "message") return "message";
    if (n.kind === "invite") return "invite";
    return n.kind === "file" ? "file" : "other";
  }
  function mrIcon(type?: string): Icon {
    if (type === "status_change" || type === "moderator_message") return "alert";
    if (type === "team_invite" || type === "organization_invite") return "invite";
    if (type === "project_update") return "file";
    return "other";
  }
  /** The alert takes the moderation result's colour (approved green, changes amber, rejected red). */
  const tone = (label?: string) => (label ?? "").toLowerCase().replace(/\s+/g, "-");

  function settings() {
    open = false;
    closeOverlays();
    app.page = "settings";
    app.settingsSection = "modrinth";
  }
</script>

{#snippet icon(kind: Icon, toneClass: string)}
  <span class="ico {kind} {toneClass}" aria-hidden="true">
    <svg viewBox="0 0 16 16" width="16" height="16" shape-rendering="crispEdges">
      {#if kind === "comment"}<path d="M2 2h12v9H7l-3 3v-3H2z" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="miter" /><path d="M5 5h6v1.5H5zM5 7.5h4V9H5z" fill="currentColor" />
      {:else if kind === "alert"}<path d="M8 1.5L15 14H1z" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="miter" /><path d="M7.25 5.5h1.5v4.5h-1.5zM7.25 11h1.5v1.5h-1.5z" fill="currentColor" />
      {:else if kind === "message"}<path d="M1.5 3.5h13v9h-13z" fill="none" stroke="currentColor" stroke-width="1.5" /><path d="M2 4l6 5 6-5" fill="none" stroke="currentColor" stroke-width="1.5" />
      {:else if kind === "invite"}<path d="M6 2.5a2.5 2.5 0 1 1 0 5a2.5 2.5 0 0 1 0-5zM1.5 14v-1.5c0-2 2-3.5 4.5-3.5s4.5 1.5 4.5 3.5V14" fill="none" stroke="currentColor" stroke-width="1.5" /><path d="M12.25 4h1.5v5h-1.5zM10.5 5.75h5v1.5h-5z" fill="currentColor" />
      {:else if kind === "file"}<path d="M3 1.5h7l3 3v10H3z" fill="none" stroke="currentColor" stroke-width="1.5" /><path d="M7.25 6h1.5v4h1.75L8 12.5L5.5 10h1.75z" fill="currentColor" />
      {:else}<path d="M4 11V7.5a4 4 0 0 1 8 0V11l1 1.5H3zM6.5 14h3" fill="none" stroke="currentColor" stroke-width="1.5" />{/if}
    </svg>
  </span>
{/snippet}

<div class="bell-wrap">
  <button class="bell" class:has={count > 0} title={count ? `${count} unread notification${count === 1 ? "" : "s"}` : "Notifications"}
    aria-label="Notifications" onclick={(e) => { e.stopPropagation(); open = !open; if (open) { loadNotes(); loadCfNotes(); } }}>
    <svg viewBox="0 0 24 24" width="22" height="22"><path d="M6 16V11a6 6 0 0 1 12 0v5l1.5 2h-15zM10 20a2 2 0 0 0 4 0" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round" /></svg>
    {#if count}<span class="dot">{count > 99 ? "99+" : count}</span>{/if}
  </button>
  {#if open}
    <div class="pop" role="dialog" aria-label="Notifications" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && (open = false)}>
      <div class="tabs">
        <button class:on={tab === "curseforge"} onclick={() => (tab = "curseforge")}>CurseForge{#if cfCount} ({cfCount}){/if}</button>
        <button class:on={tab === "modrinth"} onclick={() => (tab = "modrinth")}>Modrinth{#if mrCount} ({mrCount}){/if}</button>
      </div>
      {#if tab === "modrinth"}
        {#if !mr.account}
          <p class="muted">Sign in to Modrinth to see your notifications and follow projects.</p>
          <button class="primary" onclick={settings}>Sign in</button>
        {:else}
          {#if mrCount}<button class="link all" onclick={() => markRead(mr.notes.filter((n) => !n.read).map((n) => n.id))}>Mark all read</button>{/if}
          <div class="list">
            {#each mr.notes.slice(0, 50) as n (n.id)}
              <button class="note" class:unread={!n.read} onclick={() => { markRead([n.id]); go(n.link); }}>
                {@render icon(mrIcon(n.type), "")}
                <div class="body">
                  <strong>{n.title}</strong>
                  {#if n.text}<span>{n.text}</span>{/if}
                  <small>{ago(n.created)}</small>
                </div>
              </button>
            {:else}<p class="muted">No notifications.</p>{/each}
          </div>
        {/if}
      {:else}
        {#if !cf.account}
          <p class="muted">Sign in to CurseForge to see your notifications here (approvals, status changes, comments and replies), comment from Obelisk and keep favorites.</p>
          <button class="primary" onclick={cfSignIn}>{cf.signingIn ? "Show the CurseForge sign-in window" : "Sign in to CurseForge"}</button>
        {:else}
          {#if cfCount}<button class="link all" onclick={() => cfMarkRead(cf.notes.filter((n) => !n.read).map((n) => n.id))}>Mark all read</button>{/if}
          <div class="list">
            {#each cf.notes.slice(0, 50) as n (n.id)}
              <button class="note" class:unread={!n.read} onclick={() => cfGo(n)}>
                {@render icon(cfIcon(n), tone(n.label))}
                <div class="body">
                  <strong>{n.title}</strong>
                  {#if n.text}<span>{n.text}</span>{/if}
                  <small>{#if n.label}<em class="label {tone(n.label)}">{n.label}</em> {/if}{ago(n.created)}</small>
                </div>
              </button>
            {:else}<p class="muted">No notifications yet. Approvals, status changes, comments on your projects and replies to you show up here.</p>{/each}
          </div>
        {/if}
      {/if}
    </div>
  {/if}
</div>

<style>
  .bell-wrap { position: relative; }
  .bell { position: relative; width: 44px; height: 44px; display: grid; place-items: center; border: 0; border-radius: 10px; background: none; color: var(--muted); }
  .bell:hover, .bell.has { color: var(--text); }
  .dot { position: absolute; top: 4px; right: 2px; min-width: 16px; height: 16px; padding: 0 4px; border-radius: 999px; background: var(--accent); color: #fff; font-size: 10px; display: grid; place-items: center; }
  .pop { position: absolute; left: calc(100% + 12px); bottom: -6px; z-index: 40; width: min(420px, calc(100vw - 90px)); max-height: min(600px, calc(100vh - 40px)); display: flex; flex-direction: column; gap: 10px;
    background: var(--panel-2); border: 1px solid var(--line); border-radius: 8px; padding: 10px; box-shadow: 0 12px 34px rgb(0 0 0 / 0.5); }
  .tabs { display: flex; gap: 4px; border-bottom: 1px solid var(--line); }
  .tabs button { flex: 1; background: none; border: 0; padding: 6px 8px 8px; color: var(--muted); border-bottom: 2px solid transparent; margin-bottom: -1px; }
  .tabs button.on { color: var(--text); border-bottom-color: var(--accent); font-weight: 600; }
  /* Up and down only: a note (themes frame it with a thick border) always fits the list's width. */
  .list { overflow-y: auto; overflow-x: hidden; display: flex; flex-direction: column; gap: 4px; min-height: 0; }
  .note { box-sizing: border-box; width: 100%; min-width: 0; flex: none; display: flex; gap: 9px; align-items: flex-start; text-align: left; background: none; border: 0; border-left: 3px solid transparent; padding: 7px 8px; border-radius: 4px; color: var(--text-2); }
  .note:hover { background: var(--panel); }
  .note.unread { border-left-color: var(--accent); color: var(--text); }
  .body { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; overflow-wrap: anywhere; }
  .ico { flex: none; width: 24px; height: 24px; display: grid; place-items: center; margin-top: 1px; border-radius: 4px; background: var(--panel); color: var(--muted); }
  .ico.comment { color: #6aa8ff; }
  .ico.alert { color: #e0a040; }
  .ico.alert.approved, .ico.alert.released { color: #5cc87a; }
  .ico.alert.rejected, .ico.alert.deleted, .ico.alert.malware-detected { color: #e05c5c; }
  .ico.message { color: #b48cff; }
  .ico.invite, .ico.file { color: var(--text-2); }
  .body span { font-size: 13px; color: var(--muted); white-space: pre-line; }
  .body small { font-size: 11px; color: var(--muted); }
  .label { font-style: normal; font-weight: 600; padding: 1px 6px; border-radius: 999px; background: var(--panel); color: var(--text-2); }
  .label.approved, .label.released { color: #5cc87a; }
  .label.changes-required, .label.under-review, .label.under-manual-review { color: #e0a040; }
  .label.rejected, .label.deleted, .label.malware-detected { color: #e05c5c; }
  .muted { color: var(--muted); margin: 0; font-size: 13px; line-height: 1.5; }
  .primary { background: var(--accent); color: #fff; border: 0; border-radius: 4px; padding: 7px 14px; font-weight: 600; align-self: flex-start; }
  .link { background: none; border: 0; color: var(--accent); padding: 0; }
  .all { align-self: flex-end; font-size: 12px; }
</style>
