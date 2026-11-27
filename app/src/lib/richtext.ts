import DOMPurify from "dompurify";
import { marked } from "marked";

const YT = /^(?:https?:)?\/\/(?:www\.)?(?:youtube\.com|youtube-nocookie\.com)\/embed\/([\w-]{6,})/;

function hooks(forPlayer = true) {
  DOMPurify.removeAllHooks();
  DOMPurify.addHook("uponSanitizeElement", (node, data) => {
    if (data.tagName !== "iframe") return;
    const el = node as Element;
    const m = YT.exec(el.getAttribute("src") ?? "");
    if (!m) {
      el.parentNode?.removeChild(el);
      return;
    }
    if (!forPlayer) return el.setAttribute("src", `https://www.youtube.com/embed/${m[1]}`);
    el.setAttribute("src", `https://www.youtube-nocookie.com/embed/${m[1]}?enablejsapi=1&rel=0`);
    el.setAttribute("data-yt", "1");
  });
  DOMPurify.addHook("afterSanitizeAttributes", (node) => {
    if (node.tagName === "IMG") {
      // Old descriptions link images over plain http, which the app's security policy won't load; the same hosts
      // nearly always serve them over https too (puu.sh, imgur, ...).
      const src = node.getAttribute("src") ?? "";
      if (src.startsWith("http://")) node.setAttribute("src", "https://" + src.slice(7));
      node.setAttribute("loading", "lazy");
      node.setAttribute("decoding", "async");
    }
  });
}

const EMBED = { ADD_TAGS: ["iframe"], ADD_ATTR: ["allow", "allowfullscreen", "frameborder", "data-yt"] };

/** Markdown/HTML project body -> sanitized HTML. Only YouTube iframes survive, rewritten for the player API. */
export function renderBody(body: string): string {
  const html = marked.parse(body ?? "", { async: false, gfm: true, breaks: false }) as string;
  hooks();
  return DOMPurify.sanitize(html, EMBED);
}

/** A comment's HTML (legacy CurseForge or new) -> sanitized HTML: formatting, quotes, spoilers, pictures and YouTube
 *  videos kept; scripts, forms and other embeds dropped. */
export function cleanHtml(html: string): string {
  hooks();
  return DOMPurify.sanitize(html ?? "", { ...EMBED, FORBID_TAGS: ["style", "form", "input", "button", "textarea", "select"] });
}

const YT_URL = /^(?:https?:\/\/)?(?:www\.|m\.)?(?:youtube\.com\/(?:watch\?(?:.*&)?v=|shorts\/|embed\/)|youtu\.be\/)([\w-]{6,})\S*$/;
const IMG_URL = /^https?:\/\/\S+\.(?:png|jpe?g|gif|webp|bmp)(?:\?\S*)?$/i;

/** What the user typed in a comment box (Markdown, plus ||spoiler|| and raw <u>) -> the HTML CurseForge stores.
 *  A picture or YouTube link alone on a line becomes the picture / the video. */
export function composeHtml(text: string): string {
  const lines = (text ?? "").replace(/\r\n?/g, "\n").split("\n").map((l) => {
    const t = l.trim();
    const yt = YT_URL.exec(t);
    if (yt) return `\n<iframe src="https://www.youtube.com/embed/${yt[1]}" width="560" height="315" frameborder="0" allowfullscreen></iframe>\n`;
    if (IMG_URL.test(t)) return `![](${t})`;
    return l;
  });
  let md = lines.join("\n");
  // ||spoiler|| -> a click-to-show span.
  md = md.replace(/\|\|([^|\n][^\n]*?)\|\|/g, '<span class="spoiler">$1</span>');
  const html = marked.parse(md, { async: false, gfm: true, breaks: true }) as string;
  hooks(false);
  return DOMPurify.sanitize(html, EMBED).trim();
}

// ---- YouTube volume memory ----
const KEY = "obelisk.ytVolume";

function saved(): { volume: number; muted: boolean } {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "") ?? { volume: 50, muted: false };
  } catch {
    return { volume: 50, muted: false };
  }
}

function send(frame: HTMLIFrameElement, msg: object) {
  frame.contentWindow?.postMessage(JSON.stringify(msg), "*");
}

let listening = false;

/** Ask embedded players for state updates, apply the remembered volume, and remember changes. */
export function wireYouTube(root: HTMLElement) {
  const frames = Array.from(root.querySelectorAll<HTMLIFrameElement>("iframe[data-yt]"));
  for (const f of frames) {
    f.addEventListener("load", () => send(f, { event: "listening", id: f.src }));
  }
  if (listening) return;
  listening = true;
  window.addEventListener("message", (e) => {
    if (!/youtube(-nocookie)?\.com$/.test(new URL(e.origin).hostname)) return;
    let data: any;
    try { data = typeof e.data === "string" ? JSON.parse(e.data) : e.data; } catch { return; }
    const frame = Array.from(document.querySelectorAll<HTMLIFrameElement>("iframe[data-yt]"))
      .find((f) => f.contentWindow === e.source);
    if (!frame) return;
    if (data.event === "onReady" || data.event === "initialDelivery") {
      const { volume, muted } = saved();
      send(frame, { event: "command", func: "setVolume", args: [volume] });
      send(frame, { event: "command", func: muted ? "mute" : "unMute", args: [] });
    } else if (data.event === "infoDelivery" && data.info && typeof data.info.volume === "number") {
      const prev = saved();
      const next = { volume: data.info.volume, muted: data.info.muted ?? prev.muted };
      if (next.volume !== prev.volume || next.muted !== prev.muted) {
        try { localStorage.setItem(KEY, JSON.stringify(next)); } catch { /* storage unavailable */ }
      }
    }
  });
}
