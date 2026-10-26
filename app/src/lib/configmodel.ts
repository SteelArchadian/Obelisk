// Turns a config file into editable settings, the way the Configured mod shows them in game.
// TOML / .properties / old Forge .cfg are edited line by line (comments and layout survive);
// JSON is parsed and written back with the file's own indentation.

export type Kind = "bool" | "int" | "float" | "string" | "enum" | "list" | "raw";
export type Value = string | number | boolean | Value[] | null | { [k: string]: Value };

export type Setting = {
  id: string;
  /** Folder path, e.g. ["client", "sound"]. */
  path: string[];
  key: string;
  kind: Kind;
  value: Value;
  help: string;
  min?: number;
  max?: number;
  options?: string[];
  /** Value from the comment's "Default:" or from the copy kept before the first edit. */
  def?: Value;
};

export type Doc = {
  format: "toml" | "properties" | "cfg" | "json";
  settings: Setting[];
  /** Returns the file text with one setting changed. */
  set: (text: string, s: Setting, v: Value) => string;
};

const NUM = /^[-+]?(\d+\.?\d*|\.\d+)([eE][-+]?\d+)?$/;
const eolOf = (t: string) => (t.includes("\r\n") ? "\r\n" : "\n");

export function formatOf(path: string): Doc["format"] | null {
  const ext = path.split(".").pop()!.toLowerCase();
  if (ext === "toml") return "toml";
  if (ext === "json") return "json";
  if (ext === "properties") return "properties";
  if (ext === "cfg") return "cfg";
  return null;
}

