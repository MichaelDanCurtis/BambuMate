// Screenshots every route in both themes at two widths, for visual review.
//
// This is not a test: it asserts nothing and always exits 0 unless the app
// fails to boot. It exists so a design change can be reviewed page by page in
// the real macOS web engine, with the same fixtures the flow tests use.
//
//   trunk build && cd tests/webkit && node screens.mjs ../..
//
// writes tests/webkit/screens/<theme>-<width>-<route>.png (git-ignored), plus
// two states that are not a route: the Analyze results (`analysis-results`)
// and the setup wizard (`wizard`, and `wizard-step-2` for its second step).

import { webkit } from "playwright";
import { createServer } from "node:http";
import { readFile, mkdir } from "node:fs/promises";
import { existsSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import { FIXTURES } from "./fixtures.mjs";

const repoRoot = resolve(process.argv[2] ?? "../..");
const distDir = join(repoRoot, "dist");
const outDir = join(repoRoot, "tests/webkit/screens");

if (!existsSync(join(distDir, "index.html"))) {
  console.error(`No build found at ${distDir}. Run \`trunk build\` first.`);
  process.exit(1);
}

const THEMES = ["dark", "bambu"];
const WIDTHS = [1280, 900];
const HEIGHT = 860; // the minimum; taller pages grow the viewport, see below
const ROUTES = [
  "/",
  "/printer",
  "/filament",
  "/filament/batch",
  "/profiles",
  "/profiles/compare",
  "/slice",
  "/analysis",
  "/settings",
  "/settings/health",
  "/settings/about",
];
// A tab route is reached through its hub: the hub's rail link, then the tab.
const HUB_OF = {
  "/filament/batch": "/filament",
  "/profiles/compare": "/profiles",
  "/settings/health": "/settings",
  "/settings/about": "/settings",
};

// Copied from serve() in app-flows.mjs, which runs its flows at import time
// and so cannot export it. Keep the two in step.
const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".wasm": "application/wasm",
  ".json": "application/json",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".ico": "image/x-icon",
  ".woff2": "font/woff2",
  ".ttf": "font/ttf",
};

function serve() {
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

// A trimmed copy of installTauriMock() in app-flows.mjs. The one difference:
// get_preference answers the saved theme for the `theme` key only. Every other
// key keeps the fixture's answer, so no other preference is accidentally set
// to "dark" or "bambu".
function installTauriMock({ fixtures, theme }) {
  const invoke = async (cmd, args) => {
    if (cmd === "get_preference" && args?.key === "theme") return theme;
    if (!(cmd in fixtures)) throw new Error(`no fixture for command '${cmd}'`);
    return structuredClone(fixtures[cmd]);
  };
  window.__TAURI__ = { core: { invoke }, event: { listen: async () => () => {} } };
  window.__TAURI_INTERNALS__ = { invoke };
}

function fileName(theme, width, route) {
  const name = route === "/" ? "home" : route.slice(1).replaceAll("/", "-");
  return `${theme}-${width}-${name}.png`;
}

// Takes the whole page. The app shell is 100vh and scrolls inside `.content`,
// so fullPage alone stops at the fold: grow the viewport to the content's
// height first.
async function capture(page, theme, width, file) {
  const tall = await page.evaluate(() => {
    const c = document.querySelector(".content");
    return c ? c.scrollHeight + c.getBoundingClientRect().top : 0;
  });
  if (tall > HEIGHT) {
    await page.setViewportSize({ width, height: Math.ceil(tall) });
    await page.waitForTimeout(200);
  }
  await page.screenshot({ path: join(outDir, file), fullPage: true });
  console.log(`  ${file}`);
  written++;
}

// A 64x48 PNG: the photo the Analyze flow is given.
const PHOTO = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAEAAAAAwCAIAAAAuKetIAAAAQklEQVR4nO3PQQ0AIBDAsAP/nkHGPstbqyCbr9/fK9F1gH0dSqTEkdQA9JOWuNuO2aF3wpKkOEgAAAAAAAAAAAAA8MkBn6IBR3qGPVgAAAAASUVORK5CYII=",
  "base64",
);

