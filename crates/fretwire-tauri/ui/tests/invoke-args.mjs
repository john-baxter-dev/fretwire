// Argument names must survive the real IPC.
//
// Tauri renames a command's Rust parameters to camelCase, so `user_defaults: bool` is asked for as
// `userDefaults` and an invoke that sends the snake_case name is rejected outright — "command
// backup_device missing required key userDefaults". Neither of the other two backends is that
// strict: the serve dispatcher looks a name up camelCase-first with a snake_case fallback, and the
// mock takes whatever it destructures. So a wrong name passes `npm test`, passes `npm run dev`,
// passes serve mode, and fails only in the shipped app — which is how the backup dialog reached a
// tester broken (issue #5, 2026-09-05).
//
// This reads the command signatures out of the Rust and checks both other backends against them.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const ui = join(here, "..");
const crate = join(ui, "..");

let failures = 0;
// One line per mismatch, not per check: this walks every argument of every call site, and a clean
// run has nothing to say beyond the two counts at the end.
function ok(cond, msg) {
  if (!cond) {
    console.log(`FAIL ${msg}`);
    failures++;
  }
}
const note = (msg) => console.log(`ok   ${msg}`);

const camel = (s) => s.replace(/_([a-z])/g, (_, c) => c.toUpperCase());

/// Every `#[tauri::command]` in commands.rs → the argument names its IPC accepts (camelCase).
/// Tauri's own injected parameters (the app handle, the managed state, the window) never cross the
/// wire, so they are not argument names.
function tauriCommands() {
  const src = readFileSync(join(crate, "src/commands.rs"), "utf8");
  const out = new Map();
  const re = /#\[tauri::command\][\s\S]*?pub (?:async )?fn (\w+)\s*\(([\s\S]*?)\)\s*->/g;
  for (const [, name, params] of src.matchAll(re)) {
    const args = [];
    // One `name: Type` per top-level comma; a type may itself hold commas (Vec<i64>, Option<T>).
    let depth = 0;
    let cur = "";
    for (const ch of params) {
      if ("<([".includes(ch)) depth++;
      else if (">)]".includes(ch)) depth--;
      if (ch === "," && depth === 0) {
        args.push(cur);
        cur = "";
      } else cur += ch;
    }
    args.push(cur);
    const names = args
      .map((a) => a.trim().split(":")[0].trim())
      .filter((a) => a && !["app", "state", "window", "sink"].includes(a))
      .map(camel);
    out.set(name, new Set(names));
  }
  return out;
}

/// Strip strings, template literals and comments so a brace scan can trust what it sees.
function blank(src) {
  let out = "";
  let i = 0;
  const quotes = "\"'`";
  while (i < src.length) {
    const c = src[i];
    // Every branch keeps the length, so an offset into the blanked copy indexes the original.
    if (c === "/" && src[i + 1] === "/") {
      while (i < src.length && src[i] !== "\n") {
        out += " ";
        i++;
      }
      continue;
    }
    if (c === "/" && src[i + 1] === "*") {
      while (i < src.length && !(src[i] === "*" && src[i + 1] === "/")) {
        out += src[i] === "\n" ? "\n" : " ";
        i++;
      }
      out += "  ";
      i += 2;
      continue;
    }
    if (quotes.includes(c)) {
      const q = c;
      out += q;
      i++;
      while (i < src.length && src[i] !== q) {
        if (src[i] === "\\") {
          out += "  ";
          i += 2;
          continue;
        }
        out += src[i] === "\n" ? "\n" : " ";
        i++;
      }
      out += q;
      i++;
      continue;
    }
    out += c;
    i++;
  }
  return out;
}

/// The top-level keys and `...spreads` of the object literal starting at `open`.
function objectKeys(src, open) {
  let depth = 0;
  let end = open;
  for (let i = open; i < src.length; i++) {
    if ("{([".includes(src[i])) depth++;
    else if ("})]".includes(src[i])) {
      depth--;
      if (depth === 0) {
        end = i;
        break;
      }
    }
  }
  const body = src.slice(open + 1, end);
  const keys = [];
  const spreads = [];
  let d = 0;
  let cur = "";
  for (const ch of body) {
    if ("{([".includes(ch)) d++;
    else if ("})]".includes(ch)) d--;
    if (ch === "," && d === 0) {
      keys.push(cur);
      cur = "";
    } else cur += ch;
  }
  keys.push(cur);
  const names = [];
  for (const k of keys) {
    const t = k.trim();
    if (!t) continue;
    if (t.startsWith("...")) {
      spreads.push(t.slice(3).trim());
      continue;
    }
    const m = t.match(/^([A-Za-z_$][\w$]*)\s*[:=]?/);
    if (m) names.push(m[1]);
  }
  return { names, spreads, end };
}

