// Measures the app's full-screen overlays in the real WebKit engine.
//
// The setup wizard, the delete-confirmation modal and the change preview are
// all `position: fixed` boxes that must cover the viewport. They originally
// did that with the `inset: 0` shorthand. WebKit older than Safari 14.1 drops
// that declaration, which collapses the overlay to a zero-size box: the wizard
// becomes invisible while still swallowing clicks, so the app looks like it
// failed to start. Chromium accepts `inset`, so Windows never showed it.
//
// This asserts the geometry rather than the syntax — it stays true whichever
// way the CSS is written, so it cannot rot into a tautology.

import { chromium, webkit } from "playwright";
import { readFileSync, existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { extname, join, resolve } from "node:path";
import { FIXTURES } from "./fixtures.mjs";

const repoRoot = resolve(process.argv[2] ?? "../..");

const STYLESHEETS = [
  "style/main.css",
  "src/pages/profile_management.css",
  "src/components/change_preview.css",
];

// Every one of these must cover the viewport, or the UI it contains is
// unreachable.
const FULLSCREEN_OVERLAYS = [".wizard-overlay", ".modal-overlay", ".change-preview-overlay"];

const VIEWPORT = { width: 1280, height: 800 };

const css = STYLESHEETS.map((f) => readFileSync(resolve(repoRoot, f), "utf8")).join("\n");

function measure({ selectors }) {
  const out = {};
  for (const sel of selectors) {
    const el = document.createElement("div");
    // Selectors here are single class names.
    el.className = sel.slice(1);
    el.textContent = "x";
    document.body.appendChild(el);
    const rect = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    out[sel] = {
      width: Math.round(rect.width),
      height: Math.round(rect.height),
      position: cs.position,
      display: cs.display,
      visibility: cs.visibility,
      opacity: cs.opacity,
    };
    el.remove();
  }
  return out;
}

async function run(browserType, name) {
  const browser = await browserType.launch();
  const page = await browser.newPage({ viewport: VIEWPORT });
  await page.setContent(
    `<!doctype html><html><head><style>
       html,body{margin:0;padding:0;width:100%;height:100%}
       ${css}
     </style></head><body></body></html>`
  );
  const measured = await page.evaluate(measure, { selectors: FULLSCREEN_OVERLAYS });
  await page.screenshot({ path: `overlay-${name}.png` });
  await browser.close();
  return measured;
}

const results = {
  webkit: await run(webkit, "webkit"),
  chromium: await run(chromium, "chromium"),
};

let failures = 0;
for (const engine of ["webkit", "chromium"]) {
  console.log(`\n== ${engine} ==`);
  for (const sel of FULLSCREEN_OVERLAYS) {
    const m = results[engine][sel];
    // A few px of slack: a scrollbar or subpixel rounding must not fail this.
    const coversWidth = m.width >= VIEWPORT.width - 2;
    const coversHeight = m.height >= VIEWPORT.height - 2;
    const visible = m.visibility !== "hidden" && m.display !== "none";
    const ok = coversWidth && coversHeight && visible;
    console.log(
      `  ${ok ? "OK  " : "FAIL"} ${sel} ${m.width}x${m.height} ` +
        `position=${m.position} display=${m.display} visibility=${m.visibility}`
    );
    if (!ok) {
      failures++;
      console.error(
        `       expected to cover ${VIEWPORT.width}x${VIEWPORT.height}; ` +
          `a collapsed overlay hides its contents while still blocking clicks`
      );
    }
  }
}

if (failures > 0) {
  console.error(`\nFAIL: ${failures} overlay(s) do not cover the viewport.`);
  process.exit(1);
}
console.log("\nPASS: all full-screen overlays cover the viewport in both engines.");

// ============================================================================
// Rail z-index regression (final-fix Task 1).
//
// `.sidebar` (the 64px→220px hover rail) used to sit at z-index 45, below
// several page-level z-indexed elements (.search-container at 200 on
// /filament, an open SearchableSelect .ss-dropdown at 100 on /compare, …).
// `.content` never creates its own stacking context, so those elements
// stack directly against the rail in the root stacking context: below about
// 1076px width, where the page content and the expanded 220px rail
// physically overlap, the rail rendered *under* them instead of on top.
//
// This drives the real, built app (the same mock-server + Tauri-mock
// approach as app-flows.mjs) rather than a synthetic snippet, because the
// bug depends on real page content sitting at the real pixel coordinates the
// expanded rail covers — a fabricated snippet couldn't reproduce that
// geometry without just re-asserting the fix by construction.
//
// This test must fail on the pre-fix z-index (.sidebar at 45) and pass once
// the rail outranks page-level z-indexes (see style/main.css / style/agent.css
// for the full ladder).

const distDir = join(repoRoot, "dist");
if (!existsSync(join(distDir, "index.html"))) {
  console.error(`No build found at ${distDir}. Run \`trunk build\` first.`);
  process.exit(1);
}

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".wasm": "application/wasm",
  ".json": "application/json",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".ico": "image/x-icon",
};