/** Parse; null when the file can't be shown as settings (the text editor is used instead). */
export function parse(path: string, text: string, defaults: string | null): Doc | null {
  const format = formatOf(path);
  if (!format) return null;
  try {
    const doc = format === "json" ? parseJson(text) : parseLines(format, text);
    if (!doc || !doc.settings.length) return null;
    if (defaults) {
      const d = format === "json" ? parseJson(defaults) : parseLines(format, defaults);
      const byId = new Map(d?.settings.map((s) => [s.id, s.value]));
      for (const s of doc.settings) if (s.def === undefined && byId.has(s.id)) s.def = byId.get(s.id);
    }
    return doc;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------- line formats

function scalar(raw: string): { kind: Kind; value: Value } {
  const v = raw.trim();
  if (v === "true" || v === "false") return { kind: "bool", value: v === "true" };
  if (NUM.test(v)) return { kind: /[.eE]/.test(v) ? "float" : "int", value: Number(v) };
  if (/^"(?:[^"\\]|\\.)*"$/.test(v)) return { kind: "string", value: JSON.parse(v) };
  if (/^'[^']*'$/.test(v)) return { kind: "string", value: v.slice(1, -1) };
  if (/^\[.*\]$/.test(v)) {
    try {
      const arr = JSON.parse(v.replace(/'([^']*)'/g, '"$1"'));
      if (Array.isArray(arr) && arr.every((x) => typeof x !== "object")) return { kind: "list", value: arr };
    } catch { /* fall through */ }
  }
  return { kind: "raw", value: v };
}

function rules(s: Setting, comments: string[]) {
  const help: string[] = [];
  for (const c of comments) {
    let m;
    if ((m = /^Range:\s*([-\d.eE+]+)\s*~\s*([-\d.eE+]+)/i.exec(c))) { s.min = Number(m[1]); s.max = Number(m[2]); }
    else if ((m = /^Range:\s*>\s*([-\d.eE+]+)/i.exec(c))) s.min = Number(m[1]);
    else if ((m = /^Range:\s*<\s*([-\d.eE+]+)/i.exec(c))) s.max = Number(m[1]);
    else if ((m = /^\[?range:\s*([-\d.eE+]+)\s*~\s*([-\d.eE+]+)/i.exec(c))) { s.min = Number(m[1]); s.max = Number(m[2]); }
    else if ((m = /^Allowed Values:\s*(.+)$/i.exec(c))) s.options = m[1].split(",").map((x) => x.trim()).filter(Boolean);
    else if ((m = /^\[?default:\s*(.+?)\]?$/i.exec(c))) s.def = scalar(m[1]).value;
    else if (c.trim()) help.push(c);
  }
  // "> -2147483648" style ranges just mean "any number".
  if (s.min !== undefined && Math.abs(s.min) >= 2147483647) s.min = undefined;
  if (s.max !== undefined && Math.abs(s.max) >= 2147483647) s.max = undefined;
  if ((s.kind === "int") && [s.min, s.max].some((n) => n !== undefined && !Number.isInteger(n))) s.kind = "float";
  if (s.options && s.kind === "string") s.kind = "enum";
  s.help = help.join("\n").trim();
}

type LineSetting = Setting & { line: number; quote: string };

function parseLines(format: "toml" | "properties" | "cfg", text: string): Doc {
  const eol = eolOf(text);
  const settings: LineSetting[] = [];
  let path: string[] = [];
  let comments: string[] = [];
  text.split(eol).forEach((l, i) => {
    const t = l.trim();
    if (t.startsWith("#") || t.startsWith("!") || (format === "cfg" && t.startsWith("//"))) {
      comments.push(t.replace(/^(#+|!|\/\/)\s?/, ""));
      return;
    }
    if (!t) { if (format !== "toml") comments = []; return; }
    if (format === "toml") {
      const sec = /^\[+\s*(.+?)\s*\]+$/.exec(t);
      if (sec) { path = sec[1].split(".").map((p) => p.replace(/^"|"$/g, "")); comments = []; return; }
    }
    if (format === "cfg") {
      const open = /^"?([^"{]+?)"?\s*\{$/.exec(t);
      if (open) { path = [...path, open[1].trim()]; comments = []; return; }
      if (t === "}") { path = path.slice(0, -1); comments = []; return; }
    }
    let key = "", raw = "", kindHint = "";
    if (format === "toml") {
      const kv = /^("[^"]*"|[\w.-]+)\s*=\s*(.+?)\s*$/.exec(t);
      if (!kv) { comments = []; return; }
      key = kv[1].replace(/"/g, ""); raw = kv[2];
    } else if (format === "cfg") {
      const kv = /^([BIDS]):"?([^"=]+?)"?\s*=\s*(.*)$/.exec(t);
      if (!kv) { comments = []; return; }
      kindHint = kv[1]; key = kv[2]; raw = kv[3];
    } else {
      const kv = /^([^=:\s]+)\s*[=:]\s*(.*)$/.exec(t);
      if (!kv) { comments = []; return; }
      key = kv[1]; raw = kv[2];
    }
    let { kind, value } = scalar(raw);
    // .properties/.cfg strings aren't quoted.
    if (format !== "toml" && kind === "raw") { kind = "string"; value = raw; }
    if (kindHint === "S") { kind = "string"; value = raw; }
    if (kindHint === "D" && kind === "int") kind = "float";
    const s: LineSetting = { id: [...path, key].join("."), path: [...path], key, kind, value, help: "", line: i, quote: raw.trim()[0] };
    rules(s, comments);
    settings.push(s);
    comments = [];
  });
  return {
    format,
    settings,
    set(text, s, v) {
      const ls = s as LineSetting;
      const eol = eolOf(text);
      const lines = text.split(eol);
      const l = lines[ls.line];
      const at = format === "properties" ? l.search(/[=:]/) : l.indexOf("=");
      lines[ls.line] = `${l.slice(0, at + 1)}${format === "toml" ? " " : ""}${write(format, ls, v)}`;
      return lines.join(eol);
    },
  };
}

function write(format: string, s: LineSetting, v: Value): string {
  if (format !== "toml") return Array.isArray(v) ? v.join(",") : String(v);
  if (typeof v === "string" && (s.kind === "string" || s.kind === "enum")) {
    return s.quote === "'" ? `'${v.replace(/'/g, "")}'` : JSON.stringify(v);
  }
  if (Array.isArray(v)) return `[${v.map((x) => (typeof x === "string" ? JSON.stringify(x) : String(x))).join(", ")}]`;
  if (typeof v === "number" && s.kind === "float" && Number.isInteger(v)) return v.toFixed(1);
  return String(v);
}

// ---------------------------------------------------------------- JSON

function parseJson(text: string): Doc | null {
  const root = JSON.parse(text);
  if (!root || typeof root !== "object" || Array.isArray(root)) return null;
  const settings: Setting[] = [];
  const walk = (obj: Record<string, Value>, path: string[]) => {
    for (const [key, value] of Object.entries(obj)) {
      const id = [...path, key].join("\u0000");
      if (value && typeof value === "object" && !Array.isArray(value) && Object.keys(value).length) {
        walk(value as Record<string, Value>, [...path, key]);
        continue;
      }
      let kind: Kind = "raw";
      if (typeof value === "boolean") kind = "bool";
      else if (typeof value === "number") kind = Number.isInteger(value) ? "int" : "float";
      else if (typeof value === "string") kind = "string";
      else if (Array.isArray(value) && value.every((x) => x === null || typeof x !== "object")) kind = "list";
      settings.push({ id, path, key, kind, value, help: "" });
    }
  };
  walk(root, []);
  const indent = /\n(\s+)"/.exec(text)?.[1] ?? "  ";
  return {
    format: "json",
    settings,
    set(text, s, v) {
      const obj = JSON.parse(text);
      let o = obj;
      for (const p of s.path) o = o[p];
      o[s.key] = v;
      return JSON.stringify(obj, null, indent.includes("\t") ? "\t" : indent.length) + (text.endsWith("\n") ? "\n" : "");
    },
  };
}

/** Is `v` acceptable for this setting? Returns an error message or "". */
export function check(s: Setting, v: Value): string {
  if (s.kind === "int" || s.kind === "float") {
    if (typeof v !== "number" || Number.isNaN(v)) return "Not a number";
    if (s.kind === "int" && !Number.isInteger(v)) return "Must be a whole number";
    if (s.min !== undefined && v < s.min) return `Must be at least ${s.min}`;
    if (s.max !== undefined && v > s.max) return `Must be at most ${s.max}`;
  }
  if (s.kind === "enum" && s.options && !s.options.includes(String(v))) return "Not an allowed value";
  return "";
}

export const same = (a: Value | undefined, b: Value | undefined) => JSON.stringify(a) === JSON.stringify(b);
export const label = (k: string) => k.replace(/[_-]/g, " ").replace(/([a-z])([A-Z])/g, "$1 $2").replace(/\s+/g, " ").trim().replace(/^\w/, (c) => c.toUpperCase());