const commands = tauriCommands();
ok(commands.size > 50, "commands.rs parsed for command signatures");
note(`read ${commands.size} command signatures out of commands.rs`);

function sources(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    if (statSync(p).isDirectory()) {
      if (entry !== "mock" && entry !== "node_modules") out.push(...sources(p));
    } else if (/\.(svelte|js)$/.test(entry) && !["ipc.js", "serve.js"].includes(entry)) {
      out.push(p);
    }
  }
  return out;
}

// ---- the UI's invoke call sites ----
let checked = 0;
for (const file of sources(join(ui, "src"))) {
  const raw = readFileSync(file, "utf8");
  const src = blank(raw);
  const re = /invoke\(/g;
  for (const m of src.matchAll(re)) {
    const argsStart = m.index + m[0].length;
    // The command name ends at this call's own first top-level comma — not at the next call's, so
    // walk the parens rather than trusting a window. `invoke("undo")` takes no arguments at all.
    let depth = 1;
    let comma = -1;
    for (let i = argsStart; i < src.length && depth > 0; i++) {
      if ("{([".includes(src[i])) depth++;
      else if ("})]".includes(src[i])) depth--;
      else if (src[i] === "," && depth === 1) {
        comma = i;
        break;
      }
    }
    if (comma < 0) continue;
    // A ternary names two commands; both take the same argument object.
    const names = [...raw.slice(argsStart, comma).matchAll(/"([a-z_0-9]+)"/g)].map((x) => x[1]);
    const rest = src.slice(comma + 1);
    const brace = rest.search(/\S/);
    if (brace < 0 || rest[brace] !== "{") continue; // args passed as a variable — nothing to read
    const { names: keys, spreads } = objectKeys(src, comma + 1 + brace);
    // A spread of a plain `const x = { … }` in the same file is resolvable, and this is where the
    // backup dialog's bug lived; anything else is reported so it can't hide.
    const all = [...keys];
    for (const s of spreads) {
      // The nearest declaration *above* the call, since a file may hold several `const target`.
      const decls = [...src.matchAll(new RegExp(`const ${s}\\s*=\\s*\\{`, "g"))].map(
        (d) => d.index,
      );
      const decl = decls.filter((d) => d < m.index).pop() ?? decls[0];
      if (decl === undefined) {
        ok(false, `${file}: cannot resolve spread \`...${s}\` — check its keys by hand`);
        continue;
      }
      all.push(...objectKeys(src, src.indexOf("{", decl)).names);
    }
    for (const cmd of names) {
      const want = commands.get(cmd);
      if (!want) continue; // serve/mock-only command, or a name built at runtime
      checked++;
      for (const k of all) {
        ok(want.has(k), `invoke("${cmd}", { ${k} }) — ${file.slice(ui.length + 1)}`);
      }
    }
  }
}
ok(checked > 30, `only ${checked} invoke call sites found — the scanner has stopped seeing them`);
note(`checked ${checked} invoke call sites against the Rust signatures`);

// ---- the mock backend's handlers ----
const mock = blank(readFileSync(join(ui, "src/mock/backend.js"), "utf8"));
let mocked = 0;
for (const m of mock.matchAll(/^ {2}(\w+):\s*(?:async\s*)?\(\s*\{/gm)) {
  const cmd = m[1];
  const want = commands.get(cmd);
  if (!want) continue;
  const { names } = objectKeys(mock, m.index + m[0].length - 1);
  mocked++;
  for (const k of names) {
    ok(want.has(k), `mock ${cmd}({ ${k} }) is a name the real IPC accepts`);
  }
}
ok(mocked > 20, `only ${mocked} mock handlers found — the scanner has stopped seeing them`);
note(`checked ${mocked} mock handlers against the Rust signatures`);

console.log(failures ? `\n${failures} failed` : "\nall good");
process.exit(failures ? 1 : 0);
