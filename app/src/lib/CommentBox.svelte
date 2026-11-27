<script lang="ts">
  // The comment / reply box: Markdown with a toolbar for everything CurseForge comments show (bold, italic, underline,
  // strike, headings, links, pictures, YouTube videos, quotes, code, spoilers, lists) and a Preview of the result.
  import { cleanHtml, composeHtml, wireYouTube } from "./richtext";

  let {
    value = $bindable(""),
    placeholder = "",
    busy = false,
    label = "Post",
    rows = 3,
    autofocus = false,
    onsubmit,
    oncancel,
  }: {
    value?: string;
    placeholder?: string;
    busy?: boolean;
    label?: string;
    rows?: number;
    autofocus?: boolean;
    onsubmit: (html: string) => void;
    oncancel?: () => void;
  } = $props();

  let ta = $state<HTMLTextAreaElement>();
  let preview = $state(false);
  let note = $state("");
  let previewEl = $state<HTMLElement>();
  const focusEl = (el: HTMLElement) => el.focus();

  $effect(() => {
    if (autofocus) ta?.focus();
  });
  $effect(() => {
    if (preview && previewEl) wireYouTube(previewEl);
  });

  // Wrap the selection (or a placeholder word) in before/after, keeping the inner text selected.
  function wrap(before: string, after = before, hint = "text") {
    if (!ta) return;
    preview = false;
    const s = ta.selectionStart, e = ta.selectionEnd;
    const inner = value.slice(s, e) || hint;
    value = value.slice(0, s) + before + inner + after + value.slice(e);
    queueMicrotask(() => { ta!.focus(); ta!.setSelectionRange(s + before.length, s + before.length + inner.length); });
  }

  // Prefix every selected line (quote, lists, heading).
  function lines(prefix: (i: number) => string) {
    if (!ta) return;
    preview = false;
    const s = value.lastIndexOf("\n", ta.selectionStart - 1) + 1;
    let e = value.indexOf("\n", ta.selectionEnd);
    if (e < 0) e = value.length;
    const out = value.slice(s, e).split("\n").map((l, i) => prefix(i) + l).join("\n");
    value = value.slice(0, s) + out + value.slice(e);
    queueMicrotask(() => { ta!.focus(); ta!.setSelectionRange(s, s + out.length); });
  }

  // Put something on a line of its own at the cursor.
  function block(text: string) {
    if (!ta) return;
    preview = false;
    const s = ta.selectionStart, e = ta.selectionEnd;
    const pre = s > 0 && value[s - 1] !== "\n" ? "\n" : "";
    const post = value[e] && value[e] !== "\n" ? "\n" : "";
    value = value.slice(0, s) + pre + text + post + value.slice(e);
    const at = s + pre.length + text.length;
    queueMicrotask(() => { ta!.focus(); ta!.setSelectionRange(at, at); });
  }

  // Link / picture / video: a small address field under the toolbar (no browser pop-ups in the app window).
  let ask = $state<{ label: string; done: (url: string) => void } | null>(null);
  let askUrl = $state("");
  let saved = { s: 0, e: 0 };
  function askFor(label: string, done: (url: string) => void) {
    saved = { s: ta?.selectionStart ?? value.length, e: ta?.selectionEnd ?? value.length };
    askUrl = "";
    ask = { label, done };
  }
  function askDone() {
    const url = askUrl.trim(), a = ask;
    ask = null;
    if (!url || !a || !ta) return;
    ta.setSelectionRange(saved.s, saved.e);
    a.done(url);
  }
  const link = () => askFor("Link address", (url) => wrap("[", `](${url})`, "link text"));
  const picture = () => askFor("Picture link (png, jpg, gif, webp)", (url) => block(`![](${url})`));
  const video = () => askFor("YouTube link", (url) => block(url));

  // CurseForge has no picture upload for comments: say so when someone pastes or drops an image file.
  function fileIn(e: ClipboardEvent | DragEvent) {
    const files = "clipboardData" in e ? e.clipboardData?.files : (e as DragEvent).dataTransfer?.files;
    if (!files?.length || ![...files].some((f) => f.type.startsWith("image/"))) return;
    e.preventDefault();
    note = "CurseForge comments can't hold uploaded pictures. Upload it somewhere (imgur, Discord…) and paste its link.";
  }

  function key(e: KeyboardEvent) {
    if (!(e.ctrlKey || e.metaKey)) return;
    const k = e.key.toLowerCase();
    if (k === "b") { e.preventDefault(); wrap("**"); }
    else if (k === "i") { e.preventDefault(); wrap("*"); }
    else if (k === "u") { e.preventDefault(); wrap("<u>", "</u>"); }
    else if (k === "k") { e.preventDefault(); link(); }
    else if (k === "enter" && value.trim() && !busy) { e.preventDefault(); submit(); }
  }

  function submit() {
    note = "";
    const html = composeHtml(value);
    if (html) onsubmit(html);
  }

  const tools: [string, string, () => void][] = [
    ["B", "Bold (Ctrl+B)", () => wrap("**")],
    ["I", "Italic (Ctrl+I)", () => wrap("*")],
    ["U", "Underline (Ctrl+U)", () => wrap("<u>", "</u>")],
    ["S", "Strikethrough", () => wrap("~~")],
    ["H", "Heading", () => lines(() => "### ")],
    ["🔗", "Link (Ctrl+K)", link],
    ["🖼", "Picture by link", picture],
    ["▶", "YouTube video", video],
    ["❝", "Quote", () => lines(() => "> ")],
    ["</>", "Code", () => (ta && value.slice(ta.selectionStart, ta.selectionEnd).includes("\n") ? wrap("```\n", "\n```", "code") : wrap("`", "`", "code"))],
    ["👁", "Spoiler (hidden until clicked)", () => wrap("||", "||", "spoiler")],
    ["•", "Bullet list", () => lines(() => "- ")],
    ["1.", "Numbered list", () => lines((i) => `${i + 1}. `)],
  ];
