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
import { isDeepStrictEqual } from "node:util";
import { deflateSync } from "node:zlib";
import { extname, join, resolve } from "node:path";
import {
  A3_ASSIGNED,
  FIXTURES,
  GIF_1X1,
  makePng,
  PETG,
  PRINTER_FINGERPRINT,
  PRINTER_SERIAL,
  PRINTER_UNCONFIGURED,
  PRINTER_VIEW,
  SLICE_MODEL,
  STL_INBOX_FILE,
  sliceJob,
  sliceResult,
  UNSYNCED_CONFIRMED_PATH,
  withSlot,
} from "./fixtures.mjs";

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
  const handlers = {};
  // `settled` lists each command once its answer (or refusal) is out.
  const settled = [];
  window.__ipc = { calls, unknown, settled };
  // Steps may change answers mid-run through window.__fixtures. A fixture
  // of the form { __sequence: [a, b, …] } answers a, then b, …; the last repeats.
  const live = (window.__fixtures = structuredClone(fixtures));
  // `{ __hold: key, answer }` answers only once a step calls __release(key).
  const holds = {};
  const gate = (key) => (holds[key] ||= {}).promise ||= new Promise((r) => (holds[key].open = r));
  window.__release = (key) => {
    gate(key);
    holds[key].open();
  };
  const answerFor = async (cmd) => {
    // `{ __delay: ms, answer }` answers late, like a slow backend;
    // `{ __reject: value }` answers the way a Tauri command's Err does.
    let answer = live[cmd];
    if (answer && Array.isArray(answer.__sequence)) {
      const seq = answer.__sequence;
      answer = seq.length > 1 ? seq.shift() : seq[0];
    }
    if (answer && typeof answer === "object" && "__hold" in answer) {
      const { __hold, answer: late } = answer;
      await gate(__hold);
      answer = late;
    }
    if (answer && typeof answer === "object" && "__delay" in answer) {
      const { __delay, answer: late } = answer;
      await new Promise((r) => setTimeout(r, __delay));
      answer = late;
    }
    if (answer && typeof answer === "object" && "__reject" in answer) throw answer.__reject;
    return structuredClone(answer);
  };
  // The real backend recomputes get_feature_flags from stored preferences, so
  // flipping the Settings page's AI toggle off should be reflected the next
  // time the frontend re-fetches flags. The fixture itself is static, so this
  // is the cheapest way to get a real AI-off state without a second fixture
  // set: track the one preference write that matters and answer accordingly.
  let analysisEnabled = fixtures.get_feature_flags?.analysis_enabled ?? true;
  const invoke = async (cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "set_preference" && args?.key === "filament_search_use_ai" && args?.value === "false") {
      analysisEnabled = false;
    }
    if (cmd === "get_feature_flags") {
      settled.push(cmd);
      return { ...live.get_feature_flags, analysis_enabled: analysisEnabled };
    }
    if (!(cmd in live)) {
      unknown.push(cmd);
      throw new Error(`no fixture for command '${cmd}'`);
    }
    try {
      return await answerFor(cmd);
    } finally {
      settled.push(cmd);
    }
  };
  const listen = async (name, cb) => {
    (handlers[name] ||= []).push(cb);
    return () => {};
  };
  window.__emit = (name, payload) => (handlers[name] || []).forEach((cb) => cb({ event: name, payload }));
  window.__TAURI__ = { core: { invoke }, event: { listen } };
  window.__TAURI_INTERNALS__ = { invoke };
}

const png = makePng(320, 240, deflateSync);

/** The view once the printer reports the preset assigned to A3. */
function a3Matched() {
  const a3 = PRINTER_VIEW.slots.find((s) => s.label === "A3");
  return withSlot("A3", {
    ...A3_ASSIGNED,
    status: "matches",
    tray: { ...a3.tray, tray_type: "PLA", tray_info_idx: "PA-PL-WHTPA0-01" },
  });
}

