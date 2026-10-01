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
  FIXTURES,
  GIF_1X1,
  makePng,
  PETG,
  SLICE_MODEL,
  STL_INBOX_FILE,
  sliceJob,
  sliceResult,
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
  const invoke = async (cmd, args) => {
    calls.push({ cmd, args });
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
    const color = await page.locator(".ag-notice.ag-error").last().evaluate((el) => getComputedStyle(el).color);
    if (!/215,\s*25,\s*33/.test(color)) throw new Error(`notice color ${color}`);
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
