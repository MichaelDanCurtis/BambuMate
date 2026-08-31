// Drives the real app through its primary flows in the real macOS web engine.
//
// The Rust suite proves the backend behaves; css-compat.mjs proves the
// stylesheets parse. Neither actually runs the UI, so neither can catch the
// failures that only appear once WebKit is executing the app: a wasm panic on a
// code path Chromium tolerates, an event that never fires, a data URL WebKit
// refuses to decode, a dialog that renders off-screen.
//
// The frontend reaches the backend through `window.__TAURI__.core.invoke`,
// which does not exist outside a Tauri host, so this installs a mock that
// answers with fixtures. That keeps the test about the *frontend*: everything
// above the IPC boundary is the real shipped code, including the wasm build.
//
// Chromium runs the identical script as a control. A step that fails in both is
// an ordinary bug; a step that fails only in WebKit is the macOS-specific class
// of breakage this harness exists to find.

import { chromium, webkit } from "playwright";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { deflateSync } from "node:zlib";
import { extname, join, resolve } from "node:path";
import { FIXTURES, GIF_1X1, makePng } from "./fixtures.mjs";

const repoRoot = resolve(process.argv[2] ?? "../..");
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

// Serves the Trunk output. Unknown paths fall back to index.html because the
// router uses real paths (/filament, /analysis) rather than hash fragments.
// `application/wasm` matters: WebKit's streaming compiler rejects anything else.
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

// Installed before any page script runs, so the wasm glue finds it already in
// place. Unknown commands reject rather than returning undefined: a silent
// undefined would deserialize into a confusing UI error far from its cause,
// while a rejection is the app's normal "backend said no" path and gets
// reported here by name.
function installTauriMock(fixtures) {
  const calls = [];
  const unknown = [];
  window.__ipc = { calls, unknown };
  const invoke = async (cmd, args) => {
    calls.push({ cmd, args });
    if (!(cmd in fixtures)) {
      unknown.push(cmd);
      throw new Error(`no fixture for command '${cmd}'`);
    }
    return structuredClone(fixtures[cmd]);
  };
  window.__TAURI__ = { core: { invoke } };
  window.__TAURI_INTERNALS__ = { invoke };
}

const png = makePng(320, 240, deflateSync);

class Run {
  constructor(engine) {
    this.engine = engine;
    this.steps = [];
    this.errors = [];
  }
  record(name, ok, detail = "") {
    this.steps.push({ name, ok, detail });
    console.log(`  ${ok ? "OK  " : "FAIL"} ${name}${detail ? `  ${detail}` : ""}`);
  }
  get failed() {
    return this.steps.filter((s) => !s.ok).map((s) => s.name);
  }
}

async function step(run, page, name, fn) {
  try {
    const detail = await fn();
    run.record(name, true, detail ?? "");
    return true;
  } catch (err) {
    const msg = String(err.message ?? err).split("\n")[0].slice(0, 200);
    run.record(name, false, msg);
    await page
      .screenshot({ path: `fail-${run.engine}-${run.steps.length}.png` })
      .catch(() => {});
    return false;
  }
}