// The Analyze results: a photo, a profile and the (fixture) analysis.
async function analysisResults(browser, theme, width) {
  const page = await browser.newPage({ viewport: { width, height: HEIGHT } });
  page.on("pageerror", (e) => console.log(`  pageerror on analysis results: ${e.message}`));
  try {
    await page.addInitScript(installTauriMock, { fixtures: FIXTURES, theme });
    await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
    await page.waitForSelector(".sidebar", { timeout: 45000 });
    await page.click('a.nav-link[href="/analysis"]');
    await page.waitForSelector(".drop-zone");
    await page.setInputFiles("#photo-file-input", { name: "p.png", mimeType: "image/png", buffer: PHOTO });
    await page.waitForSelector(".profile-select option:nth-child(2)", { state: "attached", timeout: 20000 });
    await page.selectOption(".profile-select", { index: 1 });
    await page.click("text=Analyze Print");
    await page.waitForSelector(".analysis-results", { timeout: 30000 });
    await page.mouse.move(width - 20, HEIGHT - 20);
    await page.waitForTimeout(500);
    await capture(page, theme, width, `${theme}-${width}-analysis-results.png`);
  } finally {
    await page.close();
  }
}

// The setup wizard: a first run, with nothing configured. Its first two steps.
async function wizard(browser, theme, width) {
  const page = await browser.newPage({ viewport: { width, height: HEIGHT } });
  page.on("pageerror", (e) => console.log(`  pageerror on wizard: ${e.message}`));
  try {
    const fixtures = {
      ...FIXTURES,
      check_setup_complete: { bambu_studio_path: null, ai_provider: null, has_api_key: false, setup_complete: false },
    };
    await page.addInitScript(installTauriMock, { fixtures, theme });
    await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
    await page.waitForSelector(".wizard-overlay", { timeout: 45000 });
    await page.waitForTimeout(800);
    await capture(page, theme, width, `${theme}-${width}-wizard.png`);
    await page.click(".wizard-footer .btn-primary");
    await page.waitForTimeout(300);
    await capture(page, theme, width, `${theme}-${width}-wizard-step-2.png`);
  } finally {
    await page.close();
  }
}

const server = await serve();
const baseUrl = `http://127.0.0.1:${server.address().port}/`;
await mkdir(outDir, { recursive: true });

const browser = await webkit.launch();
let written = 0;
try {
  for (const theme of THEMES) {
    for (const width of WIDTHS) {
      for (const route of ROUTES) {
        const page = await browser.newPage({ viewport: { width, height: HEIGHT } });
        page.on("pageerror", (e) => console.log(`  pageerror on ${route}: ${e.message}`));
        await page.addInitScript(installTauriMock, { fixtures: FIXTURES, theme });
        await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
        await page.waitForSelector(".sidebar", { timeout: 45000 });
        const hub = HUB_OF[route];
        await page.click(`a.nav-link[href="${hub ?? route}"]`);
        if (hub) await page.click(`.nd-tabs a[href="${route}"]`);
        // Leave the rail at rest: move the pointer off it and drop focus, so the
        // screenshot shows the collapsed 64px rail and not its hover state.
        await page.mouse.move(width - 20, HEIGHT - 20);
        await page.evaluate(() => document.activeElement?.blur?.());
        await page.waitForTimeout(600);
        const applied = await page.evaluate(() => document.documentElement.dataset.theme);
        if (applied !== theme) console.log(`  WARNING ${route}: data-theme is '${applied}', expected '${theme}'`);
        await capture(page, theme, width, fileName(theme, width, route));
        await page.close();
      }
      await analysisResults(browser, theme, width);
      await wizard(browser, theme, width);
    }
  }
} finally {
  await browser.close();
  server.close();
}
console.log(`\nWrote ${written} screenshots to ${outDir}`);
