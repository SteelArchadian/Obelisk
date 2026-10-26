<script lang="ts" module>
  // Right-click menu for a modpack, shared by the library and the side rail.
  import { app } from "./state.svelte";
  import type { Instance } from "./api";

  export function openPackMenu(e: MouseEvent, inst: Instance) {
    e.preventDefault();
    app.packMenu = { x: Math.min(e.clientX, innerWidth - 260), y: Math.min(e.clientY, innerHeight - 360), inst };
  }
</script>

<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { openBrowse, toast } from "./state.svelte";
  import { setTarget, syncInstances } from "./install";
  import { kill, play, running } from "./play.svelte";

  const close = () => (app.packMenu = null);

  // Change Picture: any picture, cropped to a square and shrunk to 256 px here, so it stays small enough to share.
  let picker: HTMLInputElement;
  let pictureFor: Instance | null = null;
  function choosePicture(i: Instance) {
    pictureFor = i;
    picker.value = "";
    picker.click();
  }
  async function setPicture() {
    const file = picker.files?.[0];
    const i = pictureFor;
    if (!file || !i) return;
    try {
      const img = await createImageBitmap(file);
      const side = Math.min(img.width, img.height);
      const c = document.createElement("canvas");
      c.width = c.height = Math.min(256, side);
      c.getContext("2d")!.drawImage(img, (img.width - side) / 2, (img.height - side) / 2, side, side, 0, 0, c.width, c.height);
      const png = c.toDataURL("image/png").split(",")[1];
      await invoke("set_instance_icon", { instanceId: i.id, png });
      await syncInstances();
      toast(`New picture for ${i.name}.`);
    } catch (e) { toast(e instanceof Error ? "Obelisk couldn't read that picture." : String(e)); }
  }
  function act(fn: (inst: Instance) => void) {
    const inst = app.packMenu!.inst;
    close();
    fn(inst);
  }
</script>

<svelte:window onclick={close} onkeydown={(e) => e.key === "Escape" && close()} />

{#if app.packMenu}
  {@const m = app.packMenu}
  <div class="ctx" style:left="{m.x}px" style:top="{m.y}px" role="menu" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
    {#if running[m.inst.id]}<button class="danger" onclick={() => act(kill)} title="Force-close the game (for when it's frozen)">■ Kill instance</button>{:else}<button onclick={() => act(play)}>▶ Play</button>{/if}
    <button onclick={() => act((i) => { setTarget(i); app.current = i; openBrowse("mod"); })}>＋ Add Content</button>
    <hr />
    <button onclick={() => act((i) => invoke("reveal", { instanceId: i.id, sub: "" }))}>Open Folder ↗</button>
    <button onclick={() => act((i) => (app.profileDialog = { kind: "options", inst: i }))}>Profile Settings…</button>
    <button onclick={() => act(choosePicture)}>Change Picture…</button>
    <button onclick={() => act((i) => (app.profileDialog = { kind: "options", inst: i, tab: "backups" }))}>Backups…</button>
    <button onclick={() => act((i) => (app.profileDialog = { kind: "clone", inst: i }))}>Clone…</button>
    <button onclick={() => act((i) => (app.serverPack = i))}>Make Server &amp; Client Packs…</button>
    <button onclick={() => act((i) => (app.packShare = i))}>Share…</button>
    <button onclick={() => act((i) => (app.sharePack = i))}>Collaborate…{#if app.shareIncoming[m.inst.id]}<span class="inc">{app.shareIncoming[m.inst.id]} new</span>{/if}</button>
    <hr />
    <button class="danger" onclick={() => act((i) => (app.profileDialog = { kind: "purge", inst: i }))}>Purge Configs…</button>
    <button class="danger" onclick={() => act((i) => (app.profileDialog = { kind: "delete", inst: i }))}>Delete Profile</button>
  </div>
{/if}
<input type="file" accept="image/png,image/jpeg,image/gif,image/webp" hidden bind:this={picker} onchange={setPicture} />

<style>
  .ctx { position: fixed; z-index: 45; width: 250px; background: var(--panel-2); border: 1px solid var(--line); border-radius: 8px; padding: 6px; box-shadow: 0 12px 34px rgb(0 0 0 / 0.5); }
  .ctx button { display: block; width: 100%; background: none; border: 0; padding: 8px 12px; border-radius: 5px; text-align: left; }
  .ctx button:hover { background: var(--panel); }
  .ctx .danger { color: var(--danger); }
  .inc { margin-left: 8px; font-size: 11px; background: var(--accent); color: #fff; border-radius: 999px; padding: 1px 7px; }
  .ctx hr { border: 0; border-top: 1px solid var(--line); margin: 4px 6px; }
</style>