async function driveApp(browserType, engine, baseUrl) {
  const run = new Run(engine);
  const browser = await browserType.launch();
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });

  // A wasm panic surfaces as an uncaught exception, not a failed assertion, so
  // this is the single most valuable signal here.
  page.on("pageerror", (e) => run.errors.push(`pageerror: ${e.message}`));
  page.on("console", (m) => {
    if (m.type() === "error") run.errors.push(`console.error: ${m.text()}`);
  });

  await page.addInitScript(installTauriMock, FIXTURES);

  console.log(`\n== ${engine} ==`);

  // -- boot ----------------------------------------------------------------
  await step(run, page, "app boots and renders the shell", async () => {
    await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
    await page.waitForSelector(".sidebar", { timeout: 45000 });
    const links = await page.locator(".nav-link").count();
    if (links < 8) throw new Error(`only ${links} nav links rendered`);
    return `${links} nav links`;
  });

  await step(run, page, "startup queries the backend", async () => {
    await page.waitForFunction(
      () => window.__ipc.calls.some((c) => c.cmd === "check_setup_complete"),
      { timeout: 15000 }
    );
    const cmds = await page.evaluate(() => [
      ...new Set(window.__ipc.calls.map((c) => c.cmd)),
    ]);
    return cmds.join(", ");
  });

  // -- filament search and selection ---------------------------------------
  await step(run, page, "navigate to Create Profile", async () => {
    await page.click('a[href="/filament"]');
    await page.waitForSelector(".filament-search-page", { timeout: 15000 });
  });

  await step(run, page, "catalog status renders", async () => {
    await page.waitForSelector(".catalog-status", { timeout: 15000 });
    return (await page.locator(".catalog-status").innerText()).replace(/\s+/g, " ").trim();
  });

  const typed = await step(run, page, "typing shows autocomplete suggestions", async () => {
    await page.fill(".search-input", "Polymaker PolyLite");
    await page.waitForSelector(".suggestions-dropdown .suggestion-item", { timeout: 15000 });
    const n = await page.locator(".suggestion-item").count();
    return `${n} suggestions`;
  });

  if (typed) {
    await step(run, page, "selecting a suggestion loads its specs", async () => {
      // The dropdown closes on blur, so the app commits the choice on
      // mousedown. Playwright's click sends mousedown first, matching a user.
      await page.locator(".suggestion-item").first().click();
      await page.waitForSelector(".filament-card", { timeout: 20000 });
      // Compared case-insensitively: .filament-brand is uppercased by CSS, and
      // innerText reports what is rendered rather than the underlying value.
      const brand = await page.locator(".filament-brand").innerText();
      if (!/polymaker/i.test(brand)) throw new Error(`brand shows "${brand}"`);
      return brand;
    });

    // Proves the fixture actually reached the card rather than the card merely
    // existing. Covers both paths: the populated fields render their values,
    // and max_speed_mm_s -- deliberately null in the fixture -- renders the
    // placeholder instead of "null" or an empty cell.
    await step(run, page, "spec values reach the card", async () => {
      const text = (await page.locator(".filament-card-specs").innerText()).replace(/\s+/g, " ");
      const expected = {
        "nozzle range": /190\s*-\s*230/,
        "bed range": /35\s*-\s*65/,
        density: /1\.24/,
        diameter: /1\.75/,
        "placeholder for the absent max speed": /Max Speed\s*--/,
      };
      const missing = Object.entries(expected)
        .filter(([, re]) => !re.test(text))
        .map(([name]) => name);
      if (missing.length) throw new Error(`${missing.join(", ")} not shown in: ${text}`);
      const rows = await page.locator(".filament-card .spec-row").count();
      return `${rows} spec rows, all values present`;
    });

    await step(run, page, "installed base profiles are offered", async () => {
      await page.waitForSelector(".base-profiles-section", { timeout: 15000 });
      const n = await page.locator(".base-profiles-list .base-profile-name").count();
      return `${n} base profiles`;
    });

    await step(run, page, "Generate opens the specs editor", async () => {
      await page.click(".filament-card-generate-btn");
      await page.waitForSelector(".editor-section", { timeout: 20000 });
      const inputs = await page.locator(".editor-section input").count();
      if (inputs === 0) throw new Error("editor rendered with no fields");
      return `${inputs} editable fields`;
    });

    // The editor builds the target label by prepending "Bambu Lab " to the bare
    // model name the backend sends. Pinning the exact string guards both ends of
    // that: a model name that already carries the prefix reads "Bambu Lab Bambu
    // Lab H2C", and a formatter that stops prepending loses it entirely.
    //
    // The expected model is H2C rather than anything else because of how the
    // editor seeds its selection: it starts from the frontend's hardcoded
    // default_target_printer_options() and only overrides that from the backend
    // when the seeded model is missing from the returned list. So the backend's
    // default_printer_model is effectively ignored whenever the two agree, which
    // they always do in production. Setting this fixture's default to a
    // different valid model does not change what the editor shows. That is a
    // latent inconsistency, not a visible bug, and not platform-specific -- left
    // alone deliberately, since changing selection behaviour is out of scope for
    // a test harness.
    await step(run, page, "target printer label is composed correctly", async () => {
      const label = await page
        .locator('xpath=//label[normalize-space()="Profile targets"]/following-sibling::input[1]')
        .inputValue();
      const expected = "Bambu Lab H2C 0.4 nozzle";
      if (label !== expected) throw new Error(`reads "${label}", expected "${expected}"`);
      return label;
    });

    await page.screenshot({ path: `flow-${engine}-filament.png`, fullPage: true });
  }

  // -- print analysis ------------------------------------------------------
  await step(run, page, "navigate to Print Analysis", async () => {
    await page.click('a[href="/analysis"]');
    await page.waitForSelector(".drop-zone", { timeout: 15000 });
  });

  const uploaded = await step(run, page, "choosing a photo moves to the ready state", async () => {
    await page.setInputFiles("#photo-file-input", {
      name: "print.png",
      mimeType: "image/png",
      buffer: png,
    });
    await page.waitForSelector(".analysis-preview", { timeout: 20000 });
  });

  if (uploaded) {
    // The frontend sniffs the leading bytes to build `data:<mime>;base64,...`.
    // WKWebView honours that declared type strictly and renders nothing when it
    // is wrong, while Chromium sniffs the content and hides the mistake. So
    // "did it actually decode" is the assertion that matters, not "is there an
    // <img> tag".
    await step(run, page, "preview image decodes in this engine", async () => {
      const img = page.locator("img.preview-image").first();
      await img.waitFor({ timeout: 15000 });
      await page.waitForFunction(
        () => {
          const el = document.querySelector("img.preview-image");
          return el && el.complete;
        },
        { timeout: 15000 }
      );
      const info = await img.evaluate((el) => ({
        w: el.naturalWidth,
        h: el.naturalHeight,
        mime: (el.src.match(/^data:([^;]+)/) ?? [])[1] ?? "none",
      }));
      if (info.mime !== "image/png") throw new Error(`declared MIME ${info.mime}, expected image/png`);
      if (info.w === 0 || info.h === 0) {
        throw new Error(`declared ${info.mime} but engine decoded ${info.w}x${info.h}`);
      }
      return `${info.mime} ${info.w}x${info.h}`;
    });

    await step(run, page, "GIF is sniffed and decoded too", async () => {
      await page.click("text=Choose Different Photo");
      await page.waitForSelector(".drop-zone", { timeout: 15000 });
      await page.setInputFiles("#photo-file-input", {
        name: "print.gif",
        mimeType: "application/octet-stream", // deliberately wrong: force sniffing
        buffer: GIF_1X1,
      });
      await page.waitForSelector(".analysis-preview", { timeout: 20000 });
      const info = await page.locator("img.preview-image").first().evaluate((el) => ({
        w: el.naturalWidth,
        mime: (el.src.match(/^data:([^;]+)/) ?? [])[1] ?? "none",
      }));
      if (info.mime !== "image/gif") throw new Error(`declared ${info.mime}, expected image/gif`);
      if (info.w === 0) throw new Error("engine could not decode the declared image/gif");
      return `${info.mime} ${info.w}px`;
    });

    await step(run, page, "profiles populate the target selector", async () => {
      await page.waitForSelector(".profile-select", { timeout: 20000 });
      const n = await page.locator(".profile-select option").count();
      if (n < 2) throw new Error("no profiles listed");
      return `${n - 1} profiles`;
    });

    await step(run, page, "analysis runs and reports defects", async () => {
      await page.selectOption(".profile-select", { index: 1 });
      await page.click("text=Analyze Print");
      await page.waitForSelector(".analysis-results", { timeout: 30000 });
      const text = await page.locator(".analysis-results").innerText();
      if (!/string/i.test(text)) throw new Error("detected defect not shown");
      return `${text.replace(/\s+/g, " ").slice(0, 70)}...`;
    });

    await step(run, page, "recommendations are listed", async () => {
      const text = await page.locator(".analysis-results").innerText();
      for (const want of ["Nozzle Temperature", "Retraction"]) {
        if (!text.includes(want)) throw new Error(`"${want}" missing from results`);
      }
      return "temperature and retraction shown";
    });

    // The apply dialog is the overlay whose full-screen positioning was the
    // original macOS suspicion, so measure it with real content in it. The
    // button only renders once a target profile is selected, which the previous
    // step did.
    await step(run, page, "apply dialog covers the viewport", async () => {
      await page.click(".apply-btn");
      await page.waitForSelector(".change-preview-overlay", { timeout: 15000 });
      const box = await page.locator(".change-preview-overlay").boundingBox();
      const vp = page.viewportSize();
      if (!box) throw new Error("overlay has no box");
      if (box.width < vp.width - 2 || box.height < vp.height - 2) {
        throw new Error(`overlay ${box.width}x${box.height}, viewport ${vp.width}x${vp.height}`);
      }
      return `${Math.round(box.width)}x${Math.round(box.height)}`;
    });

    await page.screenshot({ path: `flow-${engine}-analysis.png`, fullPage: false });
  }

  // -- profile management ----------------------------------------------------
  //
  // The apply dialog from the previous flow covers the whole viewport, so it
  // would swallow the sidebar click that starts this section.
  await step(run, page, "apply dialog can be dismissed", async () => {
    await page
      .locator(".change-preview-overlay")
      .getByRole("button", { name: "Cancel" })
      .click();
    await page.waitForSelector(".change-preview-overlay", {
      state: "detached",
      timeout: 15000,
    });
  });

  await step(run, page, "navigate to Profiles", async () => {
    await page.click('a[href="/profiles"]');
    await page.waitForSelector(".profile-management-page", { timeout: 15000 });
    await page.waitForSelector(".profile-list-item", { timeout: 15000 });
    return `${await page.locator(".profile-list-item").count()} profiles listed`;
  });

  await step(run, page, "opening a profile shows its fields", async () => {
    await page.locator(".profile-list-item").first().click();
    await page.waitForSelector(".profile-detail-title", { timeout: 15000 });
    await page.waitForSelector("table.profile-fields", { timeout: 15000 });
    const title = (await page.locator(".profile-detail-title").first().innerText()).trim();
    const subtitle = (await page.locator(".profile-detail-subtitle").first().innerText()).trim();
    const keys = await page.locator("td.field-key").count();
    if (keys === 0) throw new Error("fields table rendered with no rows");
    return `${title} (${subtitle}), ${keys} field rows`;
  });

  // -- batch generate --------------------------------------------------------
  await step(run, page, "navigate to Batch Generate", async () => {
    await page.click('a[href="/batch"]');
    await page.waitForSelector(".batch-generate-page", { timeout: 15000 });
    // The real select replaces a disabled "Loading brands..." placeholder only
    // once list_catalog_brands resolves.
    await page.waitForSelector("select#brand-select", { timeout: 15000 });
    return `${await page.locator("select#brand-select option").count()} brand options`;
  });

  await step(run, page, "batch run reports per-filament results", async () => {
    // Index rather than value: option 0 is the "-- Select a brand --" prompt,
    // so this picks Polymaker out of the fixture's three brands.
    await page.selectOption("select#brand-select", { index: 2 });
    const generate = page.locator(".batch-generate-page button.btn-primary").first();
    if (await generate.isDisabled()) throw new Error("Generate stayed disabled after picking a brand");
    await generate.click();
    await page.waitForSelector(".batch-results", { timeout: 20000 });
    const summary = (await page.locator(".batch-summary").first().innerText()).replace(/\s+/g, " ").trim();
    const failed = await page.locator("tr.row-fail").count();
    if (failed !== 1) throw new Error(`expected 1 failed row, got ${failed}`);
    return summary;
  });

  // -- compare profiles ------------------------------------------------------
  await step(run, page, "navigate to Compare Profiles", async () => {
    await page.click('a[href="/compare"]');
    await page.waitForSelector(".profile-diff-page", { timeout: 15000 });
  });

  // SearchableSelect commits on mousedown, like the filament autocomplete, so a
  // blur handler cannot close the dropdown before the choice registers.
  await step(run, page, "both diff pickers accept a profile", async () => {
    const pick = async (index, text) => {
      const group = page.locator(".diff-pickers .form-group").nth(index);
      await group.locator(".ss-display").click();
      const option = group.locator(".ss-option", { hasText: text }).first();
      await option.waitFor({ timeout: 15000 });
      await option.click();
    };
    await pick(0, "PolyLite");
    await pick(1, "Bambu PLA Basic");
    const shown = await page.locator(".diff-pickers .ss-display-text").allInnerTexts();
    return shown.map((s) => s.trim()).join(" vs ");
  });

  await step(run, page, "compare renders a categorised diff", async () => {
    await page.locator(".diff-actions button.btn-primary").first().click();
    await page.waitForSelector(".diff-results", { timeout: 20000 });
    const summary = (await page.locator(".diff-summary-text").first().innerText()).trim();
    const categories = await page.locator(".diff-category-name").count();
    const rows = await page.locator("tr.diff-row.changed").count();
    if (categories !== 2) throw new Error(`expected 2 categories, got ${categories}`);
    if (rows !== 3) throw new Error(`expected 3 changed rows, got ${rows}`);
    return `${summary}, ${categories} categories`;
  });

  // .diff-table carries a border-collapse fix for WebKit. Counting rows would
  // still pass if they collapsed to zero height, so measure one instead.
  await step(run, page, "diff table rows have real height", async () => {
    const box = await page.locator("tr.diff-row.changed").first().boundingBox();
    if (!box) throw new Error("first diff row has no box");
    if (box.height < 8) throw new Error(`first diff row is only ${box.height}px tall`);
    return `row height ${Math.round(box.height)}px`;
  });

  // -- settings --------------------------------------------------------------
  await step(run, page, "navigate to Settings", async () => {
    await page.click('a[href="/settings"]');
    await page.waitForSelector(".settings-page", { timeout: 15000 });
  });

  // The frontend compiles to wasm32, where cfg!(target_os) is "unknown", so the
  // path hint is chosen at runtime from navigator.userAgent. Both engines report
  // a Mac UA on this runner, so the macOS hint is the right answer for both.
  // This is the one assertion here that no amount of Windows testing could make.
  await step(run, page, "path hint matches the host platform", async () => {
    await page.waitForSelector("#bambu-path", { timeout: 15000 });
    const ua = await page.evaluate(() => navigator.userAgent);
    if (!/Mac OS X|Macintosh/.test(ua)) throw new Error(`runner is not a Mac: ${ua}`);
    const hint = await page.locator("#bambu-path").getAttribute("placeholder");
    if (!hint.includes("/Users/") || !hint.includes("Library/Application Support")) {
      throw new Error(`host is macOS but the hint reads "${hint}"`);
    }
    return hint;
  });

  await step(run, page, "model list populates from the provider", async () => {
    await page.waitForSelector("#ai-model", { timeout: 20000 });
    const options = await page.locator("#ai-model option").count();
    if (options === 0) throw new Error("model select rendered empty");
    return `${options} models`;
  });

  // One ApiKeyForm per provider, rendered only while AI filament search is on.
  await step(run, page, "an API key form renders per provider", async () => {
    const rows = await page.locator(".settings-page .api-key-form").count();
    if (rows !== 4) throw new Error(`expected 4 key forms, got ${rows}`);
    return `${rows} providers`;
  });

  await page.screenshot({ path: `flow-${engine}-settings.png`, fullPage: false });

  // -- health check ----------------------------------------------------------
  await step(run, page, "navigate to Health Check", async () => {
    await page.click('a[href="/health"]');
    await page.waitForSelector(".health-page", { timeout: 15000 });
    await page.waitForSelector(".health-results", { timeout: 20000 });
    const items = await page.locator(".health-results .health-item").count();
    if (items !== 6) throw new Error(`expected 6 health rows, got ${items}`);
    return (await page.locator(".health-results .health-summary").first().innerText()).trim();
  });

  await step(run, page, "diagnostics panel badges each result", async () => {
    await page.locator(".diagnostics-controls button.btn-primary").first().click();
    await page.waitForSelector(".diagnostics-results", { timeout: 20000 });
    const pass = await page.locator(".diagnostics-row-pass").count();
    const warn = await page.locator(".diagnostics-row-warn").count();
    if (pass < 1 || warn < 1) throw new Error(`pass=${pass}, warn=${warn}`);
    // A remedy is rendered only for a non-passing check that carries one, so
    // this covers the branch a report of all-passes would never reach.
    const remedy = await page.locator(".diagnostics-remedy").count();
    if (remedy < 1) throw new Error("the warned check rendered no remedy");
    return `${pass} pass, ${warn} warn, ${remedy} remedy`;
  });

  await page.screenshot({ path: `flow-${engine}-health.png`, fullPage: false });

  // -- about -----------------------------------------------------------------
  await step(run, page, "navigate to About", async () => {
    await page.click('a[href="/about"]');
    await page.waitForSelector(".about-page", { timeout: 15000 });
    const version = (await page.locator(".about-version-value").first().innerText()).trim();
    if (!version.includes("1.3.0")) throw new Error(`version reads "${version}"`);
    return version;
  });

  // update_info is seeded by the startup check, so this renders without the
  // user pressing anything.
  await step(run, page, "update state reports up to date", async () => {
    await page.waitForSelector(".about-up-to-date", { timeout: 20000 });
    return (await page.locator(".about-up-to-date").first().innerText()).replace(/\s+/g, " ").trim();
  });

  run.unknown = await page.evaluate(() => [...new Set(window.__ipc.unknown)]).catch(() => []);
  await browser.close();
  return run;
}

