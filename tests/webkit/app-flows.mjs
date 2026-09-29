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
import {
  FIXTURES,
  GIF_1X1,
  makePng,
  PRINTER_FINGERPRINT,
  PRINTER_SERIAL,
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
  window.__ipc = { calls, unknown };
  // Steps swap a command's canned answer mid-run through this.
  window.__fixtures = fixtures;
  const invoke = async (cmd, args) => {
    calls.push({ cmd, args });
    if (!(cmd in fixtures)) {
      unknown.push(cmd);
      throw new Error(`no fixture for command '${cmd}'`);
    }
    // `{ __reject: "text" }` answers the way a Tauri command's Err(String) does;
    // `{ __delay: ms, answer }` answers late, like a slow printer.
    let answer = fixtures[cmd];
    if (answer && typeof answer === "object" && "__delay" in answer) {
      const { __delay, answer: late } = answer;
      await new Promise((r) => setTimeout(r, __delay));
      answer = late;
    }
    if (answer && typeof answer === "object" && "__reject" in answer) throw answer.__reject;
    return structuredClone(answer);
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

  // -- Settings → Printer -------------------------------------------------------
  const ipcCalls = (cmd) => page.evaluate((c) => window.__ipc.calls.filter((x) => x.cmd === c), cmd);
  const setFixture = (cmd, value) =>
    page.evaluate(([c, v]) => {
      window.__fixtures[c] = v;
    }, [cmd, value]);
  const emitPrinter = (state) => page.evaluate((s) => window.__emit("printer://connection", s), state);

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
    const [t] = await ipcCalls("printer_test_connection");
    if (t.args.accessCode !== "12345678" || t.args.serial !== PRINTER_SERIAL) {
      throw new Error(JSON.stringify(t.args));
    }
    const label = (await page.locator(".printer-trust-btn").innerText()).trim();
    if (label !== "Trust this printer") throw new Error(`button reads "${label}"`);
  });

  await step(run, page, "trusting pins the fingerprint, saves and connects", async () => {
    await setFixture("printer_test_connection", {
      connection: { state: "connected" },
      got_report: true,
      model: "H2D",
    });
    await page.click(".printer-trust-btn");
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "printer_save"), null, {
      timeout: 5000,
    });
    const [save] = await ipcCalls("printer_save");
    if (save.args.pinnedFingerprint !== PRINTER_FINGERPRINT || save.args.accessCode !== "12345678") {
      throw new Error(JSON.stringify(save.args));
    }
    await page.waitForFunction(
      () => document.querySelector(".printer-result")?.innerText.includes("Connected to H2D"),
      null,
      { timeout: 5000 }
    );
    if ((await page.inputValue("#printer-code")) !== "") throw new Error("the access code is still in the field");
  });

  await step(run, page, "after trust the follow-up test sends the pin and no access code", async () => {
    const calls = await ipcCalls("printer_test_connection");
    if (calls.length !== 2) throw new Error(`${calls.length} test calls`);
    const t = calls.at(-1).args;
    if (t.pinnedFingerprint !== PRINTER_FINGERPRINT || t.accessCode || t.ip !== "192.168.1.20" || t.serial !== PRINTER_SERIAL) {
      throw new Error(JSON.stringify(t));
    }
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
    const before = (await ipcCalls("printer_save")).length;
    await page.click(".printer-trust-btn");
    await page.waitForFunction(
      (n) => window.__ipc.calls.filter((c) => c.cmd === "printer_save").length > n,
      before,
      { timeout: 5000 }
    );
    const save = (await ipcCalls("printer_save")).at(-1);
    if (
      save.args.pinnedFingerprint !== other ||
      save.args.accessCode ||
      save.args.ip !== "192.168.1.20" ||
      save.args.serial !== PRINTER_SERIAL
    ) {
      throw new Error(JSON.stringify(save.args));
    }
    await emitPrinter({ state: "connected" });
    await page.waitForSelector(".printer-trust", { state: "detached", timeout: 5000 });
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
    const tests = (await ipcCalls("printer_test_connection")).length;
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
    const saves = (await ipcCalls("printer_save")).length;
    await page.click(".printer-trust-btn");
    await page.waitForFunction(
      (n) => window.__ipc.calls.filter((c) => c.cmd === "printer_save").length > n,
      saves,
      { timeout: 5000 }
    );
    const save = (await ipcCalls("printer_save")).at(-1).args;
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
    await page.waitForFunction(
      () => document.querySelector(".printer-result")?.innerText.includes("Connected to H2D"),
      null,
      { timeout: 5000 }
    );
  });

  await step(run, page, "editing the IP away from the pinned printer drops the pin", async () => {
    await page.fill("#printer-ip", "192.168.1.99");
    await page.fill("#printer-ip", "192.168.1.20");
    const tests = (await ipcCalls("printer_test_connection")).length;
    await page.click(".printer-test");
    await page.waitForFunction(
      (n) => window.__ipc.calls.filter((c) => c.cmd === "printer_test_connection").length > n,
      tests,
      { timeout: 5000 }
    );
    const t = (await ipcCalls("printer_test_connection")).at(-1).args;
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

  // Trust chains Save into Test after an await; leaving Settings in between
  // disposes the section's signals, which must not panic.
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

  // Put the canned answers back so later steps start from the defaults.
  for (const cmd of Object.keys(FIXTURES).filter((k) => k.startsWith("printer_"))) {
    await setFixture(cmd, FIXTURES[cmd]);
  }

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
