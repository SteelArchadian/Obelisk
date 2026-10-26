import DOMPurify from "dompurify";
import { marked } from "marked";

const YT = /^(?:https?:)?\/\/(?:www\.)?(?:youtube\.com|youtube-nocookie\.com)\/embed\/([\w-]{6,})/;

/** Markdown/HTML project body -> sanitized HTML. Only YouTube iframes survive, rewritten for the player API. */
export function renderBody(body: string): string {
  const html = marked.parse(body ?? "", { async: false, gfm: true, breaks: false }) as string;
  DOMPurify.removeAllHooks();
  DOMPurify.addHook("uponSanitizeElement", (node, data) => {
    if (data.tagName !== "iframe") return;
    const el = node as Element;
    const m = YT.exec(el.getAttribute("src") ?? "");
    if (!m) {
      el.parentNode?.removeChild(el);
      return;
    }
    el.setAttribute("src", `https://www.youtube-nocookie.com/embed/${m[1]}?enablejsapi=1&rel=0`);
    el.setAttribute("data-yt", "1");
  });
  DOMPurify.addHook("afterSanitizeAttributes", (node) => {
    if (node.tagName === "IMG") {
      node.setAttribute("loading", "lazy");
      node.setAttribute("decoding", "async");
    }
  });
  return DOMPurify.sanitize(html, {
    ADD_TAGS: ["iframe"],
    ADD_ATTR: ["allow", "allowfullscreen", "frameborder", "data-yt"],
  });
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