/** Invoke arguments compared exactly, whatever their key order. */
function sameArgs(got, want) {
  const norm = (o) => JSON.stringify(Object.keys(o).sort().map((k) => [k, o[k]]));
  return norm(got ?? {}) === norm(want);
}

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

  // -- rail sidebar ----------------------------------------------------------
  await step(run, page, "rail is collapsed to 64px with icons only", async () => {
    const w = await page.locator("nav.sidebar").evaluate((el) => el.getBoundingClientRect().width);
    if (Math.round(w) !== 64) throw new Error(`rail width ${w}`);
    const labelOpacity = await page.locator(".nav-label").first().evaluate((el) => getComputedStyle(el).opacity);
    if (labelOpacity !== "0") throw new Error(`label opacity ${labelOpacity}`);
  });

  await step(run, page, "hovering expands the rail over the content", async () => {
    const before = await page.locator(".content").evaluate((el) => el.getBoundingClientRect().left);
    await page.hover("nav.sidebar");
    await page.waitForFunction(() => document.querySelector("nav.sidebar").getBoundingClientRect().width >= 219, null, { timeout: 3000 });
    const after = await page.locator(".content").evaluate((el) => el.getBoundingClientRect().left);
    if (before !== after) throw new Error(`content moved from ${before} to ${after}`);
    // "220px with labels visible" -- opacity is a transition, so poll for it
    // rather than reading it once.
    await page.waitForFunction(
      () => getComputedStyle(document.querySelector(".nav-label")).opacity === "1",
      null,
      { timeout: 3000 }
    );
    if (await page.locator(".sidebar-wordmark").count()) {
      await page.waitForFunction(
        () => getComputedStyle(document.querySelector(".sidebar-wordmark")).opacity === "1",
        null,
        { timeout: 3000 }
      );
    }
    await page.mouse.move(900, 400);
    await page.waitForFunction(() => document.querySelector("nav.sidebar").getBoundingClientRect().width <= 65, null, { timeout: 3000 });
  });

  // Real Tab presses were tried first (the concern being that programmatic
  // focus() doesn't reliably set :focus-visible), but WebKit's headless
  // engine -- like real Safari's default "Text boxes and lists only" keyboard
  // setting -- never gives an <a> Tab focus at all here, in either direction,
  // so a key-driven approach can't reach this link in that engine. Verified
  // directly: after the preceding hover step, page.locator(...).focus() does
  // set :focus-visible (and expands the rail) in both engines, so it is used
  // here instead. See Task 6 report for the measurements.
  //
  // What this step proves: once a link is focus-visible, the rail's CSS
  // reacts correctly (expands, labels become visible). What it does NOT
  // prove: that the link is reachable by an actual Tab key press -- WebKit's
  // headless engine can't exercise that here (see above), so the tabIndex
  // check below is the closest available guard against a link silently
  // dropping out of the tab order (tabindex="-1"), which programmatic
  // .focus() would not otherwise reveal.
  await step(run, page, "keyboard focus expands the rail", async () => {
    await page.locator('nav.sidebar a[href="/profiles"]').focus();
    await page.waitForFunction(() => document.querySelector("nav.sidebar").getBoundingClientRect().width >= 219, null, { timeout: 3000 });
    await page.waitForFunction(
      () => getComputedStyle(document.querySelector(".nav-label")).opacity === "1",
      null,
      { timeout: 3000 }
    );
    if (await page.locator(".sidebar-wordmark").count()) {
      await page.waitForFunction(
        () => getComputedStyle(document.querySelector(".sidebar-wordmark")).opacity === "1",
        null,
        { timeout: 3000 }
      );
    }
    const outOfOrder = await page
      .locator("nav.sidebar a")
      .evaluateAll((els) => els.filter((el) => el.tabIndex < 0).map((el) => el.getAttribute("href")));
    if (outOfOrder.length) throw new Error(`removed from tab order: ${outOfOrder.join(", ")}`);
    await page.evaluate(() => document.activeElement.blur());
  });

  await step(run, page, "active route shows the signal tick", async () => {
    const current = await page.locator('nav.sidebar a[aria-current="page"]').getAttribute("href");
    if (current !== "/") throw new Error(`aria-current on ${current}`);
    const tick = await page.locator('nav.sidebar a[aria-current="page"]').evaluate((el) => getComputedStyle(el, "::before").backgroundColor);
    if (!tick || tick === "rgba(0, 0, 0, 0)") throw new Error(`no tick colour (${tick})`);
  });

  // Regression for the exact bug the Task 5 fix round addressed: Chromium
  // focuses a link on mousedown, and with :focus-within that kept the rail
  // stuck open after an ordinary click. :has(:focus-visible) should not.
  await step(run, page, "clicking a nav link does not leave the rail open", async () => {
    await page.click('nav.sidebar a[href="/analysis"]');
    await page.mouse.move(900, 400);
    await page.waitForFunction(() => document.querySelector("nav.sidebar").getBoundingClientRect().width <= 65, null, { timeout: 3000 });
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

  // -- slice ---------------------------------------------------------------
  const emitJob = (view) => page.evaluate((v) => window.__emit("slicer://job", v), view);
  const callsOf = (cmd) => page.evaluate((c) => window.__ipc.calls.filter((x) => x.cmd === c), cmd);
  const running = (id, percent) =>
    sliceJob(id, { state: "running", progress: { plate: 1, percent, stage: "Generating walls" } });
  const done = (id, result, extra) => sliceJob(id, { state: "done", result, cached: false }, extra);

  await step(run, page, "navigate to Slice", async () => {
    await page.click('a[href="/slice"]');
    await page.waitForSelector(".slice-page.nd", { timeout: 15000 });
    await page.waitForFunction(() => document.querySelector(".sl-version")?.innerText.includes("02.08.02.61"), null, { timeout: 10000 });
  });

  await step(run, page, "preset pickers load for the default printer", async () => {
    await page.waitForFunction(() => document.querySelectorAll("#sl-printer option").length === 2, null, { timeout: 10000 });
    const printer = await page.locator("#sl-printer").inputValue();
    const filament = await page.locator("#sl-filament").inputValue();
    if (printer !== "Bambu Lab H2C 0.4 nozzle") throw new Error(`printer ${printer}`);
    if (filament !== "Bambu PLA Basic @BBL H2C") throw new Error(`filament ${filament}`);
    const asked = (await callsOf("slicer_presets")).map((c) => c.args.printer);
    if (!asked.includes("Bambu Lab H2C 0.4 nozzle")) throw new Error(`presets asked for ${JSON.stringify(asked)}`);
    return `${await page.locator("#sl-filament option").count()} filaments`;
  });

  await step(run, page, "choosing a file and pressing Slice queues a job", async () => {
    await page.click(".sl-browse");
    await page.waitForFunction(() => document.querySelector(".sl-model")?.innerText === "cube.stl", null, { timeout: 5000 });
    await page.click(".sl-slice");
    await page.waitForSelector(".sl-job-status", { timeout: 5000 });
    const [call] = await callsOf("slicer_slice");
    const a = call.args;
    if (a.modelPath !== SLICE_MODEL || a.filament !== "Bambu PLA Basic @BBL H2C" || a.bedType !== "Textured PEI Plate") {
      throw new Error(JSON.stringify(a));
    }
    const status = await page.locator(".sl-job-status").innerText();
    if (!status.startsWith("Queued")) throw new Error(`status ${status}`);
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "slicer_set_settings"), null, { timeout: 5000 });
  });

  await step(run, page, "progress shows and Cancel reaches the backend", async () => {
    await emitJob(running(1, 50));
    await page.waitForFunction(() => document.querySelector(".sl-job-status")?.innerText === "Plate 1 · Generating walls · 50%", null, { timeout: 5000 });
    await page.click(".sl-cancel");
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "slicer_cancel" && c.args.jobId === 1), null, { timeout: 5000 });
  });

  await step(run, page, "a finished job shows time, weight, cost, warnings and the plate", async () => {
    await emitJob(done(1, sliceResult()));
    await page.waitForSelector(".sl-hero", { timeout: 5000 });
    const hero = await page.locator(".sl-hero").innerText();
    const weight = await page.locator(".sl-weight").innerText();
    const cost = await page.locator(".sl-cost").innerText();
    if (hero !== "14m" || weight !== "3.69 g" || cost !== "0.07") throw new Error(`${hero} / ${weight} / ${cost}`);
    if ((await page.locator(".sl-warning-warning").count()) !== 1) throw new Error("warning not shown");
    await page.waitForSelector(".sl-thumb img", { timeout: 5000 });
    const loaded = await page.locator(".sl-thumb img").evaluate((img) => img.complete && img.naturalWidth > 0);
    if (!loaded) throw new Error("thumbnail did not decode");
  });

  await step(run, page, "Open in Bambu Studio hands over the sliced file", async () => {
    await page.click(".sl-open");
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "slicer_open_in_bambu_studio" && c.args.jobId === 1), null, { timeout: 5000 });
  });

  await page.screenshot({ path: `flow-${engine}-slice.png`, fullPage: false });

  await step(run, page, "a failed job shows Bambu Studio's message", async () => {
    const message = "Bambu Studio couldn't slice this model: No valid nozzle found. Please check nozzle count.";
    await emitJob(sliceJob(1, { state: "failed", error: { kind: "slicer", message } }));
    await page.waitForSelector(".sl-job-error", { timeout: 5000 });
    const text = await page.locator(".sl-job-error").innerText();
    if (text !== message) throw new Error(text);
  });

  await step(run, page, "compare queues one job per filament and marks the best", async () => {
    await page.click(".sl-compare-toggle");
    await page.selectOption(".sl-compare-filament", PETG);
    await page.click(".sl-slice");
    await page.waitForFunction(() => window.__ipc.calls.filter((c) => c.cmd === "slicer_slice").length === 3, null, { timeout: 5000 });
    const filaments = (await callsOf("slicer_slice")).slice(1).map((c) => c.args.filament);
    if (filaments[1] !== PETG) throw new Error(JSON.stringify(filaments));
    await emitJob(done(2, sliceResult()));
    await emitJob(done(3, sliceResult({ time: 990, weight: 4.1, cost: 0.09, warnings: 0 }), { filament: PETG }));
    await page.waitForSelector(".sl-compare-table td.best", { timeout: 5000 });
    const cols = await page.locator(".sl-compare-table thead th").count();
    if (cols !== 3) throw new Error(`${cols} header cells`);
    const deltas = await page.locator(".sl-compare-table .sl-delta").allInnerTexts();
    if (!deltas.includes("+2m") || !deltas.includes("+0.41 g")) throw new Error(JSON.stringify(deltas));
    return deltas.join(", ");
  });

  // Swaps some fixtures for the length of `fn`, then puts the old ones back.
  const setFixture = (cmd, value) =>
    page.evaluate(([c, v]) => {
      window.__fixtures[c] = v;
    }, [cmd, value]);
  const withFixtures = async (changes, fn) => {
    const saved = await page.evaluate((keys) => keys.map((k) => [k, window.__fixtures[k]]), Object.keys(changes));
    for (const [cmd, value] of Object.entries(changes)) await setFixture(cmd, value);
    try {
      return await fn();
    } finally {
      for (const [cmd, value] of saved) await setFixture(cmd, value);
    }
  };
  const openJob = async (id) => {
    await page.click(`.sl-recent-row[data-job="${id}"]`);
    await page.waitForSelector(`.sl-recent-row[data-job="${id}"].active`, { timeout: 5000 });
  };
  const textOf = (sel) => page.waitForSelector(sel, { timeout: 5000 }).then((el) => el.innerText());
  const FILES_CLEARED = { kind: "files_cleared", message: "That slice's files were cleared; slice it again." };
  const settledCount = (cmd) => page.evaluate((c) => window.__ipc.settled.filter((x) => x === c).length, cmd);
  const callCount = async (cmd) => (await callsOf(cmd)).length;
  // Waits until `cmd` has been called (or answered) more than `n` times.
  const calledPast = (cmd, n) =>
    page.waitForFunction(([c, k]) => window.__ipc.calls.filter((x) => x.cmd === c).length > k, [cmd, n], { timeout: 5000 });
  const settledPast = (cmd, n) =>
    page.waitForFunction(([c, k]) => window.__ipc.settled.filter((x) => x === c).length > k, [cmd, n], { timeout: 5000 });
  const release = (key) => page.evaluate((k) => window.__release(k), key);
  const queued = (id) => sliceJob(id, { state: "queued", position: 0 });

  await step(run, page, "progress events update in place and keep focus", async () => {
    // Compare shows jobs 2 and 3, with 2 open. Job 3 slices again while the
    // filament picker has focus: only job 3's text may change.
    const petg = (state) => sliceJob(3, state, { filament: PETG });
    await page.waitForSelector(".sl-hero", { timeout: 5000 });
    await page.evaluate(() => {
      document.querySelector(".sl-hero").__mark = 1;
      document.querySelector('.sl-recent-row[data-job="2"]').__mark = 1;
      document.querySelector(".sl-compare-table thead th:nth-child(2)").__mark = 1;
    });
    await page.focus("#sl-filament");
    const thumbs = (await callsOf("slicer_thumbnail")).length;
    for (const percent of [20, 40, 60]) {
      await emitJob(petg({ state: "running", progress: { plate: 1, percent, stage: "Generating walls" } }));
    }
    await page.waitForFunction(
      () => document.querySelector(".sl-compare-table thead th:nth-child(3) .sl-col-state")?.innerText.endsWith("60%"),
      null,
      { timeout: 5000 },
    );
    const kept = await page.evaluate(() => ({
      hero: document.querySelector(".sl-hero").__mark === 1,
      row: document.querySelector('.sl-recent-row[data-job="2"]').__mark === 1,
      column: document.querySelector(".sl-compare-table thead th:nth-child(2)").__mark === 1,
      focus: document.activeElement?.id,
    }));
    if (!kept.hero || !kept.row || !kept.column || kept.focus !== "sl-filament") throw new Error(JSON.stringify(kept));
    const refetched = (await callsOf("slicer_thumbnail")).length - thumbs;
    if (refetched !== 0) throw new Error(`the open result fetched its thumbnail ${refetched} more times`);
    await emitJob(petg({ state: "done", result: sliceResult({ time: 990, weight: 4.1, cost: 0.09, warnings: 0 }), cached: false }));
    await page.waitForSelector(".sl-compare-table td.best", { timeout: 5000 });
  });

  await step(run, page, "plate tabs switch the plate, its numbers and thumbnail", async () => {
    // Plate 2 uses the same slot and repeats plate 1's warning as a notice,
    // so a row kept from plate 1 would show its numbers or level.
    const two = sliceResult();
    const first = two.plates[0];
    two.plates.push({
      ...first,
      index: 2,
      time_seconds: 8040,
      thumbnail: "plate_2.png",
      filaments: [{ slot: 1, filament_type: "PETG", color: "#112233", used_g: 7.25, used_m: 4.1, cost: 0.2 }],
      warnings: [{ ...first.warnings[0], level: "notice" }],
    });
    await emitJob(done(9, two));
    await openJob(9);
    await page.waitForSelector(".sl-plate-tab", { timeout: 5000 });
    const tabs = await page.locator(".sl-plate-tab").allInnerTexts();
    if (JSON.stringify(tabs) !== JSON.stringify(["Plate 1", "Plate 2"])) throw new Error(JSON.stringify(tabs));
    await page.click(".sl-plate-tab:nth-child(2)");
    await page.waitForFunction(() => document.querySelector(".sl-hero")?.innerText === "2h 14m", null, { timeout: 5000 });
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "slicer_thumbnail" && c.args.jobId === 9 && c.args.plate === 2), null, { timeout: 5000 });
    const filament = await page.locator(".sl-filament").allInnerTexts();
    if (JSON.stringify(filament) !== JSON.stringify(["Slot 1 · PETG · 7.25 g · 4.10 m"])) throw new Error(JSON.stringify(filament));
    const swatch = await page.locator(".sl-swatch").evaluate((el) => getComputedStyle(el).backgroundColor);
    if (swatch !== "rgb(17, 34, 51)") throw new Error(`swatch ${swatch}`);
    const levels = await page.locator(".sl-warning").evaluateAll((els) => els.map((el) => el.className));
    if (JSON.stringify(levels) !== JSON.stringify(["sl-warning sl-warning-notice"])) throw new Error(JSON.stringify(levels));
    if ((await page.locator(".sl-plate-tab.active").innerText()) !== "Plate 2") throw new Error("tab not marked");
  });

  await step(run, page, "leaving the page mid-job is safe and the job finishes", async () => {
    await page.click(".sl-compare-toggle");
    await page.click(".sl-slice");
    await page.waitForFunction(() => window.__ipc.calls.filter((c) => c.cmd === "slicer_slice").length === 4, null, { timeout: 5000 });
    await emitJob(running(4, 10));
    await page.click('a[href="/about"]');
    await page.waitForSelector(".about-page", { timeout: 15000 });
    await emitJob(running(4, 80));
    await emitJob(done(4, sliceResult()));
    await page.click('a[href="/slice"]');
    await page.waitForSelector(".slice-page", { timeout: 15000 });
    const rows = await page.locator(".sl-recent-row").allInnerTexts();
    if (!rows.some((r) => r.includes("Done"))) throw new Error(JSON.stringify(rows));
  });

  await step(run, page, "slicer errors show inline, as plain text", async () => {
    await openJob(4);
    // The page was left and opened again, so the model has to be chosen again.
    await page.click(".sl-browse");
    await page.waitForFunction(() => document.querySelector(".sl-model")?.innerText === "cube.stl", null, { timeout: 5000 });
    const failed = (kind, message) => emitJob(sliceJob(4, { state: "failed", error: { kind, message } }));
    const shows = async (message) => {
      await page.waitForFunction((m) => document.querySelector(".sl-job-error")?.innerText === m, message, { timeout: 5000 });
    };
    const incompatible = "Process '0.20mm Standard @BBL H2C' isn't made for printer 'Bambu Lab H2S 0.4 nozzle'.";
    await failed("incompatible_process", incompatible);
    await shows(incompatible);
    const orphan = "Preset 'My PLA' is missing its parent 'Generic PLA @base'.";
    await failed("missing_parent", orphan);
    await shows(orphan);
    const markup = "Bambu Studio couldn't slice this model: <img src=x onerror=\"window.__pwned=1\"><b>bold</b>";
    await failed("slicer", markup);
    await shows(markup);
    if ((await page.locator(".sl-job-error img, .sl-job-error b").count()) !== 0) throw new Error("error text became markup");
    await withFixtures({ slicer_slice: { __reject: "BambuMate is closing." } }, async () => {
      const saves = await callCount("slicer_set_settings");
      const slices = await settledCount("slicer_slice");
      await page.click(".sl-slice");
      await settledPast("slicer_slice", slices);
      await page.waitForFunction(() => document.querySelector(".sl-input-error")?.innerText === "BambuMate is closing.", null, { timeout: 5000 });
      await page.waitForSelector(".sl-slice:not([disabled])", { timeout: 5000 });
      if ((await callCount("slicer_set_settings")) !== saves) throw new Error("settings saved though nothing was queued");
    });
    if (await page.evaluate(() => window.__pwned)) throw new Error("error text ran a script");
  });

  await step(run, page, "a presets error shows inline and clears on the next load", async () => {
    const message = "Bambu Studio isn't installed. Install it to slice in BambuMate.";
    await withFixtures({ slicer_presets: { __reject: message } }, async () => {
      await page.selectOption("#sl-printer", "Bambu Lab H2S 0.4 nozzle");
      const text = await textOf(".sl-preset-error");
      if (text !== message) throw new Error(text);
    });
    await page.selectOption("#sl-printer", "Bambu Lab H2C 0.4 nozzle");
    await page.waitForSelector(".sl-preset-error", { state: "detached", timeout: 5000 });
  });

  await step(run, page, "a compare pick the new printer lacks is cleared", async () => {
    await page.click(".sl-compare-toggle");
    await page.selectOption(".sl-compare-filament", PETG);
    const presets = await page.evaluate(() => window.__fixtures.slicer_presets);
    const noPetg = { ...presets, filaments: presets.filaments.filter((f) => f.name !== PETG) };
    await withFixtures({ slicer_presets: noPetg }, async () => {
      const loads = await settledCount("slicer_presets");
      await page.selectOption("#sl-printer", "Bambu Lab H2S 0.4 nozzle");
      await settledPast("slicer_presets", loads);
      await page.waitForFunction(() => document.querySelector(".sl-compare-filament")?.value === "", null, { timeout: 5000 });
      // What is shown is what gets sliced: the main filament only.
      const slices = await callCount("slicer_slice");
      const saves = await settledCount("slicer_set_settings");
      await page.click(".sl-slice");
      await settledPast("slicer_set_settings", saves);
      const sliced = (await callsOf("slicer_slice")).slice(slices).map((c) => c.args.filament);
      if (JSON.stringify(sliced) !== JSON.stringify([sliceJob(0, null).filament])) throw new Error(JSON.stringify(sliced));
    });
    const loads = await settledCount("slicer_presets");
    await page.selectOption("#sl-printer", "Bambu Lab H2C 0.4 nozzle");
    await settledPast("slicer_presets", loads);
    await page.selectOption(".sl-compare-filament", PETG);
    const shown = await page.locator(".sl-compare-filament").evaluate((el) => el.selectedOptions[0]?.textContent);
    if (shown !== PETG) throw new Error(`shows ${shown}`);
    await page.click(".sl-compare-toggle");
  });

  await step(run, page, "cleared files show inline and Slice again queues the same job", async () => {
    await withFixtures(
      {
        slicer_thumbnail: { __reject: FILES_CLEARED },
        slicer_open_in_bambu_studio: { __reject: FILES_CLEARED },
        slicer_slice: queued(6),
      },
      async () => {
        await emitJob(done(5, sliceResult()));
        await openJob(5);
        if ((await textOf(".sl-action-error")) !== FILES_CLEARED.message) throw new Error("thumbnail error not shown");
        await page.click(".sl-open");
        await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "slicer_open_in_bambu_studio" && c.args.jobId === 5), null, { timeout: 5000 });
        if ((await textOf(".sl-action-error")) !== FILES_CLEARED.message) throw new Error("open error not shown");
        // "Slice again" is offered once the model turned out to be still there.
        await page.waitForSelector(".sl-reslice", { timeout: 5000 });
        const checked = (await callsOf("slicer_model_exists")).map((c) => c.args.path);
        if (!checked.includes(SLICE_MODEL)) throw new Error(`checked ${JSON.stringify(checked)}`);
        const before = (await callsOf("slicer_slice")).length;
        await page.click(".sl-reslice");
        await page.waitForSelector('.sl-recent-row[data-job="6"].active', { timeout: 5000 });
        const calls = await callsOf("slicer_slice");
        if (calls.length !== before + 1) throw new Error(`${calls.length - before} slice calls`);
        const a = calls.at(-1).args;
        const want = sliceJob(0, null);
        const got = JSON.stringify([a.modelPath, a.printer, a.process, a.filament, a.bedType]);
        const expected = JSON.stringify([SLICE_MODEL, want.printer, want.process, want.filament, want.bed_type]);
        if (got !== expected) throw new Error(got);
        if (!(await textOf(".sl-job-status")).startsWith("Queued")) throw new Error("re-slice not shown");
      },
    );
  });

  await step(run, page, "a dropped model cleared with the cache is not offered for slicing again", async () => {
    // Clear slice cache empties the staged inputs too, so the job's model is gone.
    const staged = "/Users/runner/Library/Application Support/com.bambumate.app/slice-inputs/6f1c2a7e-0d3b-4c5e-9a8f-1b2c3d4e5f60/bracket.stl";
    const job = (state) => sliceJob(12, state, { source_path: staged, model_name: "bracket.stl" });
    await withFixtures({ slicer_stage_model: staged, slicer_slice: job({ state: "queued", position: 0 }) }, async () => {
      const dropped = await page.evaluateHandle(() => {
        const dt = new DataTransfer();
        dt.items.add(new File(["solid bracket"], "bracket.stl", { type: "model/stl" }));
        return dt;
      });
      await page.dispatchEvent(".sl-drop", "drop", { dataTransfer: dropped });
      await page.waitForFunction(() => document.querySelector(".sl-model")?.innerText === "bracket.stl", null, { timeout: 5000 });
      const [stage] = (await callsOf("slicer_stage_model")).slice(-1);
      if (stage.args.fileName !== "bracket.stl" || !stage.args.dataBase64) throw new Error(JSON.stringify(stage.args));
      const saves = await settledCount("slicer_set_settings");
      await page.click(".sl-slice");
      await settledPast("slicer_set_settings", saves);
      if ((await callsOf("slicer_slice")).at(-1).args.modelPath !== staged) throw new Error("did not slice the staged model");
    });
    await withFixtures(
      {
        slicer_thumbnail: { __reject: FILES_CLEARED },
        slicer_open_in_bambu_studio: { __reject: FILES_CLEARED },
        slicer_model_exists: false,
      },
      async () => {
        await emitJob(job({ state: "done", result: sliceResult(), cached: false }));
        await page.waitForSelector('.sl-recent-row[data-job="12"].active', { timeout: 5000 });
        const gone = await textOf(".sl-reslice-gone");
        if (gone !== "The model was cleared too; drop it again.") throw new Error(gone);
        const checked = (await callsOf("slicer_model_exists")).map((c) => c.args.path);
        if (!checked.includes(staged)) throw new Error(`checked ${JSON.stringify(checked)}`);
        if ((await page.locator(".sl-reslice").count()) !== 0) throw new Error("offered Slice again for a cleared model");
      },
    );
  });

  await step(run, page, "a slice keeps auto-slice as it is saved now", async () => {
    // Settings turned auto-slice on after this page loaded its settings.
    const settings = await page.evaluate(() => window.__fixtures.slicer_get_settings);
    const on = { ...settings, saved: { ...settings.saved, auto_slice: true } };
    await withFixtures({ slicer_get_settings: on, slicer_slice: queued(13) }, async () => {
      await page.click(".sl-browse");
      await page.waitForFunction(() => document.querySelector(".sl-model")?.innerText === "cube.stl", null, { timeout: 5000 });
      const saves = await settledCount("slicer_set_settings");
      await page.click(".sl-slice");
      await settledPast("slicer_set_settings", saves);
      const { settings: saved } = (await callsOf("slicer_set_settings")).at(-1).args;
      if (saved.auto_slice !== true || saved.filament !== sliceJob(0, null).filament) throw new Error(JSON.stringify(saved));
    });
  });

  await step(run, page, "a cached result that arrives before Slice answers stays Done", async () => {
    // The backend publishes a job's events before `slicer_slice` returns; a
    // cache hit can be Done by then. The late Queued answer must not win.
    await withFixtures({ slicer_slice: { __hold: "cached", answer: queued(10) } }, async () => {
      const slices = await callCount("slicer_slice");
      const saves = await settledCount("slicer_set_settings");
      await page.click(".sl-slice");
      await calledPast("slicer_slice", slices);
      await emitJob(sliceJob(10, { state: "done", result: sliceResult(), cached: true }));
      await release("cached");
      await settledPast("slicer_set_settings", saves);
      await page.waitForSelector('.sl-recent-row[data-job="10"].active', { timeout: 5000 });
      const status = await page.locator(".sl-job-status").innerText();
      const row = await page.locator('.sl-recent-row[data-job="10"] .sl-recent-state').innerText();
      if (status !== "Done · from cache" || row !== "Done · from cache") throw new Error(`${status} / ${row}`);
      const want = sliceJob(0, null);
      const { settings } = (await callsOf("slicer_set_settings")).at(-1).args;
      const expected = { printer: want.printer, process: want.process, filament: want.filament, bed_type: want.bed_type, auto_slice: false };
      if (JSON.stringify(settings) !== JSON.stringify(expected)) throw new Error(JSON.stringify(settings));
    });
  });

  await step(run, page, "leaving the page while Slice is waiting on the backend is safe", async () => {
    const errors = run.errors.length;
    await withFixtures({ slicer_slice: { __hold: "away-slice", answer: queued(8) } }, async () => {
      await page.click(".sl-browse");
      await page.waitForFunction(() => document.querySelector(".sl-model")?.innerText === "cube.stl", null, { timeout: 5000 });
      const slices = await callCount("slicer_slice");
      const saves = await settledCount("slicer_set_settings");
      await page.click(".sl-slice");
      await calledPast("slicer_slice", slices);
      await page.click('a[href="/about"]');
      await page.waitForSelector(".about-page", { timeout: 15000 });
      await release("away-slice");
      // The task's last call: it ran to the end on a closed page.
      await settledPast("slicer_set_settings", saves);
    });
    if (run.errors.length !== errors) throw new Error(run.errors.slice(errors).join("; "));
    await page.click('a[href="/slice"]');
    await page.waitForSelector('.sl-recent-row[data-job="8"]', { timeout: 5000 });
  });

  await step(run, page, "leaving the page while settings load is safe", async () => {
    const errors = run.errors.length;
    await page.click('a[href="/about"]');
    await page.waitForSelector(".about-page", { timeout: 15000 });
    const settings = await page.evaluate(() => window.__fixtures.slicer_get_settings);
    const late = { __hold: "away-settings", answer: { ...settings, effective: { ...settings.effective, printer: null } } };
    await withFixtures({ slicer_get_settings: late }, async () => {
      const presets = await callCount("slicer_presets");
      const loads = await callCount("slicer_get_settings");
      const stls = await settledCount("list_received_stls");
      await page.click('a[href="/slice"]');
      await calledPast("slicer_get_settings", loads);
      await page.click('a[href="/about"]');
      await page.waitForSelector(".about-page", { timeout: 15000 });
      await release("away-settings");
      // The task's last call: it ran to the end on a closed page.
      await settledPast("list_received_stls", stls);
      const after = await callCount("slicer_presets");
      if (after !== presets) throw new Error(`${after - presets} presets loads for a closed page`);
    });
    if (run.errors.length !== errors) throw new Error(run.errors.slice(errors).join("; "));
  });

  await step(run, page, "a page whose settings failed to load saves none after a slice", async () => {
    // Starts on About (the previous step left the page).
    await withFixtures({ slicer_get_settings: { __reject: "The settings couldn't be read." }, slicer_slice: queued(14) }, async () => {
      const loads = await settledCount("slicer_get_settings");
      await page.click('a[href="/slice"]');
      await settledPast("slicer_get_settings", loads);
      await page.waitForFunction(() => document.querySelector("#sl-printer")?.value === "Bambu Lab H2C 0.4 nozzle", null, { timeout: 10000 });
      await page.click(".sl-browse");
      await page.waitForFunction(() => document.querySelector(".sl-model")?.innerText === "cube.stl", null, { timeout: 5000 });
      await page.waitForSelector(".sl-slice:not([disabled])", { timeout: 5000 });
      const saves = await callCount("slicer_set_settings");
      const gets = await callCount("slicer_get_settings");
      const slices = await settledCount("slicer_slice");
      await page.click(".sl-slice");
      await settledPast("slicer_slice", slices);
      await page.waitForSelector('.sl-recent-row[data-job="14"].active', { timeout: 5000 });
      await page.waitForSelector(".sl-slice:not([disabled])", { timeout: 5000 });
      if ((await callCount("slicer_set_settings")) !== saves) throw new Error("saved settings it never read");
      if ((await callCount("slicer_get_settings")) !== gets) throw new Error("read settings again after a failed load");
    });
  });

  await step(run, page, "the watch-folder list refreshes while the page is open", async () => {
    const file = (name) => ({ path: `/Users/runner/stl-inbox/${name}`, filename: name, received_at: "2026-10-01T12:00:00Z" });
    await withFixtures({ list_received_stls: [file("bracket.stl")] }, async () => {
      await page.waitForFunction(() => document.querySelectorAll(".sl-watch option").length === 2, null, { timeout: 15000 });
      await page.evaluate(() => (document.querySelectorAll(".sl-watch option")[1].__mark = 1));
      await setFixture("list_received_stls", [file("bracket.stl"), file("hinge.stl")]);
      await page.waitForFunction(() => document.querySelectorAll(".sl-watch option").length === 3, null, { timeout: 15000 });
      const kept = await page.evaluate(() => document.querySelectorAll(".sl-watch option")[1].__mark === 1);
      if (!kept) throw new Error("the list was rebuilt instead of keyed");
      await page.selectOption(".sl-watch", "/Users/runner/stl-inbox/hinge.stl");
      await page.waitForFunction(() => document.querySelector(".sl-model")?.innerText === "hinge.stl", null, { timeout: 5000 });
    });
    await page.waitForSelector(".sl-watch", { state: "detached", timeout: 15000 });
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

  const waitText = (sel, text) =>
    page.waitForFunction(([s, t]) => document.querySelector(s)?.innerText === t, [sel, text], { timeout: 5000 });
  // The `settings` argument of the set_settings calls made after `base` calls.
  const savedSince = async (base) => (await callsOf("slicer_set_settings")).slice(base).map((c) => c.args.settings);
  // Saved choices for every field, so the payload checks below compare real
  // values (a None goes over the wire as an absent key).
  const fixtureSettings = await page.evaluate(() => window.__fixtures.slicer_get_settings);
  const choices = { ...fixtureSettings.effective, auto_slice: false };

  await step(run, page, "turning auto-slice on saves the full settings", async () => {
    const withChoices = { ...fixtureSettings, saved: choices };
    // Bambu Studio has no printer selected, so auto-slice has nothing to use.
    const on = {
      ...fixtureSettings,
      saved: { ...choices, auto_slice: true },
      effective: { ...choices, auto_slice: true, printer: null },
    };
    await withFixtures({ slicer_get_settings: withChoices, slicer_set_settings: on }, async () => {
      // Reopen Settings so it loads the saved choices.
      await page.click('a[href="/about"]');
      await page.waitForSelector(".about-page", { timeout: 15000 });
      await page.click('a[href="/settings"]');
      await page.waitForSelector("#slice-auto:not([disabled])", { timeout: 10000 });
      const base = await callCount("slicer_set_settings");
      await page.check("#slice-auto");
      await calledPast("slicer_set_settings", base);
      const sent = await savedSince(base);
      const want = [{ ...choices, auto_slice: true }];
      if (!isDeepStrictEqual(sent, want)) throw new Error(JSON.stringify(sent));
      await waitText(".slice-auto-status", "New STLs will be sliced with your default printer, process and filament.");
      await waitText(".slice-auto-missing", "Pick a printer, process and filament on the Slice page first.");
      await page.waitForSelector("#slice-auto:not([disabled])", { timeout: 5000 });
      if (!(await page.isChecked("#slice-auto"))) throw new Error("toggle did not stay on");
    });
  });

  await step(run, page, "turning auto-slice off saves it and drops the hint", async () => {
    const base = await callCount("slicer_set_settings");
    await page.uncheck("#slice-auto");
    await calledPast("slicer_set_settings", base);
    const sent = await savedSince(base);
    const want = [{ ...choices, auto_slice: false }];
    if (!isDeepStrictEqual(sent, want)) throw new Error(JSON.stringify(sent));
    await waitText(".slice-auto-status", "Auto-slicing is off.");
    if ((await page.locator(".slice-auto-missing").count()) !== 0) throw new Error("hint still shown");
    if (await page.isChecked("#slice-auto")) throw new Error("toggle still on");
  });

  await step(run, page, "a refused auto-slice change rolls back and says why", async () => {
    const reason = "Couldn't write preferences.json";
    await withFixtures({ slicer_set_settings: { __reject: reason } }, async () => {
      const base = await callCount("slicer_set_settings");
      // click, not check: the rollback may land before check() re-reads the box.
      await page.click("#slice-auto");
      await calledPast("slicer_set_settings", base);
      await waitText(".slice-auto-status", `Failed to save: ${reason}`);
      await page.waitForSelector("#slice-auto:not([disabled])", { timeout: 5000 });
      if (await page.isChecked("#slice-auto")) throw new Error("toggle not rolled back");
    });
  });

  await step(run, page, "Clear slice cache reports bytes freed and refusals inline", async () => {
    await page.click(".slice-clear-cache");
    await waitText(".slice-cache-status", "Cleared 12.0 MB.");
    // While a slice is queued or running the backend refuses; the reason shows inline.
    const busy = "Finish or cancel the current slice first.";
    await withFixtures({ slicer_clear_cache: { __reject: busy } }, async () => {
      await page.click(".slice-clear-cache");
      await waitText(".slice-cache-status", busy);
    });
  });

  await step(run, page, "Clear slice cache is disabled until the backend answers", async () => {
    await withFixtures({ slicer_clear_cache: { __hold: "clear-cache", answer: 12582912 } }, async () => {
      await page.click(".slice-clear-cache");
      await page.waitForSelector(".slice-clear-cache[disabled]", { timeout: 5000 });
      // The previous run's message is gone while this one is in flight.
      if ((await page.locator(".slice-cache-status").count()) !== 0) throw new Error("old status still shown");
      await release("clear-cache");
      await waitText(".slice-cache-status", "Cleared 12.0 MB.");
      await page.waitForSelector(".slice-clear-cache:not([disabled])", { timeout: 5000 });
    });
  });

  // -- Settings → Printer -------------------------------------------------------
  const emitEvent = (name, payload) => page.evaluate(([n, p]) => window.__emit(n, p), [name, payload]);
  const emitPrinter = (state) => emitEvent("printer://connection", state);
  const resultText = (text) =>
    page.waitForFunction((t) => document.querySelector(".printer-result")?.innerText.trim() === t, text, {
      timeout: 5000,
    });
  // While the restarted live connection is still connecting, as right after a save.
  const CONNECTING_VIEW = { ...PRINTER_VIEW, connection: { state: "connecting" } };
  const TRUSTED_CONNECTING = "Trusted — connecting…";
  const CONNECTED_H2D = "Connected to H2D. Live status is on the Printer page.";

  await step(run, page, "printer settings find a printer on the network", async () => {
    await page.click(".printer-scan");
    await page.waitForSelector(".printer-found-item", { timeout: 10000 });
    await page.click(".printer-found-item");
    const ip = await page.inputValue("#printer-ip");
    const serial = await page.inputValue("#printer-serial");
    if (ip !== "192.168.1.20" || serial !== PRINTER_SERIAL) throw new Error(`form has ${ip} / ${serial}`);
    return `${ip} ${serial}`;
  });

  await step(run, page, "an untrusted certificate offers Trust this printer", async () => {
    await page.fill("#printer-code", "12345678");
    await page.click(".printer-test");
    await page.waitForSelector(".printer-trust", { timeout: 10000 });
    const fp = (await page.locator(".printer-fingerprint").innerText()).trim();
    if (fp !== PRINTER_FINGERPRINT) throw new Error(`fingerprint shows ${fp}`);
    const [t] = await callsOf("printer_test_connection");
    if (t.args.accessCode !== "12345678" || t.args.serial !== PRINTER_SERIAL) {
      throw new Error(JSON.stringify(t.args));
    }
    // textContent: the design system shows button labels in capitals.
    const label = ((await page.locator(".printer-trust-btn").textContent()) ?? "").trim();
    if (label !== "Trust this printer") throw new Error(`button reads "${label}"`);
  });

  await step(run, page, "trusting pins the fingerprint, saves and follows the live connection", async () => {
    // The save restarts the live connection; it reports Connecting first.
    await setFixture("printer_view", CONNECTING_VIEW);
    await emitPrinter({ state: "connecting" });
    await page.click(".printer-trust-btn");
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "printer_save"), null, {
      timeout: 5000,
    });
    const [save] = await callsOf("printer_save");
    if (save.args.pinnedFingerprint !== PRINTER_FINGERPRINT || save.args.accessCode !== "12345678") {
      throw new Error(JSON.stringify(save.args));
    }
    await resultText(TRUSTED_CONNECTING);
    if (await page.locator(".printer-trust").count()) throw new Error("the trust card is still shown");
    if ((await page.inputValue("#printer-code")) !== "") throw new Error("the access code is still in the field");
    // The live connection comes up: the line says so, from the event alone.
    await emitPrinter({ state: "connected" });
    await resultText(CONNECTED_H2D);
  });

  await step(run, page, "trust opens no second session: no test connection after the save", async () => {
    const calls = await callsOf("printer_test_connection");
    if (calls.length !== 1) throw new Error(`${calls.length} test calls`);
  });

  // The trust card must show the whole fingerprint and the serial it is for.
  await step(run, page, "a live untrusted certificate is trusted without retyping the code", async () => {
    const other = PRINTER_FINGERPRINT.replace(/^3A/, "4B");
    await emitPrinter({ state: "cert_untrusted", fingerprint: other });
    await page.waitForSelector(".printer-trust", { timeout: 5000 });
    const status = (await page.locator(".printer-live-status").innerText()).trim();
    if (status !== "The printer's certificate isn't from a Bambu CA that BambuMate knows.") {
      throw new Error(`live status reads "${status}"`);
    }
    const fp = (await page.locator(".printer-fingerprint").innerText()).trim();
    const serial = (await page.locator(".printer-trust-serial").innerText()).trim();
    if (fp !== other || serial !== PRINTER_SERIAL) throw new Error(`card shows ${serial} / ${fp}`);
    if (await page.locator(".printer-trust-warning").count()) {
      throw new Error("a printer that never verified against a Bambu CA shows the downgrade warning");
    }
    const before = (await callsOf("printer_save")).length;
    await page.click(".printer-trust-btn");
    await page.waitForFunction(
      (n) => window.__ipc.calls.filter((c) => c.cmd === "printer_save").length > n,
      before,
      { timeout: 5000 }
    );
    const save = (await callsOf("printer_save")).at(-1);
    if (
      save.args.pinnedFingerprint !== other ||
      save.args.accessCode ||
      save.args.ip !== "192.168.1.20" ||
      save.args.serial !== PRINTER_SERIAL
    ) {
      throw new Error(JSON.stringify(save.args));
    }
    // The stale rejection of the certificate just trusted is no new problem.
    await page.waitForSelector(".printer-trust", { state: "detached", timeout: 5000 });
    await resultText(TRUSTED_CONNECTING);
    if (await page.locator(".printer-live-status").count()) throw new Error("the stale rejection is still shown");
    await emitPrinter({ state: "connected" });
    await resultText(CONNECTED_H2D);
  });

  await step(run, page, "a new certificate from a printer that proved itself genuine needs a second click", async () => {
    const third = PRINTER_FINGERPRINT.replace(/^3A/, "5C");
    await emitEvent("printer://state", {
      ...PRINTER_VIEW,
      printer: { ...PRINTER_VIEW.printer, ca_verified: true },
      connection: { state: "cert_untrusted", fingerprint: third },
    });
    await page.waitForSelector(".printer-trust-warning", { timeout: 5000 });
    const warning = (await page.locator(".printer-trust-warning").innerText()).trim();
    const expected =
      "This printer previously proved it's a genuine Bambu printer. A new, unrecognised certificate could mean another device is impersonating it. Only trust it if you've just replaced or reset the printer.";
    if (warning !== expected) throw new Error(`warning reads "${warning}"`);
    const label = async () => ((await page.locator(".printer-trust-btn").textContent()) ?? "").trim();
    if ((await label()) !== "Trust this printer") throw new Error(`button reads "${await label()}"`);
    const before = (await callsOf("printer_save")).length;
    await page.click(".printer-trust-btn");
    await page.waitForFunction(
      () => document.querySelector(".printer-trust-btn")?.textContent.trim() === "Yes, trust this certificate",
      null,
      { timeout: 5000 }
    );
    await page.waitForTimeout(300);
    if ((await callsOf("printer_save")).length !== before) throw new Error("the first click saved already");
    await page.click(".printer-trust-btn");
    await page.waitForFunction(
      (n) => window.__ipc.calls.filter((c) => c.cmd === "printer_save").length > n,
      before,
      { timeout: 5000 }
    );
    const save = (await callsOf("printer_save")).at(-1).args;
    if (save.pinnedFingerprint !== third || save.accessCode || save.serial !== PRINTER_SERIAL) {
      throw new Error(JSON.stringify(save));
    }
    await resultText(TRUSTED_CONNECTING);
    await emitPrinter({ state: "connected" });
    await resultText(CONNECTED_H2D);
    await setFixture("printer_view", PRINTER_VIEW);
    await emitEvent("printer://state", PRINTER_VIEW);
  });

  await step(run, page, "a wrong serial is shown shortened and as plain text", async () => {
    const presented = `<b>X</b>${"A".repeat(200)}`;
    await emitPrinter({ state: "wrong_serial", presented });
    await page.waitForSelector(".printer-live-status", { timeout: 5000 });
    const text = await page.locator(".printer-live-status").innerText();
    const bold = await page.locator(".printer-live-status b").count();
    const shown = `<b>X</b>${"A".repeat(64 - 8)}`;
    if (bold !== 0 || !text.includes(`serial ${shown}.`)) throw new Error(`status reads "${text.slice(0, 120)}"`);
    await emitPrinter({ state: "connected" });
    await page.waitForSelector(".printer-live-status", { state: "detached", timeout: 5000 });
  });

  await step(run, page, "a serial claimed by two devices is flagged", async () => {
    await setFixture("printer_discover", [
      { ip: "192.168.1.20", serial: PRINTER_SERIAL, name: "Workshop H2D", model: "H2D", conflict: true },
      { ip: "192.168.1.31", serial: "01P00A000000002", name: "", model: "P1S", conflict: false },
    ]);
    await page.click(".printer-scan");
    await page.waitForFunction(() => document.querySelectorAll(".printer-found-item").length === 2, null, {
      timeout: 10000,
    });
    const warnings = await page.locator(".printer-conflict").count();
    const text = (await page.locator(".printer-conflict").first().innerText()).trim();
    const expected =
      "Two devices on your network claim this serial — check the IP on the printer screen before connecting.";
    if (warnings !== 1 || text !== expected) throw new Error(`${warnings} warnings: "${text}"`);
    const serials = await page.locator(".printer-found-serial code").allInnerTexts();
    if (serials.join() !== `${PRINTER_SERIAL},01P00A000000002`) throw new Error(`serials ${serials}`);
  });

  await step(run, page, "a refused save shows the backend's reason inline", async () => {
    const reason = "Enter the access code to connect to a different printer or certificate.";
    await setFixture("printer_save", { __reject: reason });
    await page.fill("#printer-ip", "192.168.1.31");
    await page.click(".printer-save");
    await page.waitForFunction(
      (r) => document.querySelector(".printer-result")?.innerText.trim() === r,
      reason,
      { timeout: 5000 }
    );
  });

  await step(run, page, "editing the form between Test and Trust still pins the tested printer", async () => {
    await setFixture("printer_save", FIXTURES.printer_save);
    await setFixture("printer_test_connection", FIXTURES.printer_test_connection);
    await page.fill("#printer-ip", "192.168.1.20");
    await page.fill("#printer-code", "12345678");
    const tests = (await callsOf("printer_test_connection")).length;
    await page.click(".printer-test");
    await page.waitForFunction(
      (n) => window.__ipc.calls.filter((c) => c.cmd === "printer_test_connection").length > n,
      tests,
      { timeout: 5000 }
    );
    await page.waitForSelector(".printer-trust", { timeout: 5000 });
    await page.fill("#printer-ip", "10.0.0.66");
    await page.fill("#printer-serial", "EVIL00000000001");
    await setFixture("printer_test_connection", { connection: { state: "connected" }, got_report: true, model: "H2D" });
    const saves = (await callsOf("printer_save")).length;
    await page.click(".printer-trust-btn");
    await page.waitForFunction(
      (n) => window.__ipc.calls.filter((c) => c.cmd === "printer_save").length > n,
      saves,
      { timeout: 5000 }
    );
    const save = (await callsOf("printer_save")).at(-1).args;
    if (
      save.ip !== "192.168.1.20" ||
      save.serial !== PRINTER_SERIAL ||
      save.pinnedFingerprint !== PRINTER_FINGERPRINT ||
      save.accessCode !== "12345678"
    ) {
      throw new Error(JSON.stringify(save));
    }
    const form = [await page.inputValue("#printer-ip"), await page.inputValue("#printer-serial")];
    if (form.join() !== `192.168.1.20,${PRINTER_SERIAL}`) throw new Error(`form has ${form}`);
    // From the live connection (connected in the shared view), not a test.
    await resultText(CONNECTED_H2D);
  });

  await step(run, page, "editing the IP away from the pinned printer drops the pin", async () => {
    await page.fill("#printer-ip", "192.168.1.99");
    await page.fill("#printer-ip", "192.168.1.20");
    const tests = (await callsOf("printer_test_connection")).length;
    await page.click(".printer-test");
    await page.waitForFunction(
      (n) => window.__ipc.calls.filter((c) => c.cmd === "printer_test_connection").length > n,
      tests,
      { timeout: 5000 }
    );
    const t = (await callsOf("printer_test_connection")).at(-1).args;
    if (t.pinnedFingerprint) throw new Error(`still sent pin ${t.pinnedFingerprint}`);
  });

  await step(run, page, "Remove calls printer_remove and clears the form", async () => {
    await page.waitForSelector(".printer-remove:not([disabled])", { timeout: 5000 });
    await page.click(".printer-remove");
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "printer_remove"), null, {
      timeout: 5000,
    });
    await page.waitForFunction(
      () => document.querySelector(".printer-result")?.innerText.trim() === "Printer removed.",
      null,
      { timeout: 5000 }
    );
    const left = [await page.inputValue("#printer-ip"), await page.inputValue("#printer-serial")];
    if (left.some(Boolean)) throw new Error(`form still has ${left}`);
    if (await page.locator(".printer-remove").count()) throw new Error("Remove is still shown");
  });

  await step(run, page, "only Test connection and Save carry the access code", async () => {
    const found = await page.evaluate(() => {
      const allowed = ["printer_test_connection", "printer_save"];
      const withCode = window.__ipc.calls.filter((c) => c.args && c.args.accessCode);
      const leaked = window.__ipc.calls
        .filter((c) => !allowed.includes(c.cmd))
        .filter((c) => (c.args && "accessCode" in c.args) || JSON.stringify(c.args ?? null).includes("12345678"))
        .map((c) => c.cmd);
      return { sent: withCode.length, leaked };
    });
    if (found.leaked.length) throw new Error(`access code sent with ${found.leaked}`);
    if (found.sent === 0) throw new Error("no command carried the access code at all");
    return `${found.sent} calls carried it`;
  });

  // Trust reads the section's signals after the save's await; leaving
  // Settings in between disposes them, which must not panic.
  await step(run, page, "leaving Settings while Trust is saving doesn't panic", async () => {
    await setFixture("printer_test_connection", FIXTURES.printer_test_connection);
    await page.fill("#printer-ip", "192.168.1.20");
    await page.fill("#printer-serial", PRINTER_SERIAL);
    await page.fill("#printer-code", "12345678");
    await page.click(".printer-test");
    await page.waitForSelector(".printer-trust", { timeout: 5000 });
    await setFixture("printer_save", { __delay: 400, answer: FIXTURES.printer_save });
    const errors = run.errors.length;
    await page.click(".printer-trust-btn");
    await page.click('a[href="/about"]');
    await page.waitForSelector(".about-page", { timeout: 15000 });
    await page.waitForTimeout(800);
    if (run.errors.length !== errors) throw new Error(run.errors.slice(errors).join("; "));
    await page.click('a[href="/settings"]');
    await page.waitForSelector(".printer-settings", { timeout: 15000 });
  });

  await step(run, page, "Reset for clean install empties Settings → Printer", async () => {
    // A saved printer, shown when the section mounts.
    await setFixture("printer_get_config", FIXTURES.printer_save);
    await page.click('a[href="/about"]');
    await page.waitForSelector(".about-page", { timeout: 15000 });
    await page.click('a[href="/settings"]');
    await page.waitForFunction(() => document.querySelector("#printer-ip")?.value === "192.168.1.20", null, {
      timeout: 15000,
    });
    await page.waitForSelector(".printer-remove", { timeout: 5000 });
    // The reset removes the printer in the backend.
    await setFixture("printer_get_config", null);
    await setFixture("printer_view", PRINTER_UNCONFIGURED);
    const gets = (await callsOf("printer_get_config")).length;
    await page.click("text=Reset for Clean Installation");
    await page.click("text=Yes, Reset Everything");
    await page.waitForFunction(
      (n) => window.__ipc.calls.filter((c) => c.cmd === "printer_get_config").length > n,
      gets,
      { timeout: 5000 }
    );
    await page.waitForFunction(
      () => document.querySelector("#printer-ip")?.value === "" && !document.querySelector(".printer-remove"),
      null,
      { timeout: 5000 }
    );
    if (await page.inputValue("#printer-serial")) throw new Error("the serial is still in the form");
  });

  // Put the canned answers back so later steps start from the defaults.
  for (const cmd of Object.keys(FIXTURES).filter((k) => k.startsWith("printer_"))) {
    await setFixture(cmd, FIXTURES[cmd]);
  }

  // Regression for the Task 5 fix that made .nav-lock absolutely positioned
  // over the icon's corner instead of flowing after the (opacity:0, but still
  // full-width) label -- otherwise it would sit past the edge of the 64px
  // collapsed rail. Turning AI off here, after the two prior steps that need
  // it on, keeps this order-independent of the rest of the flow: nothing
  // after this point revisits Print Analysis or the AI-gated Settings UI.
  await step(run, page, "AI-off lock is visible on the collapsed rail", async () => {
    // Earlier steps click rail links; start from a rail at rest, not one
    // still animating closed after the pointer left it.
    await page.mouse.move(900, 400);
    await page.waitForFunction(() => document.querySelector("nav.sidebar").getBoundingClientRect().width <= 65, null, { timeout: 3000 });
    await page.locator(".settings-page .wizard-mode-card", { hasText: "Manufacturer Specs Only" }).click();
    await page.waitForFunction(
      () => window.__ipc.calls.some(
        (c) => c.cmd === "set_preference" && c.args?.key === "filament_search_use_ai" && c.args?.value === "false"
      ),
      null,
      { timeout: 5000 }
    );
    const lock = page.locator('nav.sidebar a[href="/analysis"] .nav-lock');
    await lock.waitFor({ state: "visible", timeout: 5000 });
    const rail = await page.locator("nav.sidebar").boundingBox();
    const box = await lock.boundingBox();
    if (!box || !rail) throw new Error("lock or rail has no box");
    if (rail.width > 65) throw new Error(`rail not collapsed: ${rail.width}px`);
    if (box.x + box.width > rail.x + rail.width + 1) {
      throw new Error(`lock (x=${box.x}, w=${box.width}) extends past the ${Math.round(rail.width)}px rail`);
    }
    return `lock at x=${Math.round(box.x)} within ${Math.round(rail.width)}px rail`;
  });

  await page.screenshot({ path: `flow-${engine}-settings.png`, fullPage: false });

  {
    const inbox = [{ path: STL_INBOX_FILE, filename: "bracket.stl", received_at: "2026-10-01T12:00:00Z" }];
    const auto = { origin: "auto", source_path: STL_INBOX_FILE, model_name: "bracket.stl" };
    await withFixtures({ list_received_stls: inbox }, async () => {
      await step(run, page, "a failed auto-slice shows Slice failed in the STL list", async () => {
        // The indicator polls every 5 s.
        await page.waitForSelector(".stl-badge", { timeout: 15000 });
        await page.click(".stl-badge");
        await page.waitForSelector(".stl-item", { timeout: 5000 });
        const error = { kind: "slicer", message: "Bambu Studio couldn't slice this model: x" };
        await emitJob(sliceJob(8, { state: "failed", error }, auto));
        await waitText(".stl-slice-state", "Slice failed");
      });

      await step(run, page, "the STL list shows each file's auto-slice state", async () => {
        await emitJob(sliceJob(9, { state: "running", progress: null }, auto));
        await waitText(".stl-slice-state", "Slicing…");
        await emitJob(done(9, sliceResult(), auto));
        await waitText(".stl-slice-state", "14m · 3.69 g");
        await page.click(".stl-slice-state");
        await page.waitForFunction(() => location.pathname === "/slice" && location.search === "?job=9", null, { timeout: 5000 });
        await waitText(".sl-job-model", "bracket.stl");
      });
    });
  }

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

  await step(run, page, "preset sync check offers Review and repair", async () => {
    const button = page.locator('.diagnostics-action[data-action="repair_preset_sync"]');
    if ((await button.count()) !== 1) throw new Error("no repair action button");
    // textContent is the label as written; the design system may display it
    // in capitals via text-transform, which innerText would report.
    const label = ((await button.textContent()) ?? "").trim();
    if (label !== "Review and repair") throw new Error(`button reads "${label}"`);
    const row = page.locator(".diagnostics-row-warn", { has: button });
    const detail = (await row.locator(".diagnostics-detail").innerText()).trim();
    if (detail !== "2 presets won't sync to Bambu Cloud") throw new Error(`detail reads "${detail}"`);
    return detail;
  });

  await step(run, page, "repair panel lists candidates, confirmed ones ticked", async () => {
    await page.click('.diagnostics-action[data-action="repair_preset_sync"]');
    await page.waitForSelector(".preset-sync-panel .preset-sync-row", { timeout: 15000 });
    const ticked = await page
      .locator(".preset-sync-row input[type=checkbox]")
      .evaluateAll((els) => els.map((e) => e.checked));
    if (JSON.stringify(ticked) !== "[true,false]") throw new Error(`ticked ${JSON.stringify(ticked)}`);
    const note = (await page.locator(".preset-sync-note").innerText()).trim();
    const expected = "Might already be synced — only tick if it's missing from your printer";
    if (note !== expected) throw new Error(`note reads "${note}"`);
    return `${ticked.length} rows`;
  });

  await step(run, page, "Repair selected sends the ticked paths and reports", async () => {
    await page.click(".preset-sync-repair");
    await page.waitForSelector(".preset-sync-result", { timeout: 15000 });
    const sent = await page.evaluate(() =>
      window.__ipc.calls.filter((c) => c.cmd === "repair_preset_sync").map((c) => c.args.paths)
    );
    if (JSON.stringify(sent) !== JSON.stringify([[UNSYNCED_CONFIRMED_PATH]])) {
      throw new Error(`sent ${JSON.stringify(sent)}`);
    }
    const msg = (await page.locator(".preset-sync-result").innerText()).trim();
    const expected = "Repaired 1 preset. Open Bambu Studio while signed in to upload them.";
    if (msg !== expected) throw new Error(`result reads "${msg}"`);
    return msg;
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

  // -- printer page -------------------------------------------------------------
  const slotCard = (label) => page.locator(`.pr-slot[data-label="${label}"]`);
  const badgeOf = async (label) => (await slotCard(label).locator(".pr-status").innerText()).trim();
  const presetPath = FIXTURES.list_profiles[0].path;

  await step(run, page, "printer page points to Settings when no printer is set up", async () => {
    await setFixture("printer_view", PRINTER_UNCONFIGURED);
    await page.click('a[href="/printer"]');
    await page.waitForSelector(".printer-page .pr-empty", { timeout: 15000 });
    const link = await page.locator(".pr-setup-link").innerText();
    if (!link.includes("Settings → Printer")) throw new Error(`link reads "${link}"`);
    if ((await page.locator(".printer-dot").count()) !== 0) throw new Error("rail dot shown with no printer");
  });

  await step(run, page, "printer hero shows the current print", async () => {
    await setFixture("printer_view", PRINTER_VIEW);
    await page.click('a[href="/"]');
    await page.click('a[href="/printer"]');
    await page.waitForSelector(".printer-page.nd .pr-hero", { timeout: 15000 });
    await page.waitForFunction(() => document.querySelector(".pr-hero-percent")?.innerText === "6%", null, {
      timeout: 5000,
    });
    // Lower-cased: .nd-label uppercases its text, and innerText reports what is rendered.
    const hero = (await page.locator(".pr-hero").innerText()).replace(/\s+/g, " ").toLowerCase();
    for (const want of ["running", "1 / 200", "9 h 09 m", "t-pose - slim h2d dual ams riser", "right nozzle", "left nozzle", "245 / 245 °c", "70 / 70 °c"]) {
      if (!hero.includes(want)) throw new Error(`hero lacks "${want}": ${hero}`);
    }
  });

  await step(run, page, "rail dot shows the connection", async () => {
    const dot = page.locator('.printer-dot[data-state="connected"]');
    if ((await dot.count()) !== 1) throw new Error("no connected dot on the rail");
    const bg = await dot.evaluate((el) => getComputedStyle(el).backgroundColor);
    if (bg === "rgba(0, 0, 0, 0)" || bg === "transparent") throw new Error("dot has no colour");
    const [role, label] = await dot.evaluate((el) => [el.getAttribute("role"), el.getAttribute("aria-label")]);
    if (role !== "img" || label !== "Printer: connected") throw new Error(`dot role="${role}" aria-label="${label}"`);
    return bg;
  });

  await step(run, page, "AMS cards show every slot with its status", async () => {
    const cards = await page.locator(".pr-slot").count();
    if (cards !== 10) throw new Error(`expected 10 slot cards, got ${cards}`);
    const rows = await page.locator(".pr-ams-row").count();
    if (rows !== 3) throw new Error(`expected 3 rows (A, B, External), got ${rows}`);
    const expect = { A2: "✓ Bambu spool", A3: "Not set", B2: "✓ Set", B4: "Empty", "Ext-L": "Not set" };
    for (const [label, badge] of Object.entries(expect)) {
      const got = await badgeOf(label);
      if (got !== badge) throw new Error(`${label} shows "${got}", expected "${badge}"`);
    }
    if ((await slotCard("A2").locator(".pr-rfid").count()) !== 1) throw new Error("A2 lacks its RFID badge");
    const meta = await page.locator(".pr-ams-meta").first().innerText();
    if (!meta.includes("Humidity 21%")) throw new Error(`AMS A meta: ${meta}`);
  });

  await step(run, page, "errors show their text or the wiki link", async () => {
    const errors = await page.locator(".pr-error").count();
    if (errors !== 2) throw new Error(`expected 2 errors, got ${errors}`);
    const text = await page.locator(".pr-error-text").first().innerText();
    if (!text.includes("heatbed")) throw new Error(`error text: ${text}`);
    const href = await page.locator(".pr-error-link").getAttribute("href");
    if (!href.startsWith("https://wiki.bambulab.com/")) throw new Error(`wiki link: ${href}`);
  });

  await page.screenshot({ path: `flow-${engine}-printer.png`, fullPage: true });

  await step(run, page, "assigning a preset shows the steps to set it on the printer", async () => {
    await slotCard("A3").locator(".pr-slot-main").click();
    await page.waitForSelector(".pr-picker .pr-picker-item", { timeout: 10000 });
    await page.fill(".pr-picker-search", "PolyLite");
    await page.locator(".pr-picker-item", { hasText: "PolyLite" }).first().click();
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "printer_assign_slot"), null, {
      timeout: 5000,
    });
    const calls = await callsOf("printer_assign_slot");
    if (calls.length !== 1) throw new Error(`${calls.length} assign calls`);
    if (!sameArgs(calls[0].args, { amsId: 0, trayId: 2, presetPath })) throw new Error(JSON.stringify(calls[0].args));
    await page.waitForSelector(".pr-picker", { state: "detached", timeout: 5000 });
    const badge = await badgeOf("A3");
    if (badge !== "Set on printer") throw new Error(`A3 shows "${badge}"`);
    const steps = (await slotCard("A3").locator(".pr-steps").innerText()).replace(/\s+/g, " ");
    for (const want of [
      "On the printer: Filament → A3 → choose Polymaker PolyLite PLA @BBL X1C 0.4 nozzle.",
      "Or in Bambu Studio: Device → AMS → A3 → Polymaker PolyLite PLA @BBL X1C 0.4 nozzle.",
      "It must sync to Bambu Cloud first — open Bambu Studio while signed in.",
    ]) {
      if (!steps.includes(want)) throw new Error(`steps lack "${want}": ${steps}`);
    }
    if ((await slotCard("A3").locator(".pr-steps em").count()) !== 2) throw new Error("preset names not in <em>");
  });

  await step(run, page, "the card flips to ✓ Set when the printer reports the preset", async () => {
    // The card updates in place: someone focused on it keeps their place.
    await page.evaluate(() => {
      document.querySelector('.pr-slot[data-label="A3"] .pr-slot-main').__keep = true;
    });
    const matched = a3Matched();
    await emitEvent("printer://state", matched);
    await page.waitForFunction(
      () => document.querySelector('.pr-slot[data-label="A3"] .pr-status')?.innerText.trim() === "✓ Set",
      null,
      { timeout: 5000 }
    );
    if ((await slotCard("A3").locator(".pr-steps").count()) !== 0) throw new Error("steps still shown");
    const kept = await page.evaluate(() => document.querySelector('.pr-slot[data-label="A3"] .pr-slot-main').__keep);
    if (kept !== true) throw new Error("the A3 card was rebuilt when its status changed");
  });

  await step(run, page, "losing the printer greys the page and says why", async () => {
    await emitEvent("printer://connection", { state: "unreachable" });
    await page.waitForSelector(".pr-body.pr-stale", { timeout: 5000 });
    const notice = (await page.locator(".pr-notice").innerText()).trim();
    if (notice !== "Can't reach the printer at 192.168.1.20.") throw new Error(`notice: ${notice}`);
    if ((await page.locator('.printer-dot[data-state="error"]').count()) !== 1) throw new Error("dot not in error");
    await emitEvent("printer://connection", { state: "connected" });
    await page.waitForSelector(".pr-body:not(.pr-stale)", { timeout: 5000 });
  });

  // The backend's status rules put a spool's RFID report above an assignment
  // the user made before swapping it in.
  await step(run, page, "an RFID spool's report beats a stale assignment", async () => {
    await emitEvent("printer://state", withSlot("A2", { assigned_preset: "Old Acme PLA", assigned_filament_id: "P00000001" }));
    await page.waitForSelector('.pr-slot[data-label="A2"] .pr-prev', { timeout: 5000 });
    const preset = (await slotCard("A2").locator(".pr-preset").innerText()).trim();
    if (preset !== "PLA Basic") throw new Error(`A2 preset reads "${preset}"`);
    const prev = (await slotCard("A2").locator(".pr-prev").innerText()).trim();
    if (prev !== "Previously assigned: Old Acme PLA") throw new Error(`A2 history reads "${prev}"`);
    const badge = await badgeOf("A2");
    if (badge !== "✓ Bambu spool") throw new Error(`A2 shows "${badge}"`);
    if ((await slotCard("A2").locator(".pr-steps, .pr-no-id").count()) !== 0) throw new Error("A2 shows steps");
  });

  await step(run, page, "a preset with no filament id says to confirm by eye", async () => {
    await emitEvent(
      "printer://state",
      withSlot("A3", { ...A3_ASSIGNED, assigned_filament_id: null, needs_cloud_sync: false, preset_has_no_id: true })
    );
    await page.waitForSelector('.pr-slot[data-label="A3"] .pr-no-id', { timeout: 5000 });
    const note = (await slotCard("A3").locator(".pr-no-id").innerText()).replace(/\s+/g, " ").trim();
    const want =
      "BambuMate can't check this slot: Polymaker PolyLite PLA @BBL X1C 0.4 nozzle has no filament id. Set it on the printer and confirm by eye.";
    if (note !== want) throw new Error(`note reads "${note}"`);
    if ((await slotCard("A3").locator(".pr-no-id em").innerText()) !== A3_ASSIGNED.assigned_preset) {
      throw new Error("preset name not in <em>");
    }
    if ((await slotCard("A3").locator(".pr-steps").count()) !== 0) throw new Error("steps shown as well");
    const badge = await badgeOf("A3");
    if (badge !== "Set on printer") throw new Error(`A3 shows "${badge}"`);
  });

  await step(run, page, "clearing an assignment calls printer_clear_slot", async () => {
    await slotCard("A3").locator(".pr-slot-main").click();
    await page.click(".pr-picker-clear", { timeout: 5000 });
    await page.waitForSelector(".pr-picker", { state: "detached", timeout: 5000 });
    const calls = await callsOf("printer_clear_slot");
    if (calls.length !== 1) throw new Error(`${calls.length} clear calls`);
    if (!sameArgs(calls[0].args, { amsId: 0, trayId: 2 })) throw new Error(JSON.stringify(calls[0].args));
    const badge = await badgeOf("A3");
    if (badge !== "Not set") throw new Error(`A3 shows "${badge}"`);
  });

  await step(run, page, "a refused slot shows the backend's reason inline", async () => {
    await setFixture("printer_assign_slot", { __reject: "That isn't a slot on this printer." });
    await slotCard("Ext-L").locator(".pr-slot-main").click();
    await page.waitForSelector(".pr-picker .pr-picker-item", { timeout: 10000 });
    await page.locator(".pr-picker-item").first().click();
    await page.waitForSelector(".pr-picker-error", { timeout: 5000 });
    const text = (await page.locator(".pr-picker-error").innerText()).trim();
    if (text !== "That isn't a slot on this printer.") throw new Error(`error reads "${text}"`);
    const calls = await callsOf("printer_assign_slot");
    const last = calls.at(-1);
    if (!sameArgs(last.args, { amsId: 255, trayId: 254, presetPath })) throw new Error(JSON.stringify(last.args));
    if (!(await page.locator(".pr-picker-item").first().isEnabled())) throw new Error("picker stayed busy");
    await page.click(".pr-picker-cancel");
    await page.waitForSelector(".pr-picker", { state: "detached", timeout: 5000 });
    await setFixture("printer_assign_slot", FIXTURES.printer_assign_slot);
  });

  // The backend doesn't retry these, so the page points at Settings → Printer.
  await step(run, page, "a connection that needs the user links to Settings → Printer", async () => {
    await emitEvent("printer://connection", { state: "cert_untrusted", fingerprint: PRINTER_FINGERPRINT });
    await page.waitForSelector(".pr-notice .pr-settings-link", { timeout: 5000 });
    const notice = (await page.locator(".pr-notice").innerText()).replace(/\s+/g, " ");
    if (!notice.includes("Trust it in Settings → Printer.")) throw new Error(`notice: ${notice}`);
    const href = await page.locator(".pr-settings-link").getAttribute("href");
    if (href !== "/settings#printer") throw new Error(`link: ${href}`);

    // The presented serial comes from whoever answered: text only, cut to 64.
    await emitEvent("printer://connection", { state: "wrong_serial", presented: `<b>X</b>${"A".repeat(200)}` });
    await page.waitForFunction(() => document.querySelector(".pr-notice")?.innerText.includes("reports serial"), null, {
      timeout: 5000,
    });
    const wrong = await page.locator(".pr-notice").innerText();
    if (!wrong.includes(`<b>X</b>${"A".repeat(56)}`) || wrong.includes("A".repeat(57))) {
      throw new Error(`serial not shown as 64 characters of text: ${wrong}`);
    }
    if ((await page.locator(".pr-notice b").count()) !== 0) throw new Error("presented serial rendered as HTML");
    if ((await page.locator(".pr-settings-link").count()) !== 1) throw new Error("no Settings link");

    // A state the backend retries has nothing for the user to fix.
    await emitEvent("printer://connection", { state: "unreachable" });
    await page.waitForSelector(".pr-settings-link", { state: "detached", timeout: 5000 });

    await emitEvent("printer://connection", { state: "auth_failed" });
    await page.click(".pr-settings-link");
    await page.waitForSelector(".printer-settings", { timeout: 15000 });
    await emitEvent("printer://connection", { state: "connected" });
    await page.click('a[href="/printer"]');
    await page.waitForSelector(".pr-body:not(.pr-stale)", { timeout: 15000 });
  });

  await step(run, page, "a connected printer with no report yet says it is waiting", async () => {
    await emitEvent("printer://state", { ...PRINTER_VIEW, state: null, slots: [], errors: [] });
    await page.waitForSelector(".pr-waiting", { timeout: 5000 });
    const text = (await page.locator(".pr-waiting").innerText()).trim();
    if (text !== "Waiting for the first status report…") throw new Error(`reads "${text}"`);
    if ((await page.locator(".pr-slot, .pr-hero").count()) !== 0) throw new Error("empty cards shown");
    await emitEvent("printer://state", PRINTER_VIEW);
    await page.waitForSelector(".pr-hero", { timeout: 5000 });
  });

  // A state event arrives about twice a second during a print. One that only
  // changes temperatures must leave the cards and errors (and anything the
  // user is clicking or typing into) alone.
  const warmer = (nozzle) => ({
    ...PRINTER_VIEW,
    state: {
      ...PRINTER_VIEW.state,
      nozzles: [{ ...PRINTER_VIEW.state.nozzles[0], temp: nozzle }, PRINTER_VIEW.state.nozzles[1]],
    },
  });
  const heroShows = (text) =>
    page.waitForFunction((t) => document.querySelector(".pr-hero")?.innerText.includes(t), text, { timeout: 5000 });

  await step(run, page, "a temperature-only state event keeps the card and error nodes", async () => {
    await page.evaluate(() => {
      document.querySelector('.pr-slot[data-label="A1"]').__keep = true;
      document.querySelector(".pr-error").__keep = true;
    });
    await emitEvent("printer://state", warmer(246.0));
    await heroShows("246 / 245");
    const kept = await page.evaluate(() => [
      document.querySelector('.pr-slot[data-label="A1"]')?.__keep === true,
      document.querySelector(".pr-error")?.__keep === true,
    ]);
    if (!kept[0]) throw new Error("the A1 card was rebuilt");
    if (!kept[1]) throw new Error("the error row was rebuilt");
  });

  await step(run, page, "the picker keeps its search text and focus through a state event", async () => {
    await slotCard("A3").locator(".pr-slot-main").click();
    await page.waitForSelector(".pr-picker .pr-picker-item", { timeout: 10000 });
    if ((await page.locator(".pr-picker").getAttribute("aria-modal")) !== "true") throw new Error("not aria-modal");
    await page.waitForFunction(() => document.activeElement?.classList.contains("pr-picker-search"), null, {
      timeout: 5000,
    });
    await page.keyboard.type("Poly");
    await emitEvent("printer://state", warmer(247.0));
    await heroShows("247 / 245");
    const value = await page.inputValue(".pr-picker-search");
    if (value !== "Poly") throw new Error(`search reads "${value}"`);
    const focused = await page.evaluate(() => document.activeElement?.classList.contains("pr-picker-search"));
    if (!focused) throw new Error("the search box lost focus");
    await page.keyboard.press("Escape");
    await page.waitForSelector(".pr-picker", { state: "detached", timeout: 5000 });
    await page.waitForFunction(() => document.activeElement?.closest(".pr-slot")?.dataset.label === "A3", null, {
      timeout: 5000,
    });
  });

  await step(run, page, "the picker says when presets are loading, missing or too many", async () => {
    const open = async () => {
      await slotCard("A3").locator(".pr-slot-main").click();
      await page.waitForSelector(".pr-picker", { timeout: 5000 });
    };
    const close = async () => {
      await page.keyboard.press("Escape");
      await page.waitForSelector(".pr-picker", { state: "detached", timeout: 5000 });
    };
    const status = async () => (await page.locator(".pr-picker-status").innerText()).trim();

    await setFixture("list_profiles", { __delay: 400, answer: FIXTURES.list_profiles });
    await open();
    await page.waitForSelector(".pr-picker-status", { timeout: 2000 });
    if ((await status()) !== "Loading presets…") throw new Error(`while loading: "${await status()}"`);
    await page.waitForSelector(".pr-picker-item", { timeout: 5000 });
    await page.waitForSelector(".pr-picker-status", { state: "detached", timeout: 5000 });
    await close();

    const reason = "Bambu Studio's preset folder isn't readable.";
    await setFixture("list_profiles", { __reject: reason });
    await open();
    await page.waitForSelector(".pr-picker-error", { timeout: 5000 });
    const error = await page.locator(".pr-picker-error").innerText();
    if (!error.includes(reason)) throw new Error(`error reads "${error}"`);
    if ((await page.locator(".pr-picker-item").count()) !== FIXTURES.list_system_profiles.length) {
      throw new Error("Bambu presets not listed after the user list failed");
    }
    await close();

    await setFixture("list_profiles", FIXTURES.list_profiles);
    const many = Array.from({ length: 70 }, (_, i) => ({
      name: `Test Preset ${String(i).padStart(2, "0")}`,
      filament_type: "PLA",
      filament_id: null,
      path: `/presets/Test Preset ${i}.json`,
      is_user_profile: false,
    }));
    await setFixture("list_system_profiles", many);
    await open();
    await page.waitForSelector(".pr-picker-item", { timeout: 5000 });
    await page.waitForSelector(".pr-picker-status", { timeout: 5000 });
    if ((await status()) !== "Refine your search to see more.") throw new Error(`with 70: "${await status()}"`);
    await page.fill(".pr-picker-search", "zzzz");
    await page.waitForFunction(() => document.querySelector(".pr-picker-status")?.innerText.trim() === "No presets match.", null, {
      timeout: 5000,
    });
    await page.fill(".pr-picker-search", "Test Preset 0");
    await page.waitForSelector(".pr-picker-status", { state: "detached", timeout: 5000 });
    await close();
    await setFixture("list_system_profiles", FIXTURES.list_system_profiles);
  });

  await step(run, page, "printer-supplied names are shortened and shown as plain text", async () => {
    const odd = {
      ...withSlot("A2", {
        tray: { ...PRINTER_VIEW.slots[1].tray, tray_sub_brands: `<u>S</u>${"S".repeat(100)}` },
      }),
      printer: { ...PRINTER_VIEW.printer, name: `<b>N</b>${"N".repeat(100)}` },
      state: { ...PRINTER_VIEW.state, subtask_name: `<i>F</i>‮${"F".repeat(200)}` },
    };
    await emitEvent("printer://state", odd);
    await page.waitForFunction(() => document.querySelector(".pr-file dd")?.innerText.startsWith("<i>F</i>"), null, {
      timeout: 5000,
    });
    const file = await page.locator(".pr-file dd").innerText();
    if ([...file].length !== 120 || file.includes("‮")) throw new Error(`file: ${[...file].length} chars`);
    const ident = await page.locator(".pr-ident").innerText();
    if (!ident.includes(`<b>N</b>${"N".repeat(56)} · `)) throw new Error(`ident: ${ident}`);
    const preset = await slotCard("A2").locator(".pr-preset").innerText();
    if (preset !== `<u>S</u>${"S".repeat(56)}`) throw new Error(`A2 preset: ${preset}`);
    if ((await page.locator(".printer-page b, .printer-page i, .printer-page u").count()) !== 0) {
      throw new Error("a printer-supplied name was rendered as HTML");
    }
    await emitEvent("printer://state", PRINTER_VIEW);
    await page.waitForFunction(() => document.querySelector(".pr-ident")?.innerText.startsWith("Workshop H2D"), null, {
      timeout: 5000,
    });
  });

  // The agent's bm_navigate can leave the page while the picker waits on the
  // backend; the late answer must not touch the page's disposed signals.
  await step(run, page, "leaving the Printer page mid-assignment doesn't panic", async () => {
    await setFixture("printer_assign_slot", { __delay: 400, answer: withSlot("A3", A3_ASSIGNED) });
    const before = (await callsOf("printer_assign_slot")).length;
    await slotCard("A3").locator(".pr-slot-main").click();
    await page.waitForSelector(".pr-picker .pr-picker-item", { timeout: 10000 });
    const errors = run.errors.length;
    await page.locator(".pr-picker-item").first().click();
    // The picker's backdrop covers the rail, so navigate the way the agent would.
    await page.locator('a[href="/about"]').dispatchEvent("click");
    await page.waitForSelector(".about-page", { timeout: 15000 });
    await page.waitForTimeout(800);
    if ((await callsOf("printer_assign_slot")).length !== before + 1) throw new Error("assign was not sent");
    if (run.errors.length !== errors) throw new Error(run.errors.slice(errors).join("; "));
  });

  // Put the canned answers back so later steps start from the defaults.
  for (const cmd of Object.keys(FIXTURES).filter((k) => k.startsWith("printer_") || k === "list_profiles" || k === "list_system_profiles")) {
    await setFixture(cmd, FIXTURES[cmd]);
  }

  // -- agent drawer ------------------------------------------------------------
  const emit = (payload) => page.evaluate((p) => window.__emit("agent://event", p), payload);
  const called = (cmd) => page.evaluate((c) => window.__ipc.calls.filter((x) => x.cmd === c), cmd);

  await step(run, page, "agent drawer opens from the toggle", async () => {
    await page.click(".agent-toggle");
    await page.waitForSelector(".agent-drawer.open", { timeout: 5000 });
    const box = await page.locator(".agent-drawer").boundingBox();
    const vp = page.viewportSize();
    if (box.x + box.width > vp.width + 1) throw new Error(`drawer overflows: ${JSON.stringify(box)}`);
  });

  await step(run, page, "drawer shows readiness and the default model", async () => {
    await page.waitForFunction(() => document.querySelector(".ag-status")?.innerText.startsWith("READY"), null, { timeout: 5000 });
    const model = await page.locator(".ag-model").inputValue();
    if (model !== "gpt-test") throw new Error(`default model is "${model}", expected "gpt-test"`);
    return model;
  });

  await step(run, page, "sending starts a session and a turn", async () => {
    await page.fill(".ag-input", "Why is my PETG stringing?");
    await page.press(".ag-input", "Enter");
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "agent_send"), null, { timeout: 5000 });
    const start = await called("agent_start");
    if (start[0].args.provider !== "codex" || start[0].args.model !== "gpt-test") {
      throw new Error(`started ${JSON.stringify(start[0].args)}`);
    }
    await page.waitForSelector(".ag-user", { timeout: 2000 });
  });

  await step(run, page, "streamed text, tool activity and asks render", async () => {
    await emit({ kind: "turn_started", session_id: "sess-1", seq: 1 });
    await emit({ kind: "message_delta", session_id: "sess-1", item_id: "m1", text: "Checking your " });
    await emit({ kind: "message_delta", session_id: "sess-1", item_id: "m1", text: "profile." });
    await emit({ kind: "tool_call", session_id: "sess-1", call_id: "c1", name: "bm_read_profile", args: { path: "A.json" } });
    await emit({ kind: "tool_result", session_id: "sess-1", call_id: "c1", ok: true, summary: "{\"name\":\"A\"}" });
    await emit({
      kind: "ask",
      session_id: "sess-1",
      request: { id: "a1", header: "Confirm", question: "Lower nozzle temp to 235?", options: [{ label: "Yes", description: "" }, { label: "No", description: "" }], allow_other: false },
    });
    const text = await page.locator(".ag-agent").innerText();
    if (!text.includes("Checking your profile.")) throw new Error(`agent text: ${text}`);
    const status = await page.locator(".ag-activity .ag-activity-status").innerText();
    if (status !== "[OK]") throw new Error(`activity status: ${status}`);
    if (await page.locator(".ag-stop").count() !== 1) throw new Error("stop button missing while running");
  });

  await step(run, page, "answering an ask calls agent_answer", async () => {
    await page.click(".ag-ask button.ag-option:has-text('Yes')");
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "agent_answer"), null, { timeout: 3000 });
    const [ans] = await called("agent_answer");
    if (ans.args.askId !== "a1" || ans.args.answers[0] !== "Yes") throw new Error(JSON.stringify(ans.args));
  });

  await step(run, page, "a free-text ask can be answered by typing", async () => {
    await emit({
      kind: "ask",
      session_id: "sess-1",
      request: { id: "a2", header: "Question", question: "Which printer?", options: [], allow_other: true },
    });
    await page.fill(".ag-ask .ag-other-input", "X1 Carbon");
    await page.click(".ag-ask .ag-other-send");
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "agent_answer" && c.args.askId === "a2"), null, { timeout: 3000 });
    const ans = (await called("agent_answer")).find((c) => c.args.askId === "a2");
    if (ans.args.answers.length !== 1 || ans.args.answers[0] !== "X1 Carbon") throw new Error(JSON.stringify(ans.args));
  });

  await step(run, page, "turn completes and rewind works after confirming", async () => {
    await emit({ kind: "message_done", session_id: "sess-1", item_id: "m1", text: "Lowered to 235°C." });
    await emit({ kind: "turn_done", session_id: "sess-1", seq: 1, status: "completed" });
    await page.waitForSelector(".ag-send", { timeout: 2000 });
    await page.click(".ag-rewind");
    await page.waitForSelector(".ag-confirm", { timeout: 3000 });
    const [pv] = await called("agent_rewind_preview");
    if (pv.args.sessionId !== "sess-1" || pv.args.seq !== 1) throw new Error(JSON.stringify(pv.args));
    if ((await called("agent_rewind")).length !== 0) throw new Error("rewound before confirming");
    const card = await page.locator(".ag-confirm").innerText();
    if (!card.includes("A.json") || card.includes("/p/")) throw new Error(`confirm card: ${card}`);
    await page.click(".ag-confirm .ag-confirm-rewind");
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "agent_rewind"), null, { timeout: 3000 });
    const [rw] = await called("agent_rewind");
    if (rw.args.sessionId !== "sess-1" || rw.args.seq !== 1) throw new Error(JSON.stringify(rw.args));
    if (await page.locator(".ag-user").count() !== 0) throw new Error("rewound message still shown");
  });

  await step(run, page, "invalid-profile warning is shown in accent", async () => {
    await emit({ kind: "invalid_profiles", session_id: "sess-1", seq: 1, paths: ["/p/Broken.json"] });
    const { color, accent } = await page.locator(".ag-notice.ag-error").last().evaluate((el) => {
      const probe = document.createElement("span");
      probe.style.color = "var(--nd-accent)";
      el.parentElement.appendChild(probe);
      const accent = getComputedStyle(probe).color;
      probe.remove();
      return { color: getComputedStyle(el).color, accent };
    });
    if (color !== accent) throw new Error(`notice color ${color}, accent ${accent}`);
  });

  await page.screenshot({ path: `flow-${engine}-agent.png`, fullPage: false });

  await step(run, page, "NEW CHAT ends the old session and clears the chat", async () => {
    await page.click(".ag-new");
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "agent_delete_session"), null, { timeout: 3000 });
    const [del] = await called("agent_delete_session");
    if (del.args.sessionId !== "sess-1") throw new Error(JSON.stringify(del.args));
    if (await page.locator(".ag-stream > *").count() !== 0) throw new Error("chat not cleared");
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