const server = await serve();
const baseUrl = `http://127.0.0.1:${server.address().port}/`;
console.log(`Serving ${distDir} at ${baseUrl}`);

let wk;
let cr;
try {
  wk = await driveApp(webkit, "webkit", baseUrl);
  cr = await driveApp(chromium, "chromium", baseUrl);
} finally {
  server.close();
}

console.log("\n== summary ==");
for (const run of [wk, cr]) {
  console.log(`${run.engine}: ${run.steps.filter((s) => s.ok).length}/${run.steps.length} steps passed`);
  for (const e of [...new Set(run.errors)]) console.log(`  runtime error: ${e}`);
  if (run.unknown.length) {
    console.log(`  commands with no fixture: ${run.unknown.join(", ")}`);
  }
}

const webkitOnly = wk.failed.filter((n) => !cr.failed.includes(n));
const bothFailed = wk.failed.filter((n) => cr.failed.includes(n));
const webkitOnlyErrors = [...new Set(wk.errors)].filter((e) => !cr.errors.includes(e));

if (bothFailed.length) {
  console.log(`\nFailing in both engines (not macOS-specific): ${bothFailed.join("; ")}`);
}
if (webkitOnlyErrors.length) {
  console.log("\nRuntime errors seen only in WebKit:");
  for (const e of webkitOnlyErrors) console.log(`  ${e}`);
}

// A step that fails in both engines is a plain bug and should be fixed, but it
// is not what this job is guarding, and failing on it here would make every
// unrelated regression look like a macOS problem. WebKit-only failures are.
if (webkitOnly.length || webkitOnlyErrors.length) {
  console.error(`\nFAIL: WebKit-only breakage: ${[...webkitOnly, ...webkitOnlyErrors].join("; ")}`);
  process.exit(1);
}
if (bothFailed.length || wk.errors.length) {
  console.error("\nFAIL: the app misbehaved in both engines (see above).");
  process.exit(1);
}
console.log("\nPASS: every flow completed in both engines.");
