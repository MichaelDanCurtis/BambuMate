// Renders this app's CSS through the real WebKit engine and reports anything
// WebKit refuses to parse.
//
// Why this exists: on macOS a Tauri window is a WKWebView, so the CSS is
// parsed by WebKit, not Chromium. WebKit silently discards declarations it
// cannot parse, and discards an *entire* comma-separated selector group if any
// one selector in it is unparseable. Both failures are invisible on Windows,
// where the same markup renders through Chromium/WebView2 — which is exactly
// how a rule that blanks out a whole screen can ship unnoticed.
//
// Scope: this validates against the WebKit build Playwright ships, which
// tracks current Safari. It therefore catches "this is broken in WebKit today"
// but cannot reproduce an older Safari's missing features.

import { chromium, webkit } from "playwright";
import { readFileSync, writeFileSync } from "node:fs";
import { readdir } from "node:fs/promises";
import { join, relative, resolve } from "node:path";

const repoRoot = resolve(process.argv[2] ?? "../..");

async function cssFiles(dir, acc = []) {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    if (entry.name === "node_modules" || entry.name === "target" || entry.name === "dist") {
      continue;
    }
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      await cssFiles(full, acc);
    } else if (entry.name.endsWith(".css")) {
      acc.push(full);
    }
  }
  return acc;
}