// Serves the Trunk output, same as app-flows.mjs's `serve()`.
function serveDist() {
  const server = createServer(async (req, res) => {
    const path = decodeURIComponent(new URL(req.url, "http://x").pathname);
    let file = join(distDir, path);
    if (!existsSync(file) || path === "/") file = join(distDir, "index.html");
    try {
      const body = await readFile(file);
      res.writeHead(200, {
        "Content-Type": MIME[extname(file)] ?? "application/octet-stream",
        "Cache-Control": "no-store",
      });
      res.end(body);
    } catch (err) {
      res.writeHead(500).end(String(err));
    }
  });
  return new Promise((ok) => server.listen(0, "127.0.0.1", () => ok(server)));
}

// Same mock as app-flows.mjs's `installTauriMock`, trimmed to what this test
// needs (no event emission).
function installTauriMock(fixtures) {
  window.__ipc = { calls: [], unknown: [] };
  const invoke = async (cmd, args) => {
    window.__ipc.calls.push({ cmd, args });
    if (!(cmd in fixtures)) {
      window.__ipc.unknown.push(cmd);
      throw new Error(`no fixture for command '${cmd}'`);
    }
    return structuredClone(fixtures[cmd]);
  };
  window.__TAURI__ = { core: { invoke }, event: { listen: async () => () => {} } };
  window.__TAURI_INTERNALS__ = { invoke };
}

const RAIL_WIDTH_VIEWPORT = { width: 900, height: 800 };

// Each case: a route, how to surface a page-level z-indexed element near the
// rail (some need a click to open), and that element's selector.
const RAIL_OVERLAP_CASES = [
  {
    route: "/filament",
    reveal: async () => {},
    overlapSelector: ".search-container",
  },
  {
    route: "/compare",
    reveal: async (page) => {
      await page.locator(".ss-display").first().click();
    },
    overlapSelector: ".ss-dropdown",
  },
];

async function checkRailOverlap(browserType, name, baseUrl) {
  const failures = [];
  const browser = await browserType.launch();
  for (const { route, reveal, overlapSelector } of RAIL_OVERLAP_CASES) {
    const page = await browser.newPage({ viewport: RAIL_WIDTH_VIEWPORT });
    await page.addInitScript(installTauriMock, FIXTURES);
    try {
      await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
      await page.waitForSelector(".sidebar", { timeout: 45000 });
      await page.click(`a[href="${route}"]`);
      await reveal(page);
      await page.waitForSelector(overlapSelector, { timeout: 5000 });
      // The bug only shows once the rail is at its full 220px.
      // Navigation now dismisses hover until the pointer leaves the rail.
      // Start a fresh hover to test its expanded stacking, not the dismissed state.
      await page.mouse.move(850, 750);
      await page.hover("nav.sidebar");
      await page.waitForFunction(
        () => document.querySelector("nav.sidebar").getBoundingClientRect().width >= 219,
        null,
        { timeout: 3000 }
      );
      const result = await page.evaluate((sel) => {
        const el = document.querySelector(sel);
        const rect = el.getBoundingClientRect();
        // The middle of the overlapping element's own box, so this tracks
        // the actual overlap rather than a guessed fixed coordinate.
        const y = Math.round(rect.top + rect.height / 2);
        const points = [160, 200].map((x) => {
          const hit = document.elementFromPoint(x, y);
          return { x, inSidebar: !!hit?.closest("nav.sidebar"), hitClass: hit?.className ?? "(none)" };
        });
        return { y, points };
      }, overlapSelector);
      for (const p of result.points) {
        console.log(
          `  ${p.inSidebar ? "OK  " : "FAIL"} ${name} ${route} rail-overlap x=${p.x} y=${result.y} -> ${p.hitClass}`
        );
        if (!p.inSidebar) {
          failures.push(
            `${route}: elementFromPoint(${p.x}, ${result.y}) hit "${p.hitClass}", not nav.sidebar -- ` +
              `the expanded rail is covered by ${overlapSelector} at 900px width`
          );
        }
      }
    } finally {
      await page.close();
    }
  }
  await browser.close();
  return failures;
}

const server = await serveDist();
const baseUrl = `http://127.0.0.1:${server.address().port}/`;
console.log(`\nServing ${distDir} at ${baseUrl}`);

let railFailures;
try {
  railFailures = [
    ...(await checkRailOverlap(webkit, "webkit", baseUrl)),
    ...(await checkRailOverlap(chromium, "chromium", baseUrl)),
  ];
} finally {
  server.close();
}

if (railFailures.length) {
  console.error(`\nFAIL: ${railFailures.length} rail z-index overlap(s):\n` + railFailures.join("\n"));
  process.exit(1);
}
console.log("\nPASS: the expanded rail is never covered by page content in either engine.");