</script>

<div class="box">
  <div class="tools" role="toolbar" aria-label="Formatting">
    {#each tools as [icon, title, act] (title)}
      <button type="button" class="tool" class:b={icon === "B"} class:i={icon === "I"} class:u={icon === "U"} class:s={icon === "S"} {title} aria-label={title} disabled={busy} onclick={act}>{icon}</button>
    {/each}
    <span class="grow"></span>
    <button type="button" class="tool tab" class:on={preview} disabled={!value.trim()} onclick={() => (preview = !preview)}>{preview ? "Write" : "Preview"}</button>
  </div>
  {#if preview}
    <div class="rich body preview" bind:this={previewEl}>{@html cleanHtml(composeHtml(value))}</div>
  {/if}
    <textarea hidden={preview} bind:this={ta} {rows} {placeholder} bind:value disabled={busy} onkeydown={key} onpaste={fileIn} ondrop={fileIn}></textarea>
  {#if ask}
    <div class="ask">
      <input placeholder={ask.label} bind:value={askUrl} use:focusEl onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); askDone(); } else if (e.key === "Escape") ask = null; }} />
      <button class="ghost" onclick={() => (ask = null)}>Cancel</button>
      <button class="primary" disabled={!askUrl.trim()} onclick={askDone}>Insert</button>
    </div>
  {/if}
  {#if note}<p class="note">{note}</p>{/if}
  <div class="row">
    <small class="muted hint">Markdown works. ||spoiler||. A picture or YouTube link on its own line shows inline.</small>
    {#if oncancel}<button class="ghost" onclick={oncancel}>Cancel</button>{/if}
    <button class="primary" disabled={!value.trim() || busy} onclick={submit}>{busy ? "Posting…" : label}</button>
  </div>
</div>

<style>
  .box { display: flex; flex-direction: column; gap: 6px; }
  .tools { display: flex; flex-wrap: wrap; gap: 2px; align-items: center; }
  .tool { background: none; border: 1px solid transparent; border-radius: 4px; color: inherit; min-width: 28px; min-height: 26px; padding: 0 6px; line-height: 1; font-size: 13px; }
  .tool:hover:not(:disabled) { background: var(--panel); border-color: var(--line); }
  .tool.b { font-weight: 800; }
  .tool.i { font-style: italic; }
  .tool.u { text-decoration: underline; }
  .tool.s { text-decoration: line-through; }
  .tool.tab { border-color: var(--line); padding: 0 10px; }
  .tool.on { background: var(--panel); }
  .grow { flex: 1; }
  textarea, .preview { width: 100%; background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 6px; padding: 8px 10px; font: inherit; }
  textarea { resize: vertical; }
  .preview { min-height: 64px; overflow-wrap: anywhere; }
  .row { display: flex; gap: 8px; justify-content: flex-end; align-items: center; }
  .hint { margin-right: auto; }
  .ask { display: flex; gap: 6px; }
  .ask input { flex: 1; background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 4px; padding: 6px 8px; font: inherit; }
  .note { margin: 0; color: var(--warn, #e0a040); font-size: 13px; }
  .muted { color: var(--muted); }
  .primary { background: var(--accent); color: #fff; border: 0; border-radius: 4px; padding: 7px 14px; font-weight: 600; }
  .primary:disabled { opacity: 0.6; }
  .ghost { background: none; border: 1px solid var(--line); border-radius: 4px; padding: 7px 12px; color: inherit; }
</style>