const stripComments = (css) => css.replace(/\/\*[\s\S]*?\*\//g, "");

/**
 * Pull the selectors and declarations out of a stylesheet.
 *
 * Deliberately a small scanner rather than a real CSS parser: it only needs to
 * recover the author's *intent* so we can ask WebKit whether each piece
 * survives.
 *
 * Two things are deliberately *not* reported, because in both cases the
 * author already said "I know WebKit may not take this":
 *   - anything guarded by `@supports`, since being unsupported is the entire
 *     point of such a block;
 *   - a declaration whose block also carries the `-webkit-` prefixed spelling
 *     of the same property and value. That pair is the correct progressive
 *     enhancement idiom: WebKit consumes the prefixed one and drops the
 *     standard one, every other engine does the reverse, and both end up
 *     styled. Flagging it would be flagging the fix itself.
 */
function extractRules(rawCss) {
  const css = stripComments(rawCss);
  const selectors = new Set();
  const declarations = new Map(); // "prop: value" -> {prop, value, context}
  const stack = [];
  const blocks = []; // declarations of each currently-open block
  let buf = "";

  const inKeyframes = () => stack.some((h) => /^@(-\w+-)?keyframes/i.test(h));
  const inSupports = () => stack.some((h) => /^@supports/i.test(h));

  const flushDeclaration = () => {
    const text = buf.trim();
    buf = "";
    if (!text || stack.length === 0 || inSupports()) return;
    const idx = text.indexOf(":");
    if (idx <= 0) return;
    const prop = text.slice(0, idx).trim().toLowerCase();
    let value = text.slice(idx + 1).trim();
    if (!prop || !value) return;
    // Custom properties accept literally any value, so testing them is noise.
    if (prop.startsWith("--")) return;
    if (!/^[a-zA-Z-]+$/.test(prop)) return;
    value = value.replace(/!\s*important$/i, "").trim();
    if (!value || value.includes("var(")) return; // var() defers validation
    blocks[blocks.length - 1].push({ prop, value });
  };

  const closeBlock = () => {
    const decls = blocks.pop() ?? [];
    const context = stack[stack.length - 1] ?? "";
    const prefixed = new Set(
      decls
        .filter((d) => d.prop.startsWith("-webkit-"))
        .map((d) => `${d.prop.slice("-webkit-".length)}: ${d.value}`)
    );
    for (const d of decls) {
      const key = `${d.prop}: ${d.value}`;
      if (prefixed.has(key)) continue;
      if (!declarations.has(key)) declarations.set(key, { ...d, context });
    }
  };

  for (const ch of css) {
    if (ch === "{") {
      const head = buf.trim();
      buf = "";
      if (head && !head.startsWith("@") && !inKeyframes()) {
        selectors.add(head);
      }
      stack.push(head);
      blocks.push([]);
    } else if (ch === "}") {
      flushDeclaration();
      closeBlock();
      stack.pop();
    } else if (ch === ";") {
      flushDeclaration();
    } else {
      buf += ch;
    }
  }

  return { selectors: [...selectors], declarations: [...declarations.values()] };
}

// Runs inside the browser. Returns whatever the engine rejects.
function probe({ selectors, declarations }) {
  const badSelectors = [];
  for (const sel of selectors) {
    try {
      document.querySelector(sel);
    } catch {
      badSelectors.push(sel);
    }
  }
  const badDeclarations = [];
  for (const { prop, value } of declarations) {
    let ok = false;
    try {
      ok = CSS.supports(prop, value);
    } catch {
      ok = false;
    }
    if (!ok) badDeclarations.push(`${prop}: ${value}`);
  }
  return { badSelectors, badDeclarations };
}

async function analyse(browserType, rulesByFile) {
  const browser = await browserType.launch();
  const page = await browser.newPage();
  await page.setContent("<!doctype html><html><body></body></html>");
  const results = {};
  for (const [name, rules] of rulesByFile) {
    results[name] = await page.evaluate(probe, rules);
  }
  await browser.close();
  return results;
}

const files = (await cssFiles(repoRoot)).sort();
if (files.length === 0) {
  console.error("No CSS files found under", repoRoot);
  process.exit(1);
}

// Parse once and hand the same extraction to both engines, so any difference in
// the results is a difference between the engines and nothing else.
const rulesByFile = new Map(
  files.map((f) => [
    relative(repoRoot, f).split("\\").join("/"),
    extractRules(readFileSync(f, "utf8")),
  ])
);

console.log(`Scanning ${files.length} stylesheets\n`);

const wk = await analyse(webkit, rulesByFile);
// Chromium is the control: it is what the app renders in on Windows, so a
// problem that shows up in both is a plain bug, while a WebKit-only problem is
// the macOS-specific breakage this whole harness exists to catch.
const cr = await analyse(chromium, rulesByFile);

// Point a finding back at the rule it came from, so the log says where to look.
const where = (file, key) => {
  const hit = rulesByFile.get(file).declarations.find((d) => `${d.prop}: ${d.value}` === key);
  return hit?.context ? `${key}   (in ${hit.context})` : key;
};

let webkitOnly = 0;
let both = 0;
const report = {};

for (const file of Object.keys(wk)) {
  const w = wk[file];
  const c = cr[file];
  const entry = {
    webkitOnlySelectors: w.badSelectors.filter((s) => !c.badSelectors.includes(s)),
    webkitOnlyDeclarations: w.badDeclarations.filter((d) => !c.badDeclarations.includes(d)),
    bothSelectors: w.badSelectors.filter((s) => c.badSelectors.includes(s)),
    bothDeclarations: w.badDeclarations.filter((d) => c.badDeclarations.includes(d)),
  };
  const n =
    entry.webkitOnlySelectors.length +
    entry.webkitOnlyDeclarations.length +
    entry.bothSelectors.length +
    entry.bothDeclarations.length;
  if (n === 0) continue;

  report[file] = entry;
  webkitOnly += entry.webkitOnlySelectors.length + entry.webkitOnlyDeclarations.length;
  both += entry.bothSelectors.length + entry.bothDeclarations.length;

  console.log(file);
  for (const s of entry.webkitOnlySelectors) console.log(`  WEBKIT-ONLY selector    ${s}`);
  for (const d of entry.webkitOnlyDeclarations) {
    console.log(`  WEBKIT-ONLY declaration ${where(file, d)}`);
  }
  for (const s of entry.bothSelectors) console.log(`  both engines  selector    ${s}`);
  for (const d of entry.bothDeclarations) {
    console.log(`  both engines  declaration ${where(file, d)}`);
  }
  console.log("");
}

writeFileSync(
  "webkit-css-report.json",
  JSON.stringify({ webkitOnly, both, files: report }, null, 2)
);

console.log(`WebKit-only problems: ${webkitOnly}`);
console.log(`Problems in both engines: ${both}`);

// Only WebKit-only findings fail the run. Something both engines reject is
// either dead CSS or a deliberate progressive-enhancement fallback, neither of
// which is a macOS regression.
if (webkitOnly > 0) {
  console.error("\nFAIL: CSS that WebKit rejects but Chromium accepts will break only on macOS.");
  process.exit(1);
}
console.log("\nPASS: no WebKit-only CSS problems.");
