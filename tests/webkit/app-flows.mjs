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
  A3_DETECTED,
  A3_DETECTED_PATH,
  ADVICE,
  FIXTURES,
  GIF_1X1,
  JOURNAL_FAILED,
  JOURNAL_FINISHED,
  JOURNAL_GOOD,
  JOURNAL_PRINTING,
  JOURNAL_UNKNOWN,
  makePng,
  PETG,
  PRINTER_FINGERPRINT,
  PRINTER_SERIAL,
  PRINTER_SLOTS,
  PRINTER_UNCONFIGURED,
  PRINTER_VIEW,
  product,
  reason,
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
    if (cmd === "set_preference" && args?.key === "agent_codex_model") live.agent_get_settings.codex_model = args.value;
    if (cmd === "set_preference" && args?.key === "agent_codex_effort") live.agent_get_settings.codex_effort = args.value;
    if (cmd === "get_feature_flags") {
      settled.push(cmd);
      return { ...live.get_feature_flags, analysis_enabled: analysisEnabled };
    }
    if (cmd === "check_setup_complete" && sessionStorage.getItem("mock-setup-complete") === "true") {
      settled.push(cmd);
      return { ...live.check_setup_complete, setup_complete: true };
    }
    if (!(cmd in live)) {
      unknown.push(cmd);
      throw new Error(`no fixture for command '${cmd}'`);
    }
    try {
      const answer = await answerFor(cmd);
      if (cmd === "set_preference" && args?.key === "setup_complete") {
        sessionStorage.setItem("mock-setup-complete", args.value);
      }
      return answer;
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
  // Follows an in-app link without the rail: the router handles any
  // same-origin <a> click, which is what a link elsewhere in the app does.
  // Waits for the route to land (`expect` is where it ends up, for an old
  // route that redirects), then fails if the browser did a full page load
  // instead: a load would have dropped the marker set before the click.
  const goTo = async (route, expect = route) => {
    await page.evaluate((r) => {
      window.__sameDocument = true;
      const a = document.createElement("a");
      a.href = r;
      document.body.append(a);
      a.click();
      a.remove();
    }, route);
    await page.waitForFunction((p) => location.pathname === p, expect, { timeout: 5000 });
    if (!(await page.evaluate(() => window.__sameDocument === true))) {
      throw new Error(`following ${route} reloaded the page`);
    }
  };
  // Opens a hub from the rail, then one of its tabs.
  const openTab = async (hub, tab) => {
    await page.click(`nav.sidebar a[href="${hub}"]`);
    await page.click(`.nd-tabs a[href="${tab}"]`);
  };
  const callsOf = (cmd) => page.evaluate((c) => window.__ipc.calls.filter((x) => x.cmd === c), cmd);
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

  console.log(`\n== ${engine} ==`);

  // -- boot ----------------------------------------------------------------
  await step(run, page, "app boots and renders the shell", async () => {
    await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
    await page.waitForSelector(".sidebar", { timeout: 45000 });
    const links = await page.locator(".nav-link").count();
    if (links !== 7) throw new Error(`${links} nav links rendered, expected the 7 hubs`);
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
    await page.hover('nav.sidebar a[href="/analysis"]');
    await page.waitForFunction(() => document.querySelector("nav.sidebar").getBoundingClientRect().width >= 219);
    await page.click('nav.sidebar a[href="/analysis"]');
    // Deliberately leave the pointer where the user clicked. The UI itself
    // must finish this hover interaction; moving it away masks the regression.
    await page.waitForFunction(() => document.querySelector("nav.sidebar").getBoundingClientRect().width <= 65, null, { timeout: 3000 });
  });

  await step(run, page, "navigation leaves the new page's left controls clickable", async () => {
    await page.click('nav.sidebar a[href="/settings"]');
    await page.waitForSelector("#slice-auto");
    await page.locator("#slice-auto").click({ trial: true });
    await openTab("/settings", "/settings/health");
    await page.waitForSelector(".diagnostics-controls button.btn-primary");
    await page.locator(".diagnostics-controls button.btn-primary").click({ trial: true });
    // A fresh hover remains available after leaving the rail.
    await page.mouse.move(900, 400);
    await page.hover("nav.sidebar");
    await page.waitForFunction(() => document.querySelector("nav.sidebar").getBoundingClientRect().width >= 219);
    await page.mouse.move(900, 400);
    await page.waitForFunction(() => document.querySelector("nav.sidebar").getBoundingClientRect().width <= 65);
  });

  await step(run, page, "Settings leads with the Health summary", async () => {
    await page.click('nav.sidebar a[href="/settings"]');
    await page.waitForFunction(
      () => /^(ALL GOOD|\d+ ISSUES?)$/.test(document.querySelector(".settings-page .health-hero .nd-hero-number")?.textContent?.trim() ?? ""),
      null,
      { timeout: 15000 },
    );
    const word = (await page.locator(".settings-page .health-hero .nd-hero-number").textContent()).trim();
    if (!/^(ALL GOOD|\d+ ISSUES?)$/.test(word)) throw new Error(`hero reads "${word}"`);
    const h2s = await page.locator(".settings-page h2.settings-section-title").count();
    if (h2s < 4) throw new Error(`${h2s} section titles`);
    const h3s = await page.locator(".settings-page .settings-section h3").count();
    if (h3s !== 0) throw new Error(`${h3s} section h3s left unstyled`);
  });

  // Preferred brands. Waits here are on the fixture's invoke log, the proof
  // that a save reached the backend, never on the optimistic button state.
  const PB = ".preferred-brands";
  const pbSaves = () => callsOf("set_preferred_brands");
  const pbPressed = async () => (await page.locator(`${PB} .pb-brand[aria-pressed='true']`).allTextContents()).join("|");
  const pbSavedPast = (n) =>
    page.waitForFunction((k) => window.__ipc.calls.filter((c) => c.cmd === "set_preferred_brands").length >= k, n, { timeout: 5000 });
  const pbSettledPast = (n) =>
    page.waitForFunction((k) => window.__ipc.settled.filter((c) => c === "set_preferred_brands").length >= k, n, { timeout: 5000 });
  const pbStatusIs = (text) =>
    page.waitForFunction(([sel, t]) => document.querySelector(sel)?.textContent === t, [`${PB} .pb-status`, text], { timeout: 5000 });
  // What the picker and its status line show right now. Used after an answer
  // has settled: the invoke log's `settled` entry is pushed just before the
  // answer reaches the page, whose continuation runs as microtasks, so by the
  // time this poll reads the DOM that answer has already been handled.
  const pbShows = (pressed, status) =>
    page.waitForFunction(
      ([sel, want, text]) => {
        const on = [...document.querySelectorAll(`${sel} .pb-brand[aria-pressed='true']`)].map((b) => b.textContent).join("|");
        return on === want && document.querySelector(`${sel} .pb-status`)?.textContent === text;
      },
      [PB, pressed, status],
      { timeout: 5000 }
    );
  const pbArgs = async (from) => JSON.stringify((await pbSaves()).slice(from).map((c) => c.args.brands));
  const FAILED_SENTENCE = "[ERROR: Couldn't save your preferred brands.]";

  await step(run, page, "Settings lists the catalog's brands and saves a toggle inline", async () => {
    await page.waitForSelector(`${PB} .pb-brand`, { timeout: 15000 });
    const labels = await page.locator(`${PB} .pb-brand`).allTextContents();
    if (labels.join("|") !== "Bambu Lab|Polymaker|Prusament") throw new Error(`brands ${labels.join("|")}`);
    if ((await pbPressed()) !== "") throw new Error(`pressed before any choice: ${await pbPressed()}`);
    // The status node is there from the start and empty, so its text is announced when it arrives.
    const status = page.locator(`${PB} .pb-status[role='status']`);
    if ((await status.count()) !== 1 || (await status.textContent()) !== "") throw new Error("the status line is not mounted empty");
    const before = (await pbSaves()).length;
    await withFixtures({ set_preferred_brands: ["Polymaker"] }, async () => {
      await page.click(`${PB} .pb-brand:has-text('Polymaker')`);
      await pbSavedPast(before + 1);
      await pbStatusIs("[SAVED]");
    });
    if ((await pbSaves()).length !== before + 1) throw new Error("not exactly one save for one toggle");
    if ((await pbArgs(before)) !== '[["Polymaker"]]') throw new Error(await pbArgs(before));
    if ((await pbPressed()) !== "Polymaker") throw new Error(`pressed ${await pbPressed()}`);
    if ((await page.locator(".toast").count()) !== 0) throw new Error("a toast announced the save");
  });

  await step(run, page, "toggling a brand off saves the empty list", async () => {
    const before = (await pbSaves()).length;
    await withFixtures({ set_preferred_brands: [] }, async () => {
      await page.click(`${PB} .pb-brand:has-text('Polymaker')`);
      await pbSavedPast(before + 1);
      await pbStatusIs("[SAVED]");
    });
    if ((await pbArgs(before)) !== "[[]]") throw new Error(await pbArgs(before));
    if ((await pbPressed()) !== "") throw new Error(`pressed ${await pbPressed()}`);
  });

  await step(run, page, "a failed save reverts the picker and says so in a fixed sentence", async () => {
    const before = (await pbSaves()).length;
    await withFixtures({ set_preferred_brands: { __reject: "disk full: /secret/path" } }, async () => {
      await page.click(`${PB} .pb-brand:has-text('Prusament')`);
      await pbSavedPast(before + 1);
      await pbStatusIs(FAILED_SENTENCE);
    });
    if ((await page.locator(`${PB} .pb-status`).getAttribute("data-kind")) !== "error") throw new Error("the failure is not marked as an error");
    if ((await pbPressed()) !== "") throw new Error(`the failed toggle stayed pressed: ${await pbPressed()}`);
    if ((await page.locator(PB).innerText()).includes("disk full")) throw new Error("the raw error leaked into the page");
  });

  await step(run, page, "rapid toggles settle on the last save, shown and stored agreeing", async () => {
    const before = (await pbSaves()).length;
    const settled = await page.evaluate(() => window.__ipc.settled.filter((c) => c === "set_preferred_brands").length);
    const seq = [
      { __hold: "pb-a", answer: ["Bambu Lab"] },
      { __hold: "pb-b", answer: ["Bambu Lab", "Polymaker"] },
    ];
    await withFixtures({ set_preferred_brands: { __sequence: seq } }, async () => {
      await page.click(`${PB} .pb-brand:has-text('Bambu Lab')`);
      await page.click(`${PB} .pb-brand:has-text('Polymaker')`);
      await pbSavedPast(before + 2);
      if ((await pbPressed()) !== "Bambu Lab|Polymaker") throw new Error(`optimistic ${await pbPressed()}`);
      // The newest answer arrives first; the older one arrives late and must change nothing.
      await page.evaluate(() => window.__release("pb-b"));
      await pbStatusIs("[SAVED]");
      await page.evaluate(() => window.__release("pb-a"));
      await pbSettledPast(settled + 2);
      await pbShows("Bambu Lab|Polymaker", "[SAVED]");
    });
    if ((await pbArgs(before)) !== '[["Bambu Lab"],["Bambu Lab","Polymaker"]]') throw new Error(await pbArgs(before));
    if ((await pbPressed()) !== "Bambu Lab|Polymaker") throw new Error(`shown ${await pbPressed()}`);
    if ((await page.locator(`${PB} .pb-status`).textContent()) !== "[SAVED]") throw new Error("status not [SAVED]");
  });

  await step(run, page, "a success dropped before a failed newest save is not lost", async () => {
    // Stored: Bambu Lab, Polymaker. Toggle Prusament (saves), then Bambu Lab (fails).
    const before = (await pbSaves()).length;
    const settled = await page.evaluate(() => window.__ipc.settled.filter((c) => c === "set_preferred_brands").length);
    const seq = [
      { __hold: "pb-c", answer: ["Bambu Lab", "Polymaker", "Prusament"] },
      { __hold: "pb-d", answer: { __reject: "nope" } },
    ];
    await withFixtures({ set_preferred_brands: { __sequence: seq } }, async () => {
      await page.click(`${PB} .pb-brand:has-text('Prusament')`);
      await page.click(`${PB} .pb-brand:has-text('Bambu Lab')`);
      await pbSavedPast(before + 2);
      await page.evaluate(() => window.__release("pb-d"));
      await pbStatusIs(FAILED_SENTENCE);
      // The newest failed while the first is still out: back to what was confirmed so far.
      await page.waitForFunction(() => [...document.querySelectorAll(".preferred-brands .pb-brand[aria-pressed='true']")].map((b) => b.textContent).join("|") === "Bambu Lab|Polymaker", null, { timeout: 5000 });
      await page.evaluate(() => window.__release("pb-c"));
      await pbSettledPast(settled + 2);
      await pbShows("Bambu Lab|Polymaker|Prusament", FAILED_SENTENCE);
    });
    // The first save did reach the store, so the picker shows it.
    if ((await pbPressed()) !== "Bambu Lab|Polymaker|Prusament") throw new Error(`shown ${await pbPressed()}`);
    if ((await page.locator(`${PB} .pb-status`).textContent()) !== FAILED_SENTENCE) throw new Error("the failure was forgotten");
  });

  await step(run, page, "an unreadable stored set hides the picker until a read succeeds", async () => {
    await page.click('nav.sidebar a[href="/profiles"]');
    await page.waitForSelector(".profile-list-item", { timeout: 15000 });
    const before = (await pbSaves()).length;
    await withFixtures({ get_preferred_brands: { __reject: "ipc down" } }, async () => {
      await page.click('nav.sidebar a[href="/settings"]');
      await page.waitForSelector(`${PB} .pb-retry`, { timeout: 5000 });
      if ((await page.locator(`${PB} .pb-brand`).count()) !== 0) throw new Error("the picker is offered without the stored set");
      const msg = (await page.locator(`${PB} .nd-state[data-kind='error']`).textContent()).trim();
      if (msg !== "[ERROR: Couldn't read your preferred brands.]") throw new Error(msg);
    });
    await withFixtures({ get_preferred_brands: ["Polymaker"] }, async () => {
      await page.click(`${PB} .pb-retry`);
      await page.waitForSelector(`${PB} .pb-brand`, { timeout: 5000 });
    });
    if ((await pbPressed()) !== "Polymaker") throw new Error(`pressed ${await pbPressed()}`);
    if ((await pbSaves()).length !== before) throw new Error("a save was sent while the stored set was unknown");
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

    await step(run, page, "the filament hero and step strip follow the flow", async () => {
      const stats = await page.locator(".fs-specs-hero .nd-hero-stat-value").allTextContents();
      if (stats.join(" ") !== "220° 65° 0.98") throw new Error(`hero reads "${stats.join(" ")}"`);
      const current = await page.locator('.nd-step[data-state="current"] .nd-step-name').textContent();
      if (current !== "Tune") throw new Error(`current step "${current}"`);
      if ((await page.locator(".search-container").count()) !== 0) throw new Error("search box still shown after picking");
      await page.click('.nd-step[data-state="done"] button.nd-step-btn');
      await page.waitForSelector(".filament-card", { timeout: 5000 });
      const back = await page.locator('.nd-step[data-state="current"] .nd-step-name').textContent();
      if (back !== "Specs") throw new Error(`after Back the current step is "${back}"`);
      // Keyboard focus follows to the current step instead of being lost with the button.
      await page.waitForFunction(
        () => document.activeElement?.matches('.nd-step[data-state="current"] .nd-step-btn'),
        null,
        { timeout: 2000 }
      ).catch(() => {
        throw new Error("after Back, focus is not on the current step");
      });
      // Return to the editor for the steps that follow.
      await page.click(".filament-card-generate-btn");
      await page.waitForSelector(".editor-section", { timeout: 20000 });
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

    // Back from the Install step goes to the editor; a profile can then be generated again.
    const doneStep = (name) => page.locator(`.nd-step[data-state="done"]:has(.nd-step-name:text-is("${name}")) .nd-step-btn`);
    const currentStep = () => page.locator('.nd-step[data-state="current"] .nd-step-name').textContent();
    await step(run, page, "Back from Install returns to the editor", async () => {
      await page.click('.editor-section button:has-text("Generate Profile")');
      await page.waitForSelector(".profile-preview", { timeout: 20000 });
      if ((await currentStep()) !== "Install") throw new Error(`current step "${await currentStep()}" after generating`);
      await doneStep("Tune").click();
      await page.waitForSelector(".editor-section", { timeout: 5000 });
      if ((await page.locator(".profile-preview").count()) !== 0) throw new Error("preview still shown after Back");
      if ((await currentStep()) !== "Tune") throw new Error(`after Back the current step is "${await currentStep()}"`);
      await page.click('.editor-section button:has-text("Generate Profile")');
      await page.waitForSelector(".profile-preview", { timeout: 20000 });
      if ((await currentStep()) !== "Install") throw new Error("generating again did not land on Install");
    });

    await step(run, page, "Back is blocked while a profile is generating", async () => {
      await doneStep("Tune").click();
      await page.waitForSelector(".editor-section", { timeout: 5000 });
      await withFixtures(
        { generate_profile_from_specs: { __hold: "gen-back", answer: FIXTURES.generate_profile_from_specs } },
        async () => {
          const before = (await callsOf("generate_profile_from_specs")).length;
          await page.click('.editor-section button:has-text("Generate Profile")');
          await page.waitForFunction(
            (n) => window.__ipc.calls.filter((c) => c.cmd === "generate_profile_from_specs").length > n,
            before,
            { timeout: 5000 }
          );
          const buttons = await page.locator("button.nd-step-btn").count();
          if (buttons !== 0) throw new Error(`${buttons} step buttons while generating, expected none`);
          if (!(await page.locator(".fs-search-another").isDisabled())) throw new Error("Search another enabled while generating");
          if ((await page.locator(".nd-step[data-state=\"done\"] span.nd-step-btn").count()) < 1) {
            throw new Error("done steps are not plain text while generating");
          }
          await page.evaluate(() => window.__release("gen-back"));
          await page.waitForSelector(".profile-preview", { timeout: 20000 });
          if ((await currentStep()) !== "Install") throw new Error("finishing the generate did not land on Install");
          if ((await doneStep("Tune").count()) !== 1) throw new Error("Back is not offered once generating is over");
        }
      );
    });

    await step(run, page, "Back is blocked while a profile is installing", async () => {
      await withFixtures(
        { install_generated_profile: { __hold: "install-back", answer: FIXTURES.install_generated_profile } },
        async () => {
          await page.click(".profile-preview-actions .btn-primary");
          await page.waitForFunction(
            () => document.querySelector(".profile-preview-actions .btn-primary")?.disabled === true,
            null,
            { timeout: 5000 }
          );
          const buttons = await page.locator("button.nd-step-btn").count();
          if (buttons !== 0) throw new Error(`${buttons} step buttons while installing, expected none`);
          if (!(await page.locator(".fs-search-another").isDisabled())) throw new Error("Search another enabled while installing");
          await page.evaluate(() => window.__release("install-back"));
          await page.waitForSelector(".success-message", { timeout: 20000 });
          if ((await page.locator("button.nd-step-btn").count()) !== 0) throw new Error("Back offered after an install");
          if (await page.locator(".fs-search-another").isDisabled()) throw new Error("Search another still disabled after the install");
        }
      );
    });

    await page.screenshot({ path: `flow-${engine}-filament.png`, fullPage: true });

    await step(run, page, "Search another brings the search box back", async () => {
      await page.click(".fs-search-another");
      await page.waitForSelector(".fs-search-hero .search-input", { timeout: 5000 });
      if ((await page.locator(".fs-specs-hero").count()) !== 0) throw new Error("specs hero still shown");
    });
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

    // The picker is the page's own control; the mock answers with SLICE_MODEL.
    // No slice has run yet, so there is no "last sliced model" to offer.
    const settledNow = (cmd) => page.evaluate((c) => window.__ipc.settled.filter((x) => x === c).length, cmd);
    const toReady = async () => {
      await page.click("text=Choose Different Photo");
      await page.waitForSelector(".drop-zone", { timeout: 15000 });
      await page.setInputFiles("#photo-file-input", { name: "print.png", mimeType: "image/png", buffer: png });
      await page.waitForSelector(".analysis-preview", { timeout: 20000 });
    };

    await step(run, page, "the ready state offers an optional model, picked and cleared", async () => {
      await page.waitForSelector(".an-pick-model", { timeout: 5000 });
      if ((await page.locator(".an-last-model").count()) !== 0) throw new Error("offered a last sliced model with no jobs");
      if ((await page.locator(".an-model-name").count()) !== 0) throw new Error("a model is named before one is chosen");
      await page.click(".an-pick-model");
      await page.waitForSelector(".an-model-name", { timeout: 5000 });
      const name = (await page.locator(".an-model-name").textContent()).trim();
      if (name !== "cube.stl") throw new Error(`named "${name}"`);
      await page.click(".an-model-clear");
      await page.waitForFunction(() => !document.querySelector(".an-model-name"), null, { timeout: 5000 });
      await page.click(".an-pick-model");
      await page.waitForSelector(".an-model-name", { timeout: 5000 });
    });

    await step(run, page, "a model does not survive Reset, and a pick finishing after Reset is dropped", async () => {
      await toReady();
      if ((await page.locator(".an-model-name").count()) !== 0) throw new Error("the model came back after Reset");
      const settled = await settledNow("slicer_pick_model");
      await withFixtures({ slicer_pick_model: { __hold: "an-pick-back", answer: SLICE_MODEL } }, async () => {
        await page.click(".an-pick-model");
        await toReady();
        await page.evaluate(() => window.__release("an-pick-back"));
        await page.waitForFunction((k) => window.__ipc.settled.filter((x) => x === "slicer_pick_model").length > k, settled, { timeout: 5000 });
        // One more round trip to the page: the app's continuation ran before it.
        await page.evaluate(() => 0);
      });
      if ((await page.locator(".an-model-name").count()) !== 0) throw new Error("a late pick set the model after Reset");
      await page.click(".an-pick-model");
      await page.waitForSelector(".an-model-name", { timeout: 5000 });
    });

    await step(run, page, "analysis runs and reports defects", async () => {
      await page.selectOption(".profile-select", { index: 1 });
      await page.click("text=Analyze Print");
      await page.waitForSelector(".analysis-results", { timeout: 30000 });
      const text = await page.locator(".analysis-results").innerText();
      if (!/string/i.test(text)) throw new Error("detected defect not shown");
      const word = (await page.locator(".an-hero-row .nd-hero-number").textContent()).trim();
      if (word !== "FAIR") throw new Error(`quality reads "${word}"`);
      const tone = await page.locator(".an-hero-row .nd-hero").getAttribute("data-tone");
      if (tone !== "warning") throw new Error(`quality tone ${tone}`);
      const order = await page.locator(".defects-list .defect-card").evaluateAll((els) => els.map((e) => e.dataset.tier));
      if (order.join(",") !== "significant,noticeable") throw new Error(`defects in order ${order}`);
      const first = (await page.locator(".defects-list .defect-number").first().textContent()).trim();
      if (first !== "01") throw new Error(`first defect numbered ${first}`);
      return `${text.replace(/\s+/g, " ").slice(0, 70)}...`;
    });

    await step(run, page, "recommendations are listed", async () => {
      const text = await page.locator(".analysis-results").innerText();
      for (const want of ["Nozzle Temperature", "Retraction"]) {
        if (!text.includes(want)) throw new Error(`"${want}" missing from results`);
      }
      return "temperature and retraction shown";
    });

    await step(run, page, "the model's 3D view shows with its risks beside the analysis", async () => {
      await page.waitForSelector(".analysis-results .an-model .vp", { timeout: 15000 });
      await page.waitForSelector(".analysis-results .an-model .vp-risk", { timeout: 15000 });
      const risks = await page.locator(".analysis-results .an-model .vp-risk").count();
      if (risks !== 2) throw new Error(`${risks} risk rows`);
      const asked = (await callsOf("model_view")).at(-1)?.args;
      if (asked?.path !== SLICE_MODEL) throw new Error(`model_view asked for ${JSON.stringify(asked)}`);
      const name = (await page.locator(".analysis-results .an-model .vp-name").textContent()).trim();
      if (name !== "cube.stl") throw new Error(`the view names "${name}"`);
      const geometry = await page.evaluate(() => {
        const box = (sel) => document.querySelector(sel).getBoundingClientRect();
        const vp = document.querySelector(".an-model .vp");
        const cs = getComputedStyle(vp);
        const photo = box(".an-hero-row").bottom;
        return {
          below: box(".an-model").top >= photo - 1,
          wrapper: box(".an-model").width,
          view: box(".an-model .vp").width,
          border: cs.borderTopWidth,
          padding: cs.paddingTop,
          overflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
        };
      });
      if (!geometry.below) throw new Error("the model sits above the hero row");
      if (geometry.view > geometry.wrapper + 1) throw new Error(`view ${geometry.view} wider than its section ${geometry.wrapper}`);
      if (geometry.border !== "0px" || geometry.padding !== "0px") throw new Error(`the view is styled as a card: ${JSON.stringify(geometry)}`);
      if (geometry.overflow > 0) throw new Error(`page overflows by ${geometry.overflow}px`);
      const gl2 = await page.evaluate(() => !!document.createElement("canvas").getContext("webgl2"));
      if (gl2 && (await page.locator(".analysis-results .an-model .vp-canvas").count()) !== 1) {
        throw new Error("WebGL2 is available but there is no canvas");
      }
      return `${Math.round(geometry.view)}px wide, ${gl2 ? "webgl" : "fallback"}`;
    });

    await step(run, page, "Analyze Another Photo leaves no model behind", async () => {
      await page.click("text=Analyze Another Photo");
      await page.waitForSelector(".drop-zone", { timeout: 15000 });
      await page.setInputFiles("#photo-file-input", { name: "print.png", mimeType: "image/png", buffer: png });
      await page.waitForSelector(".analysis-preview", { timeout: 20000 });
      if ((await page.locator(".an-model-name").count()) !== 0) throw new Error("the model is still named");
      await page.selectOption(".profile-select", { index: 1 });
      await page.click("text=Analyze Print");
      await page.waitForSelector(".analysis-results", { timeout: 30000 });
      await page.waitForSelector(".analysis-results .defects-list", { timeout: 15000 });
      if ((await page.locator(".an-model").count()) !== 0) throw new Error("a model section shows without a chosen model");
    });

    await step(run, page, "Use last sliced model picks the newest job's model", async () => {
      const older = "/Users/runner/models/older.stl";
      const newer = "/Users/runner/models/newer.stl";
      const fb = await browser.newPage({ viewport: { width: 1280, height: 900 } });
      fb.on("pageerror", (e) => run.errors.push(`pageerror: ${e.message}`));
      fb.on("console", (m) => { if (m.type() === "error") run.errors.push(`console.error: ${m.text()}`); });
      try {
        await fb.addInitScript(installTauriMock, {
          ...FIXTURES,
          slicer_jobs: [
            sliceJob(1, { state: "cancelled" }, { source_path: older }),
            sliceJob(2, { state: "queued", position: 0 }, { source_path: newer, model_name: "newer.stl" }),
            sliceJob(3, { state: "cancelled" }, { source_path: "" }),
          ],
        });
        await fb.goto(baseUrl, { waitUntil: "domcontentloaded" });
        await fb.waitForSelector(".sidebar", { timeout: 45000 });
        await fb.click('a[href="/analysis"]');
        await fb.waitForSelector(".drop-zone", { timeout: 15000 });
        await fb.setInputFiles("#photo-file-input", { name: "print.png", mimeType: "image/png", buffer: png });
        await fb.waitForSelector(".an-last-model", { timeout: 15000 });
        await fb.click(".an-last-model");
        await fb.waitForFunction(() => document.querySelector(".an-model-name")?.textContent.trim() === "newer.stl", null, { timeout: 5000 });
        await fb.click("text=Analyze Print");
        await fb.waitForSelector(".analysis-results .an-model .vp-risk", { timeout: 30000 });
        const asked = await fb.evaluate(() => window.__ipc.calls.filter((c) => c.cmd === "model_view").at(-1)?.args.path);
        if (asked !== newer) throw new Error(`model_view asked for ${asked}`);
      } finally {
        await fb.close();
      }
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
    await page.waitForSelector(".profile-management-page .nd-hero", { timeout: 15000 });
    await page.waitForSelector("table.profile-fields", { timeout: 15000 });
    const title = (await page.locator(".pm-detail-head .nd-hero-label").textContent()).trim();
    const nozzle = (await page.locator(".pm-detail-head .nd-hero-number").textContent()).trim();
    if (nozzle !== "220°") throw new Error(`hero reads "${nozzle}"`);
    const subtitle = (await page.locator(".pm-detail-head .nd-hero-detail").textContent()).trim();
    if (!subtitle.includes("42 fields")) throw new Error(`detail reads "${subtitle}"`);
    const stats = await page.locator(".pm-stats .nd-stat-row").count();
    if (stats !== 4) throw new Error(`${stats} stat rows`);
    const keys = await page.locator("td.field-key").count();
    if (keys === 0) throw new Error("fields table rendered with no rows");
    const pills = await page.locator(".pm-actions .btn").count();
    if (pills !== 4) throw new Error(`${pills} action buttons`);
    const calHref = await page.locator(".pm-actions a.pm-calibrate").getAttribute("href");
    if (!calHref || !calHref.startsWith("/filament/calibrate?preset=") || calHref.includes(" ")) throw new Error(`profile Calibrate link: ${calHref}`);
    return `${title}, ${keys} field rows`;
  });

  await step(run, page, "a profile's Calibrate link carries its exact path and opens the tab on it", async () => {
    // Every character a partial encoder gets wrong: space, &, #, @ and +.
    const path = "/Users/runner/Library/Application Support/BambuStudio/user/00000001/filament/Poly & Co #1 PLA+ @BBL X1C.json";
    const strict = (s) => encodeURIComponent(s).replace(/[!'()*]/g, (c) => "%" + c.charCodeAt(0).toString(16).toUpperCase());
    const first = { ...FIXTURES.list_profiles[0], name: "Poly & Co #1 PLA+ @BBL X1C", path };
    await withFixtures(
      { list_profiles: [first, ...FIXTURES.list_profiles.slice(1)], calibration_start: { __reject: "stub: only the request is under test" } },
      async () => {
        await page.click('nav.sidebar a[href="/"]');
        await page.click('nav.sidebar a[href="/profiles"]');
        await page.waitForSelector(".profile-list-item", { timeout: 15000 });
        await page.locator(".profile-list-item").first().click();
        const link = page.locator(".pm-actions a.pm-calibrate");
        await link.waitFor({ timeout: 15000 });
        const href = await link.getAttribute("href");
        if (href !== `/filament/calibrate?preset=${strict(path)}`) throw new Error(`profile Calibrate href: ${href}`);
        const before = (await callsOf("calibration_start")).length;
        await link.click();
        await page.waitForSelector(".calibrate-page", { timeout: 15000 });
        await page.waitForFunction((n) => window.__ipc.calls.filter((c) => c.cmd === "calibration_start").length > n, before, { timeout: 5000 });
        const asked = (await callsOf("calibration_start")).at(-1).args;
        if (asked.preset !== path || asked.slot != null || asked.run != null) throw new Error(`calibration_start got ${JSON.stringify(asked)}`);
        if (new URL(page.url()).searchParams.get("preset") !== path) throw new Error(`the page URL names "${new URL(page.url()).searchParams.get("preset")}"`);
      }
    );
    await page.click('nav.sidebar a[href="/"]');
    await page.click('nav.sidebar a[href="/profiles"]');
    await page.waitForSelector(".profile-list-item", { timeout: 15000 });
  });

  await step(run, page, "a preset that isn't syncing gets a warning dot", async () => {
    const path = FIXTURES.list_profiles[0].path;
    await withFixtures(
      { list_unsynced_presets: { presets: [{ path, profile_name: "x", file_name: "x.json", source: "confirmed" }], bambu_studio_running: false } },
      async () => {
        await page.click('nav.sidebar a[href="/"]');
        await page.click('nav.sidebar a[href="/profiles"]');
        await page.waitForSelector(".profile-list-item .pm-sync-dot", { timeout: 15000 });
        const dots = await page.locator(".pm-sync-dot").count();
        if (dots !== 1) throw new Error(`${dots} dots, expected 1`);
        const label = await page.locator(".pm-sync-dot").getAttribute("aria-label");
        if (label !== "Not syncing") throw new Error(`dot label "${label}"`);
      }
    );
  });

  await step(run, page, "a failed unsynced scan still shows the profile list", async () => {
    await withFixtures({ list_unsynced_presets: { __reject: "scan failed" } }, async () => {
      await page.click('nav.sidebar a[href="/"]');
      await page.click('nav.sidebar a[href="/profiles"]');
      await page.waitForSelector(".profile-list-item", { timeout: 15000 });
      const dots = await page.locator(".pm-sync-dot").count();
      if (dots !== 0) throw new Error(`${dots} dots after a failed scan`);
    });
  });

  // -- batch generate --------------------------------------------------------
  await step(run, page, "navigate to Batch Generate", async () => {
    await openTab("/filament", "/filament/batch");
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
    const hero = (await page.locator(".batch-results .nd-hero-number").textContent()).trim();
    if (hero !== "2/3") throw new Error(`hero reads "${hero}"`);
    const detail = (await page.locator(".batch-results .nd-hero-detail").textContent()).trim();
    if (detail !== "1 failed") throw new Error(`detail reads "${detail}"`);
    const on = await page.locator(".batch-results .nd-seg-on").count();
    if (on !== 8) throw new Error(`${on} segments for 2 of 3`);
    const failed = await page.locator("tr.row-fail").count();
    if (failed !== 1) throw new Error(`expected 1 failed row, got ${failed}`);
    return `${hero}, ${detail}`;
  });

  await step(run, page, "batch locks every input while generating", async () => {
    await withFixtures(
      { batch_generate_brand: { __hold: "batch-lock", answer: FIXTURES.batch_generate_brand } },
      async () => {
        await page.locator(".batch-generate-page button.btn-primary").first().click();
        await page.waitForSelector(".batch-generate-page .nd-state", { timeout: 5000 });
        for (const sel of ["#brand-select", "#printer-select", '.batch-config input[type="checkbox"]']) {
          if (!(await page.locator(sel).isDisabled())) throw new Error(`${sel} is editable while generating`);
        }
        await page.evaluate(() => window.__release("batch-lock"));
        await page.waitForSelector(".batch-results", { timeout: 20000 });
        if (await page.locator("#printer-select").isDisabled()) throw new Error("printer select stayed disabled");
      }
    );
  });

  await step(run, page, "an empty brand reads as nothing to generate", async () => {
    const empty = { total: 0, completed: 0, succeeded: 0, failed: 0, results: [] };
    await withFixtures({ batch_generate_brand: empty }, async () => {
      await page.locator(".batch-generate-page button.btn-primary").first().click();
      await page.waitForFunction(
        () => document.querySelector(".batch-results .nd-hero-detail")?.textContent.trim() === "Nothing to generate",
        null,
        { timeout: 5000 }
      );
      const tone = await page.locator(".batch-results .nd-hero").getAttribute("data-tone");
      if (tone !== "neutral") throw new Error(`hero tone "${tone}", expected neutral`);
    });
  });

  // -- compare profiles ------------------------------------------------------
  await step(run, page, "navigate to Compare Profiles", async () => {
    await openTab("/profiles", "/profiles/compare");
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
    const hero = (await page.locator(".diff-results .nd-hero-number").textContent()).trim();
    if (hero !== "3") throw new Error(`hero reads "${hero}"`);
    const calls = await callsOf("compare_profiles");
    if (calls.at(-1)?.args?.showIdentical !== true) {
      throw new Error(`compare was not asked for every field: ${JSON.stringify(calls.at(-1)?.args)}`);
    }
    const categories = await page.locator(".diff-category-name").count();
    const rows = await page.locator("tr.diff-row.changed").count();
    if (categories !== 2) throw new Error(`expected 2 categories, got ${categories}`);
    if (rows !== 3) throw new Error(`expected 3 changed rows, got ${rows}`);
    return `${hero} differences, ${categories} categories`;
  });

  // .diff-table carries a border-collapse fix for WebKit. Counting rows would
  // still pass if they collapsed to zero height, so measure one instead.
  await step(run, page, "diff table rows have real height", async () => {
    const box = await page.locator("tr.diff-row.changed").first().boundingBox();
    if (!box) throw new Error("first diff row has no box");
    if (box.height < 8) throw new Error(`first diff row is only ${box.height}px tall`);
    return `row height ${Math.round(box.height)}px`;
  });

  await step(run, page, "Changed only hides identical rows without comparing again", async () => {
    const withIdentical = {
      ...FIXTURES.compare_profiles,
      categories: [
        ...FIXTURES.compare_profiles.categories,
        { category: "Cooling", diffs: [{ key: "fan_max_speed", label: "Fan Max", base_value: "100", new_value: "100" }] },
      ],
    };
    await withFixtures({ compare_profiles: withIdentical }, async () => {
      await page.locator(".diff-actions button.btn-primary").first().click();
      await page.waitForFunction(() => document.querySelectorAll("tr.diff-row").length === 3, null, { timeout: 5000 });
      const before = (await callsOf("compare_profiles")).length;
      await page.locator(".diff-changed-only").uncheck();
      await page.waitForSelector("tr.diff-row.identical", { timeout: 5000 });
      if ((await page.locator(".diff-category-name").count()) !== 3) throw new Error("Cooling category not shown");
      if ((await callsOf("compare_profiles")).length !== before) throw new Error("toggling re-compared");
      await page.locator(".diff-changed-only").check();
      await page.waitForSelector("tr.diff-row.identical", { state: "detached", timeout: 5000 });
    });
  });

  await step(run, page, "a category header collapses and restores its rows", async () => {
    const header = page.locator(".diff-category-header").first();
    const rows = page.locator(".diff-category-group").first().locator("tr.diff-row");
    const shown = await rows.count();
    if (shown === 0) throw new Error("first category has no rows to collapse");
    if ((await header.getAttribute("aria-expanded")) !== "true") throw new Error("header does not start expanded");
    await header.click();
    await page.waitForFunction(
      () => document.querySelector(".diff-category-header")?.getAttribute("aria-expanded") === "false",
      null,
      { timeout: 5000 }
    );
    if ((await rows.count()) !== 0) throw new Error("rows still shown after collapsing");
    const glyph = await header.evaluate((el) => getComputedStyle(el, "::after").content);
    if (!glyph.includes("+")) throw new Error(`collapsed cue reads ${glyph}`);
    await header.click();
    await page.waitForFunction((n) => document.querySelectorAll(".diff-category-group:first-of-type tr.diff-row").length === n, shown, { timeout: 5000 });
    if ((await header.getAttribute("aria-expanded")) !== "true") throw new Error("header did not re-expand");
  });

  await step(run, page, "singular and empty results read right", async () => {
    const same = (k) => ({ key: k, label: k, base_value: "1", new_value: "1" });
    const none = { ...FIXTURES.compare_profiles, categories: [{ category: "Temperature", diffs: [same("a")] }], changed_fields: 0 };
    await withFixtures({ compare_profiles: none }, async () => {
      await page.locator(".diff-actions button.btn-primary").first().click();
      await page.waitForSelector('.diff-results .nd-state[data-kind="empty"]', { timeout: 5000 });
      const text = await page.locator(".diff-results .nd-state").textContent();
      if (!/No differences/.test(text)) throw new Error(`empty state reads "${text}"`);
      if ((await page.locator("table.diff-table").count()) !== 0) throw new Error("table shown with no differences");
      const hero = (await page.locator(".diff-results .nd-hero-value").textContent()).replace(/\s+/g, " ").trim();
      if (hero !== "0 differences") throw new Error(`hero reads "${hero}"`);
      await page.locator(".diff-changed-only").uncheck();
      await page.waitForSelector("table.diff-table tr.diff-row.identical", { timeout: 5000 });
      await page.locator(".diff-changed-only").check();
    });
    const one = {
      ...FIXTURES.compare_profiles,
      categories: [{ category: "Temperature", diffs: [{ key: "a", label: "a", base_value: "1", new_value: "2" }] }],
      changed_fields: 1,
    };
    await withFixtures({ compare_profiles: one }, async () => {
      await page.locator(".diff-actions button.btn-primary").first().click();
      await page.waitForSelector("tr.diff-row.changed", { timeout: 5000 });
      const hero = (await page.locator(".diff-results .nd-hero-value").textContent()).replace(/\s+/g, " ").trim();
      if (hero !== "1 difference") throw new Error(`hero reads "${hero}"`);
    });
  });

  // -- slice ---------------------------------------------------------------
  const emitJob = (view) => page.evaluate((v) => window.__emit("slicer://job", v), view);
  const running = (id, percent) =>
    sliceJob(id, { state: "running", progress: { plate: 1, percent, stage: "Generating walls" } });
  const done = (id, result, extra) => sliceJob(id, { state: "done", result, cached: false }, extra);

  await step(run, page, "navigate to Slice", async () => {
    await page.click('a[href="/slice"]');
    await page.waitForSelector(".slice-page.nd", { timeout: 15000 });
    await page.waitForFunction(() => document.querySelector(".sl-version")?.innerText.includes("02.08.02.61"), null, { timeout: 10000 });
    const title = (await page.locator(".slice-page h1.nd-page-title").textContent()).trim();
    if (title !== "Slice") throw new Error(`Slice page title was "${title}"`);
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

  await step(run, page, "a chosen model offers a Suggest filament link to Advise", async () => {
    const link = page.locator("a.sl-suggest");
    if ((await link.count()) !== 1) throw new Error(`${await link.count()} Suggest filament links`);
    const href = await link.getAttribute("href");
    const want = `/filament/advise?model=${encodeURIComponent(SLICE_MODEL)}`;
    if (href !== want) throw new Error(`href ${href}, wanted ${want}`);
    if ((await link.innerText()).trim().toLowerCase() !== "suggest filament") throw new Error(`text ${await link.innerText()}`);
    // A real anchor with an href: keyboard-focusable and middle-clickable.
    const tag = await link.evaluate((el) => `${el.tagName}${el.tabIndex}`);
    if (tag !== "A0") throw new Error(`link is ${tag}`);
  });

  // Whether this engine can give a WebGL2 context at all. When it can, the
  // viewport must draw: a shader or link failure shows the "couldn't start"
  // text, which no step here accepts.
  const engineHasWebGl2 = await page.evaluate(() => !!document.createElement("canvas").getContext("webgl2"));

  await step(run, page, "the model shows as a 3D view with its risks", async () => {
    await page.waitForSelector(".slice-page .vp", { timeout: 15000 });
    const risks = await page.locator(".slice-page .vp-risk").count();
    if (risks !== 2) throw new Error(`${risks} risk rows`);
    // WebKit drops list semantics from an unstyled <ol>; the role restores them.
    if ((await page.locator(".slice-page .vp-risks").getAttribute("role")) !== "list") throw new Error("the risk list has no list role");
    const asked = (await callsOf("model_view")).at(-1)?.args;
    if (asked?.path !== SLICE_MODEL) throw new Error(`model_view asked for ${JSON.stringify(asked)}`);
    if ((await page.locator(".slice-page .vp-name").count()) !== 0) throw new Error("the viewport repeats the file name");
    const hasCanvas = (await page.locator(".slice-page .vp-canvas").count()) === 1;
    const stateText = (await page.locator(".slice-page .vp .nd-state").allTextContents()).join(" | ");
    if (engineHasWebGl2) {
      if (!hasCanvas) throw new Error(`WebGL2 is available but there is no canvas: ${stateText}`);
      if (stateText.includes("WebGL2") || stateText.includes("couldn't start")) throw new Error(stateText);
      if (!(await page.locator(".slice-page .vp-canvas").evaluate((c) => c.width > 0 && c.height > 0))) throw new Error("the canvas has no backing store");
      const box = await page.locator(".slice-page .vp-canvas").boundingBox();
      const pins = page.locator(".slice-page .vp-pin");
      if ((await pins.count()) !== 2) throw new Error(`${await pins.count()} pins`);
      const centres = [];
      for (let i = 0; i < 2; i++) {
        const b = await pins.nth(i).boundingBox();
        const c = { x: b.x + b.width / 2, y: b.y + b.height / 2 };
        if (c.x < box.x || c.x > box.x + box.width || c.y < box.y || c.y > box.y + box.height) throw new Error(`pin ${i + 1} is off the canvas`);
        centres.push(c);
      }
      if (Math.hypot(centres[0].x - centres[1].x, centres[0].y - centres[1].y) < 10) throw new Error("the two pins are in the same place");
      // Each pin selects its own row, and aria-pressed follows.
      for (const i of [1, 0]) {
        await pins.nth(i).click();
        if ((await page.locator(".slice-page .vp-risk[data-selected]").count()) !== 1) throw new Error(`pin ${i + 1} did not select exactly one row`);
        if ((await page.locator(`.slice-page .vp-risk:nth-child(${i + 1})[data-selected]`).count()) !== 1) throw new Error(`pin ${i + 1} selected the wrong row`);
        // The selected row says so to assistive tech, and shows it by more than colour.
        if ((await page.locator('.slice-page .vp-risk[aria-current="true"]').count()) !== 1) throw new Error(`pin ${i + 1}: not exactly one aria-current row`);
        if ((await page.locator(`.slice-page .vp-risk:nth-child(${i + 1})[aria-current="true"]`).count()) !== 1) throw new Error(`pin ${i + 1}: aria-current on the wrong row`);
        const cue = (n) => page.locator(`.slice-page .vp-risk:nth-child(${n}) .vp-risk-num`).evaluate((el) => {
          const s = getComputedStyle(el);
          return `${s.fontWeight} ${s.textDecorationLine}`;
        });
        const [on, off] = [await cue(i + 1), await cue(2 - i)];
        if (on !== "700 underline") throw new Error(`selected number is "${on}", not bold and underlined`);
        if (off !== "400 none") throw new Error(`unselected number is "${off}"`);
        for (const j of [0, 1]) {
          const pressed = await pins.nth(j).getAttribute("aria-pressed");
          if (pressed !== (j === i ? "true" : "false")) throw new Error(`pin ${j + 1} aria-pressed=${pressed} with pin ${i + 1} selected`);
        }
      }
      // Keys turn the view: yaw moves, and a tilt moves a pin on screen.
      const yaw0 = await page.locator(".slice-page .vp-canvas").getAttribute("data-yaw");
      const pinStyle0 = await pins.nth(1).getAttribute("style");
      await page.locator(".slice-page .vp-canvas").focus();
      await page.keyboard.press("ArrowRight");
      if ((await page.locator(".slice-page .vp-canvas").getAttribute("data-yaw")) === yaw0) throw new Error("arrow key did not rotate");
      await page.waitForFunction((s0) => document.querySelectorAll(".slice-page .vp-pin")[1].getAttribute("style") !== s0, pinStyle0, { timeout: 5000 });
    } else {
      if (hasCanvas || !stateText.includes("WebGL2")) throw new Error(`no WebGL2 here, expected the fallback text: ${stateText}`);
    }
    await page.click('.slice-page .vp-mode:has-text("Wireframe")');
    if ((await page.locator(".slice-page .vp").getAttribute("data-mode")) !== "wireframe") throw new Error("mode did not switch");
    if ((await page.locator('.slice-page .vp-mode:has-text("Wireframe")').getAttribute("aria-pressed")) !== "true") throw new Error("Wireframe is not pressed");
    if ((await page.locator('.slice-page .vp-mode:has-text("Dots")').getAttribute("aria-pressed")) !== "false") throw new Error("Dots is still pressed");
    return engineHasWebGl2 ? "webgl" : "fallback";
  });

  await step(run, page, "the 3D view leaves the drop zone and the file picker alone", async () => {
    const stage = page.locator(".slice-page .vp-canvas").or(page.locator(".slice-page .vp-stage")).first();
    const canvas = page.locator(".slice-page .vp-canvas");
    const picks = (await callsOf("slicer_pick_model")).length;
    if (engineHasWebGl2) {
      const box = await canvas.boundingBox();
      const yaw0 = await canvas.getAttribute("data-yaw");
      await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
      await page.mouse.down();
      await page.mouse.move(box.x + box.width / 2 + 40, box.y + box.height / 2 + 10, { steps: 5 });
      if ((await page.locator(".sl-drop.sl-drop-over").count()) !== 0) throw new Error("rotating lit the drop zone");
      await page.mouse.up();
      if ((await canvas.getAttribute("data-yaw")) === yaw0) throw new Error("dragging did not rotate");
      // A release we never hear about (a system gesture, a switch away) must
      // not leave the view turning: a move with no button held ends the
      // drag, and so does losing pointer capture.
      const synthetic = (steps) => canvas.evaluate((c, steps) => {
        const r = c.getBoundingClientRect();
        for (const [type, dx, buttons] of steps) {
          c.dispatchEvent(new PointerEvent(type, {
            pointerId: 7, isPrimary: true, bubbles: true, button: 0, buttons,
            clientX: r.x + r.width / 2 + dx, clientY: r.y + r.height / 2,
          }));
        }
      }, steps);
      const settle = () => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
      // The same synthetic events with the button held do turn the view.
      const yawHeld = await canvas.getAttribute("data-yaw");
      await synthetic([["pointerdown", 0, 1], ["pointermove", 60, 1], ["pointerup", 60, 0]]);
      await page.waitForFunction((y) => document.querySelector(".slice-page .vp-canvas").getAttribute("data-yaw") !== y, yawHeld, { timeout: 5000 });
      for (const [what, end] of [["a move with no button", ["pointermove", 5, 0]], ["lost capture", ["lostpointercapture", 0, 1]]]) {
        const yawA = await canvas.getAttribute("data-yaw");
        await synthetic([["pointerdown", 0, 1], end, ["pointermove", 60, 1], ["pointermove", 90, 1]]);
        await settle();
        if ((await canvas.getAttribute("data-yaw")) !== yawA) throw new Error(`the view kept turning after ${what}`);
      }
    }
    await page.click('.slice-page .vp-mode:has-text("Dots")');
    if ((await callsOf("slicer_pick_model")).length !== picks) throw new Error("the view opened the file picker");
    // A file dragged over, then dropped on, the view still reaches the drop zone.
    await stage.dispatchEvent("dragover");
    if ((await page.locator(".sl-drop.sl-drop-over").count()) !== 1) throw new Error("dragover on the view did not light the drop zone");
    await stage.dispatchEvent("dragleave");
    if ((await page.locator(".sl-drop.sl-drop-over").count()) !== 0) throw new Error("dragleave did not clear the drop zone");
    // The drop stages a different path; the viewport must ask for that one
    // and replace its canvas.
    const droppedPath = "/Users/runner/models/dropped.stl";
    await withFixtures({ slicer_stage_model: droppedPath }, async () => {
      await page.evaluate(() => document.querySelector(".slice-page .vp")?.setAttribute("data-before-drop", ""));
      const staged = (await callsOf("slicer_stage_model")).length;
      const dropped = await page.evaluateHandle(() => {
        const dt = new DataTransfer();
        dt.items.add(new File(["solid cube"], "dropped.stl", { type: "model/stl" }));
        return dt;
      });
      await stage.dispatchEvent("drop", { dataTransfer: dropped });
      if ((await callsOf("slicer_stage_model")).length !== staged + 1) {
        await page.waitForFunction((n) => window.__ipc.calls.filter((c) => c.cmd === "slicer_stage_model").length > n, staged, { timeout: 5000 });
      }
      await page.waitForFunction((p) => window.__ipc.calls.filter((c) => c.cmd === "model_view").at(-1)?.args.path === p, droppedPath, { timeout: 5000 });
      // A fresh viewport, not the old one still on screen.
      await page.waitForSelector(".slice-page .vp:not([data-before-drop]) .vp-risk", { timeout: 15000 });
    });
    // Back to the picked model for the steps that follow.
    await page.click(".sl-browse");
    await page.waitForFunction((p) => window.__ipc.calls.filter((c) => c.cmd === "model_view").at(-1)?.args.path === p, SLICE_MODEL, { timeout: 5000 });
    await page.waitForSelector(".slice-page .vp-risk", { timeout: 15000 });
    await page.waitForFunction(() => document.querySelector(".sl-model")?.innerText === "cube.stl", null, { timeout: 5000 });
  });

  await step(run, page, "without WebGL2 the view says so and the risk list still works", async () => {
    const fb = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    fb.on("pageerror", (e) => run.errors.push(`pageerror: ${e.message}`));
    fb.on("console", (m) => { if (m.type() === "error") run.errors.push(`console.error: ${m.text()}`); });
    try {
      await fb.addInitScript(installTauriMock, FIXTURES);
      await fb.addInitScript(() => {
        const real = HTMLCanvasElement.prototype.getContext;
        HTMLCanvasElement.prototype.getContext = function (kind, ...rest) {
          return kind === "webgl2" ? null : real.call(this, kind, ...rest);
        };
      });
      await fb.goto(baseUrl, { waitUntil: "domcontentloaded" });
      await fb.waitForSelector(".sidebar", { timeout: 45000 });
      await fb.click('a[href="/slice"]');
      await fb.waitForSelector(".slice-page.nd", { timeout: 15000 });
      await fb.click(".sl-browse");
      await fb.waitForSelector(".slice-page .vp-risk", { timeout: 15000 });
      await fb.waitForFunction(() => [...document.querySelectorAll(".slice-page .vp .nd-state")].some((e) => e.textContent.includes("3D preview needs WebGL2")), null, { timeout: 5000 });
      if ((await fb.locator(".slice-page .vp-canvas").count()) !== 0) throw new Error("a canvas stayed");
      if ((await fb.locator(".slice-page .vp-pin").count()) !== 0) throw new Error("pins without a canvas");
      if ((await fb.locator(".slice-page .vp-risk").count()) !== 2) throw new Error("the risk list is gone");
      await fb.click('.slice-page .vp-mode:has-text("Wireframe")');
      if ((await fb.locator(".slice-page .vp").getAttribute("data-mode")) !== "wireframe") throw new Error("the mode toggle does not work");
      if ((await fb.locator(".slice-page .vp .nd-state", { hasText: "couldn't start" }).count()) !== 0) throw new Error("said couldn't start");
    } finally {
      await fb.close();
    }
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

  // The line under the plate's filament follows the printer's trays.
  const emitView = (v) => page.evaluate((x) => window.__emit("printer://state", x), v);
  const amsLine = (text) =>
    page.waitForFunction((t) => document.querySelector(".sl-ams")?.innerText.trim() === t, text, { timeout: 5000 });
  const amsTone = () => page.locator(".sl-ams").first().getAttribute("data-match");
  /** PRINTER_VIEW with some slots patched; a `tray` patch merges into the tray. */
  const slotsWith = (patch) => ({
    ...PRINTER_VIEW,
    slots: PRINTER_SLOTS.map((s) =>
      patch[s.label] ? { ...s, ...patch[s.label], tray: { ...s.tray, ...(patch[s.label].tray ?? {}) } } : s
    ),
  });

  await step(run, page, "the result says whether the job's filament is in the AMS", async () => {
    // GFA00 is in A2, A4 and B1, none of them the job's green.
    await emitView(PRINTER_VIEW);
    await amsLine("✓ Loaded in A2 · different colour");
    if ((await amsTone()) !== "loaded") throw new Error("not marked loaded");
    const otherPla = { tray: { tray_info_idx: "GFA01" }, detected: null };
    await emitView(slotsWith({ A2: otherPla, A4: otherPla, B1: otherPla }));
    await amsLine("~ PLA in A2, different profile: check temperatures · different colour");
    if ((await amsTone()) !== "same") throw new Error("not marked same material");
    const petg = { tray: { tray_type: "PETG", tray_info_idx: "GFG99" }, material: "petg", detected: null };
    await emitView(slotsWith({ A2: petg, A4: petg, B1: petg, B2: petg, "Ext-L": petg }));
    await amsLine("✗ Not loaded: load PLA before printing");
    if ((await amsTone()) !== "missing") throw new Error("not marked missing");
    // A filament that isn't loaded is not an error: plain text, never the accent.
    const [line, plain, accent] = await page.evaluate(() => {
      const probe = document.createElement("span");
      probe.style.color = "var(--nd-accent)";
      document.body.append(probe);
      const out = [
        getComputedStyle(document.querySelector(".sl-ams")).color,
        getComputedStyle(document.querySelector(".sl-filament")).color,
        getComputedStyle(probe).color,
      ];
      probe.remove();
      return out;
    });
    if (line !== plain || line === accent) throw new Error(`not loaded drawn in ${line}`);
    // Disconnected: the stale trays are not used.
    await page.evaluate(() => window.__emit("printer://connection", { state: "unreachable" }));
    await amsLine("Can't check the AMS: printer not connected");
    if ((await amsTone()) !== "unknown") throw new Error("not marked unknown");
    await emitView(PRINTER_VIEW);
    await amsLine("✓ Loaded in A2 · different colour");
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

  await step(run, page, "agent context follows model, presets, job and selected plate", async () => {
    await page.waitForFunction(() => {
      const state = window.__ipc.calls.filter((c) => c.cmd === "agent_set_app_state").at(-1)?.args.state;
      return state?.slice?.selected_job_id === 9 && state.slice.selected_plate === 2;
    }, null, { timeout: 5000 });
    const state = (await callsOf("agent_set_app_state")).at(-1).args.state;
    if (state.route !== "/slice" || state.slice.model_path !== SLICE_MODEL) throw new Error(JSON.stringify(state));
    for (const [key, id] of [["printer","sl-printer"], ["process","sl-process"], ["filament","sl-filament"], ["bed_type","sl-bed"]]) {
      const picker = page.locator(`#${id}`);
      if (await picker.count() && state.slice[key] !== await picker.inputValue()) throw new Error(`context differs from ${id}`);
    }
    if (!state.slice.compare_filaments.includes(PETG)) throw new Error("comparison context missing");
    await page.click(".agent-toggle");
    await page.waitForSelector(".agent-drawer.open", { timeout: 5000 });
    await page.waitForFunction(() => document.querySelector(".ag-context")?.textContent.includes("Plate 2"));
    await page.waitForFunction(() => Number(getComputedStyle(document.querySelector(".agent-drawer")).opacity) === 1);
    await page.screenshot({ path: `flow-${engine}-slice-agent.png`, fullPage: false });
    await page.click(".ag-close");
    await openJob(2);
    await page.waitForFunction(() => {
      const s = window.__ipc.calls.filter((c) => c.cmd === "agent_set_app_state").at(-1)?.args.state.slice;
      return s?.selected_job_id === 2 && s.selected_plate === 1;
    }, null, { timeout: 5000 });
    await emitJob(sliceJob(2, { state:"failed", error:{ kind:"slicer", message:"Fixture failure" } }));
    await page.waitForFunction(() => window.__ipc.calls.filter((c) => c.cmd === "agent_set_app_state").at(-1)?.args.state.slice?.selected_plate == null, null, { timeout:5000 });
    await emitJob(done(2, sliceResult()));
  });

  await step(run, page, "leaving the page mid-job is safe and the job finishes", async () => {
    await page.click(".sl-compare-toggle");
    await page.click(".sl-slice");
    await page.waitForFunction(() => window.__ipc.calls.filter((c) => c.cmd === "slicer_slice").length === 4, null, { timeout: 5000 });
    await emitJob(running(4, 10));
    await goTo("/settings/about");
    await page.waitForSelector(".about-page", { timeout: 15000 });
    await page.waitForFunction(() => {
      const s = window.__ipc.calls.filter((c) => c.cmd === "agent_set_app_state").at(-1)?.args.state;
      return s?.route === "/settings/about" && s.slice == null;
    }, null, { timeout: 5000 });
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
      await goTo("/settings/about");
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
    await goTo("/settings/about");
    await page.waitForSelector(".about-page", { timeout: 15000 });
    const settings = await page.evaluate(() => window.__fixtures.slicer_get_settings);
    const late = { __hold: "away-settings", answer: { ...settings, effective: { ...settings.effective, printer: null } } };
    await withFixtures({ slicer_get_settings: late }, async () => {
      const presets = await callCount("slicer_presets");
      const loads = await callCount("slicer_get_settings");
      const stls = await settledCount("list_received_stls");
      await page.click('a[href="/slice"]');
      await calledPast("slicer_get_settings", loads);
      await goTo("/settings/about");
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

  // -- filament advisor ------------------------------------------------------
  // Every wait here is on the invoke log, a DOM condition or a __hold gate;
  // none is on a clock. Fixture overrides go through withFixtures, so each
  // step puts back what it changed.
  const ADV = ".advise-page";
  const countOf = (cmd) => page.evaluate((c) => window.__ipc.calls.filter((x) => x.cmd === c).length, cmd);
  const pastCalls = (cmd, n) =>
    page.waitForFunction(([c, k]) => window.__ipc.calls.filter((x) => x.cmd === c).length > k, [cmd, n], { timeout: 5000 });
  // Advise is aria-disabled, never disabled, while a request is out.
  const adviseIdle = () => page.waitForSelector(`${ADV} .adv-run[aria-disabled="false"]`, { timeout: 10000 });
  const openAdvise = async () => {
    await openTab("/filament", "/filament/advise");
    await page.waitForSelector(`${ADV} .adv-run`, { timeout: 15000 });
    await adviseIdle();
  };
  // Presses Advise and returns once that answer has been handled.
  const pressAdvise = async () => {
    const settled = await settledCount("advise");
    await page.click(`${ADV} .adv-run`);
    await page.waitForFunction((k) => window.__ipc.settled.filter((c) => c === "advise").length > k, settled, { timeout: 10000 });
    await adviseIdle();
  };
  const pressedChips = async () => (await page.locator(`${ADV} .adv-chip[aria-pressed="true"]`).allTextContents()).join("|");
  const chipNamed = (label) => page.locator(`${ADV} .adv-chip`, { hasText: label });
  const statusText = async () => (await page.locator(`${ADV} .adv-status`).textContent()).trim();
  const ASA_ROW = (over = {}) => ({ ...ADVICE.fits[0], ...over });

  await step(run, page, "Suggest filament on Slice opens Advise with the model", async () => {
    await page.click('nav.sidebar a[href="/slice"]');
    await page.waitForSelector(".slice-page", { timeout: 15000 });
    await page.click(".sl-browse");
    await page.waitForFunction(() => document.querySelector(".sl-model")?.innerText === "cube.stl", null, { timeout: 5000 });
    const views = (await callsOf("model_view")).length;
    await page.click(".sl-suggest");
    await page.waitForFunction(() => location.pathname === "/filament/advise", null, { timeout: 5000 });
    const model = await page.evaluate(() => new URLSearchParams(location.search).get("model"));
    if (model !== SLICE_MODEL) throw new Error(`?model= is ${model}`);
    await page.waitForSelector(`${ADV} .adv-model-name`, { timeout: 15000 });
    const name = (await page.locator(".adv-model-name").textContent()).trim();
    if (name !== "cube.stl") throw new Error(`model row reads "${name}"`);
    await page.waitForFunction(
      () => document.querySelector(".adv-model-line")?.textContent === "10 × 10 mm · Overhang 64 mm² · Tall and thin 6.0:1",
      null,
      { timeout: 5000 }
    );
    const asked = (await callsOf("model_view")).slice(views).map((c) => c.args.path);
    if (!asked.includes(SLICE_MODEL)) throw new Error(`model_view asked for ${JSON.stringify(asked)}`);
    const current = await page.locator('.nd-tabs a[aria-current="page"]').getAttribute("href");
    if (current !== "/filament/advise") throw new Error(`the current tab is ${current}`);
    const tabs = await page.locator(".nd-tabs a").allTextContents();
    if (tabs.join("|") !== "Advise|Search|Batch|Calibrate") throw new Error(`tabs read ${tabs.join("|")}`);
  });

  await step(run, page, "before advising the page says what to do", async () => {
    await page.waitForSelector(`${ADV} .adv-description`, { timeout: 10000 });
    const text = (await page.locator(`${ADV} .nd-state[data-kind="empty"]`).textContent()).trim();
    if (text !== "[EMPTY: Describe the part or pick what it needs]") throw new Error(`reads "${text}"`);
    if (await page.locator(`${ADV} .adv-results`).count()) throw new Error("results are shown before advising");
  });

  await step(run, page, "the needs chips toggle and the heat options exclude each other", async () => {
    await chipNamed("Strong").click();
    await chipNamed("Heat ≥60 °C").click();
    await chipNamed("Heat ≥100 °C").click();
    const pressed = await pressedChips();
    if (pressed !== "Strong|Heat ≥100 °C") throw new Error(`pressed: ${pressed}`);
    await chipNamed("Strong").click();
    await chipNamed("Heat ≥100 °C").click();
    const left = await page.locator(`${ADV} .adv-chip[aria-pressed="true"]`).count();
    if (left !== 0) throw new Error(`${left} chips stayed on`);
    if ((await countOf("advise")) !== 0) throw new Error("toggling a chip asked the backend");
  });

  await step(run, page, "a description fills the chips and the hero shows the top material", async () => {
    await page.fill(`${ADV} .adv-description`, "A sensor bracket on a hot car dashboard");
    await pressAdvise();
    await page.waitForSelector(`${ADV} .adv-results .nd-hero-number`, { timeout: 10000 });
    const interp = (await callsOf("advise_interpret")).at(-1)?.args;
    if (interp?.text !== "A sensor bracket on a hot car dashboard") throw new Error(JSON.stringify(interp));
    const pressed = await pressedChips();
    if (pressed !== "Heat ≥80 °C|Outdoor/UV") throw new Error(`pressed: ${pressed}`);
    const advise = (await callsOf("advise")).at(-1).args;
    if (!sameArgs(advise, { needs: ["heat_80", "outdoor_uv"], modelPath: SLICE_MODEL })) throw new Error(JSON.stringify(advise));
    const word = (await page.locator(`${ADV} .adv-results .nd-hero-number`).textContent()).trim();
    if (word !== "ASA") throw new Error(`hero reads "${word}"`);
    const detail = (await page.locator(`${ADV} .adv-results .nd-hero-detail`).first().textContent()).trim();
    if (detail !== "Fit 100% · ✓ Heat ≥80 °C · ✓ Outdoor/UV") throw new Error(`detail reads "${detail}"`);
    const tone = await page.locator(`${ADV} .adv-results .nd-hero`).getAttribute("data-tone");
    if (tone !== "signal") throw new Error(`a fit with no unmet need has the ${tone} tone`);
    const ranks = await page.locator(`${ADV} .adv-rank`).allTextContents();
    if (ranks.join("|") !== "1 · ASA|2 · PA-CF|3 · PET-CF") throw new Error(`ranks: ${ranks.join("|")}`);
  });

  await step(run, page, "the live region carries the result summary", async () => {
    const region = page.locator(`${ADV} .adv-status`);
    if ((await region.getAttribute("role")) !== "status" || (await region.getAttribute("aria-live")) !== "polite") {
      throw new Error("the status line is not a polite live region");
    }
    const text = await statusText();
    if (text !== "Best fit ASA, fit 100%. Also PA-CF, PET-CF.") throw new Error(`reads "${text}"`);
  });

  await step(run, page, "choosing rank 2 changes the reasons", async () => {
    const reasons = () => page.locator(`${ADV} .adv-reasons .nd-stat-row-value`).allTextContents();
    const first = await reasons();
    if (first.join("|") !== "100%|✓ heat deflection ≈95 °C|✓ 3/3") throw new Error(`rank 1: ${first.join("|")}`);
    await page.locator(`${ADV} .adv-rank`).nth(1).click();
    await page.waitForFunction(() => document.querySelector(".adv-detail")?.dataset.material === "pa_cf", null, { timeout: 5000 });
    const second = await reasons();
    if (second.join("|") !== "83%|✓ heat deflection ≈120 °C|~ 2/3") throw new Error(`rank 2: ${second.join("|")}`);
    if ((await page.locator(`${ADV} .adv-rank`).nth(1).getAttribute("aria-pressed")) !== "true") throw new Error("rank 2 is not pressed");
    if ((await page.locator(`${ADV} .adv-rank`).nth(0).getAttribute("aria-pressed")) !== "false") throw new Error("rank 1 stayed pressed");
    const notes = await page.locator(`${ADV} .adv-notes li`).allTextContents();
    if (notes.join("|") !== "Supports likely, whatever the material|Dry the spool before printing.") throw new Error(`notes: ${notes.join("|")}`);
    const none = (await page.locator(`${ADV} .adv-detail .nd-state[data-kind="empty"]`).textContent()).trim();
    if (none !== "[EMPTY: No PA-CF filaments in the catalog]") throw new Error(`products read "${none}"`);
    // One hero still: the pills switch the panel, they do not add a second.
    if ((await page.locator(`${ADV} .nd-hero`).count()) !== 1) throw new Error("more than one hero");
    await page.locator(`${ADV} .adv-rank`).nth(0).click();
    await page.waitForFunction(() => document.querySelector(".adv-detail")?.dataset.material === "asa", null, { timeout: 5000 });
  });

  await step(run, page, "AMS-loaded products come first and the order is stated", async () => {
    const statuses = await page.locator(`${ADV} .adv-product`).evaluateAll((rows) => rows.map((r) => r.dataset.status));
    if (statuses.join("|") !== "loaded|preferred|catalog") throw new Error(`rows: ${statuses.join("|")}`);
    const tag = (await page.locator(`${ADV} .adv-product`).first().locator(".adv-tag").textContent()).trim();
    if (tag !== "Loaded in A2") throw new Error(`tag reads "${tag}"`);
    const brands = await page.locator(`${ADV} .adv-product-brand`).allTextContents();
    if (brands.join("|") !== "Bambu Lab|Polymaker|Sunlu") throw new Error(`brands: ${brands.join("|")}`);
    const rule = (await page.locator(`${ADV} .adv-order-rule`).textContent()).trim();
    if (!rule.startsWith("Loaded in your AMS first, then your preferred brands, then the rest A–Z.")) throw new Error(`rule reads "${rule}"`);
  });

  await step(run, page, "Where to buy opens the plain link", async () => {
    const before = (await callsOf("open_external_url")).length;
    await page.locator(`${ADV} .adv-product`).nth(1).locator(".adv-buy").click();
    await page.waitForFunction((n) => window.__ipc.calls.filter((c) => c.cmd === "open_external_url").length > n, before, { timeout: 5000 });
    const url = (await callsOf("open_external_url")).at(-1).args.url;
    if (url !== "https://www.spoolscout.com/data-sheets/polymaker/asa-asa") throw new Error(`opened ${url}`);
    if ((await page.locator(`${ADV} .adv-buy-error`).count()) !== 0) throw new Error("an error shows after a link that opened");
  });

  await step(run, page, "a failed Where to buy shows its inline sentence", async () => {
    await withFixtures({ open_external_url: { __reject: "no browser: /secret/path" } }, async () => {
      const before = await countOf("open_external_url");
      const row = page.locator(`${ADV} .adv-product`).nth(2);
      await row.locator(".adv-buy").click();
      await pastCalls("open_external_url", before);
      await row.locator(".adv-buy-error").waitFor({ timeout: 5000 });
      const text = (await row.locator(".adv-buy-error").textContent()).trim();
      if (text !== "Couldn't open that link in your browser.") throw new Error(`reads "${text}"`);
      if ((await row.locator(".adv-buy-error").getAttribute("role")) !== "alert") throw new Error("the error is not an alert");
      if ((await row.innerText()).includes("secret")) throw new Error("the raw error leaked into the row");
      // The other rows are untouched.
      if ((await page.locator(`${ADV} .adv-buy-error`).count()) !== 1) throw new Error("the error shows on other rows");
    });
  });

  await step(run, page, "Generate profile hands the entry to Search exactly once and installs nothing", async () => {
    const fetches = await countOf("fetch_filament_from_catalog");
    const installs = await countOf("install_generated_profile");
    await page.locator(`${ADV} .adv-product`).nth(1).locator(".adv-generate").click();
    await page.waitForFunction(() => location.pathname === "/filament", null, { timeout: 5000 });
    await page.waitForSelector(".filament-search-page", { timeout: 15000 });
    await pastCalls("fetch_filament_from_catalog", fetches);
    const entry = (await callsOf("fetch_filament_from_catalog")).at(-1).args.entry;
    if (entry.brand !== "Polymaker" || entry.name !== "ASA") throw new Error(JSON.stringify(entry));
    // The pick is spent: leaving Search and coming back hands over nothing.
    await openAdvise();
    await page.click('.nd-tabs a[href="/filament"]');
    await page.waitForSelector(".filament-search-page", { timeout: 15000 });
    if ((await countOf("fetch_filament_from_catalog")) !== fetches + 1) throw new Error("the entry was handed over more than once");
    if ((await countOf("install_generated_profile")) !== installs) throw new Error("something was installed");
  });

  await step(run, page, "Where to buy and Generate profile show only for a product that has them", async () => {
    const bare = { ...product("Sunlu", "catalog", null), buy_url: null, entry: null };
    const noEntry = { ...product("Polymaker", "preferred", null), entry: null };
    const advice = { ...ADVICE, fits: [ASA_ROW({ products: [bare, noEntry] })] };
    await withFixtures({ advise: advice }, async () => {
      await openAdvise();
      await pressAdvise();
      await page.waitForSelector(`${ADV} .adv-product`, { timeout: 5000 });
      const rows = page.locator(`${ADV} .adv-product`);
      if ((await rows.count()) !== 2) throw new Error(`${await rows.count()} product rows`);
      const first = `${await rows.nth(0).locator(".adv-buy").count()}${await rows.nth(0).locator(".adv-generate").count()}`;
      const second = `${await rows.nth(1).locator(".adv-buy").count()}${await rows.nth(1).locator(".adv-generate").count()}`;
      if (first !== "00") throw new Error(`a product with no link or entry offers buy/generate: ${first}`);
      if (second !== "10") throw new Error(`a product with a link and no entry offers buy/generate: ${second}`);
    });
  });

  await step(run, page, "a double press of Advise sends one request", async () => {
    await openAdvise();
    const before = await countOf("advise");
    await withFixtures({ advise: { __hold: "adv-double", answer: ADVICE } }, async () => {
      // Both clicks land before the page can answer. Playwright's own click
      // would refuse the aria-disabled button, so dispatch them directly.
      await page.evaluate(() => {
        const button = document.querySelector(".advise-page .adv-run");
        button.click();
        button.click();
      });
      await pastCalls("advise", before);
      if ((await page.locator(`${ADV} .adv-run`).getAttribute("aria-disabled")) !== "true") throw new Error("Advise stayed enabled while a request was out");
      if ((await page.locator(`${ADV} .adv-status .nd-state[data-kind="loading"]`).count()) !== 1) throw new Error("no loading line while a request is out");
      await page.evaluate(() => window.__release("adv-double"));
      await page.waitForSelector(`${ADV} .adv-results .nd-hero-number`, { timeout: 5000 });
      await adviseIdle();
    });
    const sent = (await countOf("advise")) - before;
    if (sent !== 1) throw new Error(`${sent} requests for two presses`);
  });

  await step(run, page, "a chip edit survives the next Advise and the description is not read again", async () => {
    await openAdvise();
    await page.fill(`${ADV} .adv-description`, "A gate hinge outdoors");
    const reads = await countOf("advise_interpret");
    await pressAdvise();
    if ((await countOf("advise_interpret")) !== reads + 1) throw new Error("the description was not read once");
    await chipNamed("Strong").click();
    const edited = { ...ADVICE, needs: ["strong", "heat_80", "outdoor_uv"] };
    await withFixtures({ advise: edited }, async () => {
      await pressAdvise();
    });
    if ((await countOf("advise_interpret")) !== reads + 1) throw new Error("an unchanged description was read again");
    const sent = (await callsOf("advise")).at(-1).args.needs;
    if (JSON.stringify(sent) !== '["strong","heat_80","outdoor_uv"]') throw new Error(`advise got ${JSON.stringify(sent)}`);
    const pressed = await pressedChips();
    if (pressed !== "Strong|Heat ≥80 °C|Outdoor/UV") throw new Error(`pressed: ${pressed}`);
    // A new description is read, and it fills the chips again.
    await page.fill(`${ADV} .adv-description`, "A gate hinge outdoors, in the sun");
    await pressAdvise();
    if ((await countOf("advise_interpret")) !== reads + 2) throw new Error("a changed description was not read");
    const reread = await pressedChips();
    if (reread !== "Heat ≥80 °C|Outdoor/UV") throw new Error(`pressed after the new reading: ${reread}`);
  });

  await step(run, page, "the hero is neutral when the top fit misses a need and signal when it misses none", async () => {
    const unmet = ASA_ROW({
      fit_pct: 56,
      reasons: [reason("heat_80", "met", "heat deflection ≈95 °C"), reason("flexible", "not_met", "0/3")],
    });
    await withFixtures({ advise: { ...ADVICE, needs: ["flexible", "heat_80"], fits: [unmet] } }, async () => {
      await openAdvise();
      await pressAdvise();
      await page.waitForSelector(`${ADV} .adv-results .nd-hero-number`, { timeout: 5000 });
      const tone = await page.locator(`${ADV} .adv-results .nd-hero`).getAttribute("data-tone");
      if (tone !== "neutral") throw new Error(`a fit with an unmet need has the ${tone} tone`);
      const rows = await page.locator(`${ADV} .adv-reasons .nd-stat-row-value`).allTextContents();
      if (rows.join("|") !== "56%|✓ heat deflection ≈95 °C|✗ 0/3") throw new Error(`rows: ${rows.join("|")}`);
    });
    await pressAdvise();
    await page.waitForSelector(`${ADV} .adv-results .nd-hero[data-tone="signal"]`, { timeout: 5000 });
  });

  await step(run, page, "when nothing fits the page says so and Ruled out opens with every reason", async () => {
    const out = (material, name, ...reasons) => ({ material, name, reasons });
    const none = {
      ...ADVICE,
      fits: [],
      ruled_out: [
        out("pla", "PLA", "Needs an enclosed printer", "Softens around 55 °C"),
        out("petg", "PETG", "Softens around 70 °C"),
      ],
    };
    await withFixtures({ advise: none }, async () => {
      await openAdvise();
      await pressAdvise();
      await page.waitForSelector(`${ADV} .adv-excluded`, { timeout: 5000 });
      const text = await statusText();
      if (text !== "[EMPTY: Nothing on this printer meets all of that.]") throw new Error(`status reads "${text}"`);
      if (await page.locator(`${ADV} .nd-hero`).count()) throw new Error("a hero is shown with nothing that fits");
      if (await page.locator(`${ADV} .adv-rank`).count()) throw new Error("rank pills are shown with nothing that fits");
      if (!(await page.locator(`${ADV} details.adv-excluded`).evaluate((d) => d.open))) throw new Error("Ruled out is closed");
      const summary = (await page.locator(`${ADV} .adv-excluded summary`).textContent()).trim();
      if (summary !== "Ruled out (2)") throw new Error(`summary reads "${summary}"`);
      const rows = (await page.locator(`${ADV} .adv-excluded li`).allTextContents()).map((t) => t.replace(/\s+/g, " ").trim());
      if (rows.join("|") !== "PLA Needs an enclosed printer; Softens around 55 °C|PETG Softens around 70 °C") throw new Error(`rows: ${rows.join("|")}`);
      if ((await page.locator(`${ADV} .adv-excluded li:visible`).count()) !== 2) throw new Error("the reasons are not visible");
    });
  });

  await step(run, page, "Ruled out stays closed when something fits", async () => {
    const some = { ...ADVICE, ruled_out: [{ material: "pla", name: "PLA", reasons: ["Softens around 55 °C"] }] };
    await withFixtures({ advise: some }, async () => {
      await openAdvise();
      await pressAdvise();
      await page.waitForSelector(`${ADV} .adv-excluded`, { timeout: 5000 });
      if (await page.locator(`${ADV} details.adv-excluded`).evaluate((d) => d.open)) throw new Error("Ruled out opened with a fit shown");
      if ((await page.locator(`${ADV} .adv-excluded summary`).textContent()).trim() !== "Ruled out (1)") throw new Error("wrong count");
    });
  });

  await step(run, page, "an empty catalog asks for a refresh in place of the products", async () => {
    const empty = { ...ADVICE, catalog_empty: true, fits: [ASA_ROW({ products: [] })] };
    await withFixtures({ advise: empty }, async () => {
      await openAdvise();
      await pressAdvise();
      await page.waitForSelector(`${ADV} .adv-detail`, { timeout: 5000 });
      const text = (await page.locator(`${ADV} .adv-detail .nd-state[data-kind="empty"]`).textContent()).trim();
      if (text !== "[EMPTY: The filament catalog is empty. Refresh it on the Search tab.]") throw new Error(`reads "${text}"`);
      if (await page.locator(`${ADV} .adv-products`).count()) throw new Error("a products table is shown with an empty catalog");
    });
  });

  await step(run, page, "a failed description read still scores from the chips", async () => {
    await withFixtures({ advise_interpret: { __reject: "Couldn't read the description; pick the needs below." } }, async () => {
      await openAdvise();
      await page.fill(`${ADV} .adv-description`, "something");
      const before = await countOf("advise");
      await page.click(`${ADV} .adv-run`);
      await page.waitForSelector(`${ADV} .adv-ai-error`, { timeout: 5000 });
      const text = (await page.locator(`${ADV} .adv-ai-error`).textContent()).trim();
      if (text !== "Couldn't read the description; pick the needs below.") throw new Error(`reads "${text}"`);
      await pastCalls("advise", before);
      await page.waitForSelector(`${ADV} .adv-results .nd-hero-number`, { timeout: 5000 });
      // Pressing Advise again scores the chips; the same failed text isn't re-read.
      const reads = await countOf("advise_interpret");
      const scored = await countOf("advise");
      await adviseIdle();
      await page.click(`${ADV} .adv-run`);
      await pastCalls("advise", scored);
      if ((await countOf("advise_interpret")) !== reads) throw new Error("a failed description was read again");
    });
  });

  await step(run, page, "with no AI the description field is hidden", async () => {
    await withFixtures({ get_ai_capabilities: { provider: null, model_id: null, text: false, vision: false } }, async () => {
      await openTab("/filament", "/filament/advise");
      await page.waitForSelector(`${ADV} .adv-ai-off`, { timeout: 10000 });
      if (await page.locator(`${ADV} .adv-description`).count()) throw new Error("the description field is shown without AI");
      // The chips still work and Advise scores them without reading anything.
      const reads = await countOf("advise_interpret");
      await chipNamed("Strong").click();
      await pressAdvise();
      if ((await countOf("advise_interpret")) !== reads) throw new Error("a description was read without AI");
      await page.waitForSelector(`${ADV} .adv-results .nd-hero-number`, { timeout: 5000 });
    });
  });

  // -- print journal -----------------------------------------------------------
  // Printer → Journal. Waits are on the invoke log or the DOM, never a clock.
  const JR = ".journal-page";
  const journalEmit = (name, payload) => page.evaluate(([n, p]) => window.__emit(n, p), [name, payload]);
  const journalRow = (id) => `${JR} .jr-row[data-entry="${id}"]`;
  const openJournal = async () => {
    await openTab("/printer", "/printer/journal");
    await page.waitForSelector(`${JR} .jr-row`, { timeout: 15000 });
  };
  const rowNames = () => page.locator(`${JR} .jr-row .jr-name`).allInnerTexts();
  const rowsRead = (names) =>
    page.waitForFunction(
      (n) => [...document.querySelectorAll(".journal-page .jr-row .jr-name")].map((e) => e.textContent).join("|") === n,
      names.join("|"),
      { timeout: 5000 }
    );
  const ALL_ROWS = [JOURNAL_FINISHED.name, JOURNAL_FAILED.name, JOURNAL_GOOD.name, JOURNAL_UNKNOWN.name];

  await step(run, page, "the journal lists prints newest first under this month's hero", async () => {
    await setFixture("printer_view", PRINTER_VIEW);
    await openJournal();
    const tabs = await page.locator('.hub[data-hub="printer"] .nd-tabs a').evaluateAll((as) => as.map((a) => a.textContent.trim()));
    if (tabs.join("|") !== "Live|Journal") throw new Error(`tabs: ${tabs.join("|")}`);
    if ((await page.locator('.hub[data-hub="printer"] h1').count()) !== 1) throw new Error("expected one page title");
    if ((await page.locator(`${JR} .nd-hero`).count()) !== 1) throw new Error("expected one hero");
    const text = (sel) => page.locator(`${JR} ${sel}`).evaluate((el) => el.textContent.trim());
    if ((await text(".nd-hero-label")) !== "This month") throw new Error("hero label");
    if ((await text(".nd-hero-number")) !== "5" || (await text(".nd-hero-unit")) !== "prints") throw new Error("hero value");
    const detail = await text(".nd-hero-detail");
    if (detail !== "1 good · 1 finished · 1 failed · 4 h printing") throw new Error(`hero detail "${detail}"`);
    const now = await page.locator(`${JR} .jr-now`).evaluate((el) => el.textContent);
    if (!now.includes("Printing now") || !now.includes(JOURNAL_PRINTING.name)) throw new Error(`printing now reads "${now}"`);
    if ((await page.locator(`${JR} .jr-now a[href="/printer"]`).count()) !== 1) throw new Error("printing now doesn't link to Live");
    const names = await rowNames();
    if (names.join("|") !== ALL_ROWS.join("|")) throw new Error(`rows: ${names.join("|")}`);
    const chips = await page.locator(`${journalRow(2)} .jr-chip`).allInnerTexts();
    if (chips.map((c) => c.trim()).join("|") !== "PLA · A2|PETG-CF · B3") throw new Error(`chips: ${chips.join("|")}`);
    const swatch = await page.locator(`${journalRow(2)} .jr-swatch`).first().evaluate((el) => getComputedStyle(el).backgroundColor);
    if (swatch !== "rgb(255, 255, 255)") throw new Error(`the A2 swatch is ${swatch}`);
    if ((await page.locator(`${journalRow(2)} .jr-outcome`).getAttribute("data-outcome")) !== "finished") throw new Error("outcome");
    if ((await page.locator(`${journalRow(2)} .jr-verdict-tag`).innerText()).trim() !== "") throw new Error("an unrated print shows a verdict");
    const verdictOf = (id) => page.locator(`${journalRow(id)} .jr-verdict-tag`).evaluate((el) => el.textContent.replace(/\s+/g, " ").trim());
    if ((await verdictOf(1)) !== "Verdict Failed") throw new Error(`verdict reads "${await verdictOf(1)}"`);
    if ((await verdictOf(4)) !== "Verdict Good") throw new Error(`the good verdict reads "${await verdictOf(4)}"`);
    if ((await page.locator(`${journalRow(2)} .jr-thumb`).count()) !== 1 || (await page.locator(`${JR} .jr-row-main.jr-has-thumb`).count()) !== 1) throw new Error("only the linked print has a thumbnail column");
    const unknown = (await page.locator(`${journalRow(5)} .jr-outcome`).innerText()).trim();
    if (unknown.toLowerCase() !== "ended while bambumate wasn't watching") throw new Error(`unknown reads "${unknown}"`);
    const approx = await page.locator(`${journalRow(1)} .jr-when`).innerText();
    if (!approx.includes("(approximate)")) throw new Error(`an approximate start reads "${approx}"`);
    if ((await page.locator(`${journalRow(2)} .jr-when`).innerText()).includes("(approximate)")) throw new Error("an exact start is marked approximate");
    await page.waitForSelector(`${journalRow(2)} .jr-thumb img`, { timeout: 5000 });
    const thumbs = await callsOf("journal_image");
    if (!thumbs.length || thumbs.some((c) => c.args.id !== 2 || c.args.photoId != null)) throw new Error(`thumbnails asked for: ${JSON.stringify(thumbs.map((c) => c.args))}`);
    if ((await page.locator(`${journalRow(1)} .jr-thumb`).count()) !== 0) throw new Error("a thumbnail without a slice");
    const sidebarActive = await page.locator('nav.sidebar a[href="/printer"]').getAttribute("aria-current");
    if (sidebarActive !== "page") throw new Error("the rail doesn't mark Printer");
  });

  await step(run, page, "a journal row opens its detail in place, one at a time", async () => {
    const path = await page.evaluate(() => location.pathname);
    const main = (id) => page.locator(`${journalRow(id)} .jr-row-main`);
    if ((await main(2).evaluate((el) => el.tagName)) !== "BUTTON") throw new Error("the row is not a button");
    if ((await main(2).getAttribute("aria-expanded")) !== "false") throw new Error("a row starts open");
    await main(2).click();
    await page.waitForSelector(`${journalRow(2)} .jr-detail`, { timeout: 5000 });
    if ((await main(2).getAttribute("aria-expanded")) !== "true") throw new Error("not marked expanded");
    const controls = await main(2).getAttribute("aria-controls");
    if (!controls || (await page.locator(`${journalRow(2)} .jr-detail`).getAttribute("id")) !== controls) throw new Error("aria-controls names no detail");
    if (await page.locator('[role="dialog"]').count()) throw new Error("the detail opened a dialog");
    if ((await page.evaluate(() => location.pathname)) !== path) throw new Error("the detail changed the page");
    await main(1).click();
    await page.waitForSelector(`${journalRow(1)} .jr-detail`, { timeout: 5000 });
    if (await page.locator(`${journalRow(2)} .jr-detail`).count()) throw new Error("two entries open at once");
    if ((await main(2).getAttribute("aria-expanded")) !== "false") throw new Error("the closed row is still marked expanded");
    await main(1).click();
    await page.waitForSelector(`${journalRow(1)} .jr-detail`, { state: "detached", timeout: 5000 });
  });

  await step(run, page, "a calibration print shows a plain chip in its row and a link in its open entry", async () => {
    const linked = { ...JOURNAL_FINISHED, purpose: { calibration_result_id: 11, test: "temperature" } };
    await withFixtures({ journal_list: [linked] }, async () => {
      const lists = await countOf("journal_list");
      await page.click(`${JR} .jr-filter[data-kind="needs_verdict"]`);
      await pastCalls("journal_list", lists);
      await rowsRead([JOURNAL_FINISHED.name]);
      const chip = page.locator(`${journalRow(2)} .jr-row-main .jr-purpose`);
      if ((await chip.innerText()).trim() !== "Calibration: Temperature") throw new Error(`chip reads "${await chip.innerText()}"`);
      if ((await page.locator(`${journalRow(2)} .jr-row-main a`).count()) !== 0) throw new Error("the row button holds a link");
      await page.click(`${journalRow(2)} .jr-row-main`);
      await page.waitForSelector(`${journalRow(2)} .jr-detail`, { timeout: 5000 });
      const link = page.locator(`${journalRow(2)} .jr-detail a.jr-purpose-link`);
      if ((await link.getAttribute("href")) !== "/filament/calibrate?result=11") throw new Error(`detail link: ${await link.getAttribute("href")}`);
      if ((await link.innerText()).trim() !== "Calibration: Temperature") throw new Error("detail link text");
      await page.click(`${journalRow(2)} .jr-row-main`);
      await page.waitForSelector(`${journalRow(2)} .jr-detail`, { state: "detached", timeout: 5000 });
    });
    // A print without a purpose has neither.
    await page.click(`${JR} .jr-filter[data-kind="all"]`);
    await rowsRead(ALL_ROWS);
    if ((await page.locator(`${JR} .jr-purpose`).count()) !== 0) throw new Error("a plain print shows a calibration chip");
  });

  await step(run, page, "journal filters and search ask the backend for what was picked", async () => {
    const lastFilter = async () => (await callsOf("journal_list")).at(-1).args.filter;
    await withFixtures({ journal_list: [JOURNAL_FINISHED] }, async () => {
      const before = await countOf("journal_list");
      await page.click(`${JR} .jr-filter[data-kind="needs_verdict"]`);
      await pastCalls("journal_list", before);
      await page.waitForFunction(() => document.querySelectorAll(".journal-page .jr-row").length === 1, null, { timeout: 5000 });
      if (!sameArgs(await lastFilter(), { kind: "needs_verdict", query: "" })) throw new Error(JSON.stringify(await lastFilter()));
      if ((await page.locator(`${JR} .jr-filter[data-kind="needs_verdict"]`).getAttribute("aria-pressed")) !== "true") throw new Error("pill not pressed");
      if ((await page.locator(`${JR} .jr-filter[data-kind="all"]`).getAttribute("aria-pressed")) !== "false") throw new Error("All still pressed");
    });
    await withFixtures({ journal_list: [JOURNAL_FAILED] }, async () => {
      const months = await countOf("journal_month_summary");
      await page.click(`${JR} .jr-filter[data-kind="failed"]`);
      await page.fill(`${JR} .jr-search`, "petg");
      await page.waitForFunction(
        () => window.__ipc.calls.filter((c) => c.cmd === "journal_list").at(-1)?.args.filter.query === "petg",
        null,
        { timeout: 5000 }
      );
      if (!sameArgs(await lastFilter(), { kind: "failed", query: "petg" })) throw new Error(JSON.stringify(await lastFilter()));
      await rowsRead([JOURNAL_FAILED.name]);
      if ((await countOf("journal_month_summary")) !== months) throw new Error("a filter or search recounted the month");
    });
    await withFixtures({ journal_list: [] }, async () => {
      await page.fill(`${JR} .jr-search`, "nothing like this");
      await page.waitForFunction(() => document.querySelector(".journal-page .nd-state")?.textContent === "[EMPTY: No prints match]", null, { timeout: 5000 });
    });
    await page.fill(`${JR} .jr-search`, "");
    await page.click(`${JR} .jr-filter[data-kind="all"]`);
    await page.waitForFunction(() => document.querySelectorAll(".journal-page .jr-row").length === 4, null, { timeout: 5000 });
  });

  await step(run, page, "a journal answer overtaken by a newer search is dropped", async () => {
    // The first search's answer is held back until the second one has landed.
    const slow = { __hold: "journal-stale", answer: [JOURNAL_GOOD] };
    await withFixtures({ journal_list: { __sequence: [slow, [JOURNAL_FAILED]] } }, async () => {
      const lists = await countOf("journal_list");
      await page.fill(`${JR} .jr-search`, "swatch");
      await pastCalls("journal_list", lists);
      await page.fill(`${JR} .jr-search`, "bracket");
      await pastCalls("journal_list", lists + 1);
      await rowsRead([JOURNAL_FAILED.name]);
      await page.evaluate(() => window.__release("journal-stale"));
      // Wait for the held answer to be out, then let its continuation run
      // before reading the rows again.
      await page.waitForFunction(
        (n) => window.__ipc.settled.filter((c) => c === "journal_list").length >= n,
        lists + 2,
        { timeout: 5000 }
      );
      await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
      const names = await rowNames();
      if (names.join("|") !== JOURNAL_FAILED.name) throw new Error(`the stale answer landed: ${names.join("|")}`);
    });
    await page.fill(`${JR} .jr-search`, "");
    await rowsRead(ALL_ROWS);
  });

  await step(run, page, "the journal reloads when the backend says it changed", async () => {
    await withFixtures({ journal_list: [JOURNAL_GOOD] }, async () => {
      const before = await countOf("journal_list");
      const months = await countOf("journal_month_summary");
      await journalEmit("journal://changed", null);
      await pastCalls("journal_list", before);
      await pastCalls("journal_month_summary", months);
      await rowsRead([JOURNAL_GOOD.name]);
    });
    await journalEmit("journal://changed", null);
    await rowsRead(ALL_ROWS);
  });

  await step(run, page, "a thumbnail that can't be read, or can't be decoded, says so", async () => {
    // The backend refuses it, or sends a PNG cut off after its signature.
    for (const unreadable of [{ __reject: "Photo unavailable" }, "data:image/png;base64,iVBORw0KGgo="]) {
      await withFixtures({ journal_image: unreadable }, async () => {
        await page.click('.nd-tabs a[href="/printer"]');
        await page.waitForSelector(".printer-page", { timeout: 15000 });
        await page.click('.nd-tabs a[href="/printer/journal"]');
        await page.waitForFunction(
          () => document.querySelector('.journal-page .jr-row[data-entry="2"] .jr-thumb')?.textContent === "Photo unavailable",
          null,
          { timeout: 5000 }
        );
        if (await page.locator(`${journalRow(2)} .jr-thumb img`).count()) throw new Error("an image beside the placeholder");
        const box = await page.locator(`${journalRow(2)} .jr-thumb`).boundingBox();
        if (Math.round(box.width) !== 56 || Math.round(box.height) !== 56) throw new Error(`the placeholder is ${box.width}x${box.height}`);
      });
    }
  });

  await step(run, page, "thumbnails load only as their rows come near the visible list", async () => {
    const many = Array.from({ length: 40 }, (_, i) => ({ ...JOURNAL_FINISHED, id: 100 + i, name: `Plate ${i + 1}` }));
    await withFixtures({ journal_list: many }, async () => {
      await page.click('.nd-tabs a[href="/printer"]');
      await page.waitForSelector(".printer-page", { timeout: 15000 });
      await page.evaluate(() => document.querySelector(".content").scrollTo(0, 0));
      const before = await countOf("journal_image");
      await page.click('.nd-tabs a[href="/printer/journal"]');
      await page.waitForSelector(`${journalRow(100)} .jr-thumb img`, { timeout: 5000 });
      const asked = (await callsOf("journal_image")).slice(before).map((c) => c.args.id);
      if (asked.includes(139) || asked.length >= many.length) throw new Error(`asked for ${asked.length} thumbnails: ${asked.join(",")}`);
      await page.locator(journalRow(139)).scrollIntoViewIfNeeded();
      await page.waitForSelector(`${journalRow(139)} .jr-thumb img`, { timeout: 5000 });
      const again = (await callsOf("journal_image")).slice(before).map((c) => c.args.id);
      if (new Set(again).size !== again.length) throw new Error("a thumbnail was asked for twice");
      await page.evaluate(() => document.querySelector(".content").scrollTo(0, 0));
    });
  });

  await step(run, page, "an empty journal says prints appear as the printer runs them", async () => {
    await withFixtures({ journal_list: [] }, async () => {
      await page.click('nav.sidebar a[href="/"]');
      await openTab("/printer", "/printer/journal");
      await page.waitForFunction(
        () => document.querySelector(".journal-page .nd-state")?.textContent === "[EMPTY: Prints appear here as your printer runs them]",
        null,
        { timeout: 5000 }
      );
    });
  });

  await step(run, page, "without a printer the journal says it fills in by itself", async () => {
    await journalEmit("printer://state", PRINTER_UNCONFIGURED);
    await page.waitForSelector(`${JR} .jr-no-printer`, { timeout: 5000 });
    const note = await page.locator(`${JR} .jr-no-printer`).innerText();
    if (!note.includes("The journal fills in by itself once a printer is set up.")) throw new Error(`note reads "${note}"`);
    if (!note.includes("Set one up in Settings → Printer")) throw new Error(`note reads "${note}"`);
    if ((await page.locator(`${JR} .jr-no-printer a[href="/settings#printer"]`).count()) !== 1) throw new Error("no Settings link");
    await journalEmit("printer://state", PRINTER_VIEW);
    await page.waitForSelector(`${JR} .jr-no-printer`, { state: "detached", timeout: 5000 });
  });

  // -- the entry detail: verdict, tags, notes, photos, Analyze, delete ------
  const journalDetail = (id) => `${journalRow(id)} .jr-detail`;
  const expand = async (id) => {
    if (!(await page.locator(journalDetail(id)).count())) await page.click(`${journalRow(id)} .jr-row-main`);
    await page.waitForSelector(journalDetail(id), { timeout: 5000 });
  };
  const collapse = async (id) => {
    if (await page.locator(journalDetail(id)).count()) await page.click(`${journalRow(id)} .jr-row-main`);
    await page.waitForSelector(journalDetail(id), { state: "detached", timeout: 5000 });
  };
  // The status line of one part of the open detail.
  const statusIs = (scope, text) =>
    page.waitForFunction(([s, t]) => document.querySelector(`${s} .jr-status`)?.textContent === t, [scope, text], { timeout: 5000 });
  const pressedTags = (id) =>
    page.locator(`${journalDetail(id)} .jr-tag[aria-pressed="true"]`).evaluateAll((bs) => bs.map((b) => b.dataset.tag).join("|"));
  const pngBase64 = png.toString("base64");
  // Hands files to the page the way a drop or a pick does, built in the page
  // so a 25 MB file doesn't cross the wire.
  const dropFiles = (selector, files) =>
    page.evaluate(([sel, list]) => {
      const dt = new DataTransfer();
      for (const f of list) {
        const bytes = f.base64 ? Uint8Array.from(atob(f.base64), (c) => c.charCodeAt(0)) : new Uint8Array(f.size);
        dt.items.add(new File([bytes], f.name, { type: f.type }));
      }
      const target = document.querySelector(sel);
      target.dispatchEvent(new DragEvent("dragenter", { bubbles: true, cancelable: true, dataTransfer: dt }));
      target.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: dt }));
    }, [selector, files]);
  const pickFiles = (selector, files) =>
    page.evaluate(([sel, list]) => {
      const dt = new DataTransfer();
      for (const f of list) dt.items.add(new File([new Uint8Array(f.size)], f.name, { type: f.type }));
      const input = document.querySelector(sel);
      input.files = dt.files;
      input.dispatchEvent(new Event("change", { bubbles: true }));
    }, [selector, files]);
  const SAVE_FAILED = "The print journal couldn't be saved.";

  await step(run, page, "an entry expands in place with its facts", async () => {
    await openJournal();
    const path = await page.evaluate(() => location.pathname);
    await expand(1);
    if ((await page.locator(`${journalRow(1)} .jr-row-main`).getAttribute("aria-expanded")) !== "true") throw new Error("not marked expanded");
    if ((await page.locator(journalDetail(1)).getAttribute("role")) !== "region") throw new Error("the detail is not a region");
    if (!(await page.locator(journalDetail(1)).getAttribute("aria-label"))) throw new Error("the region has no name");
    if (await page.locator('[role="dialog"]').count()) throw new Error("the detail opened a dialog");
    if ((await page.evaluate(() => location.pathname)) !== path) throw new Error("the detail changed the page");
    const facts = (await page.locator(`${journalDetail(1)} dl.jr-facts`).textContent()).replace(/\s+/g, " ");
    for (const want of ["(approximate)", "0300-400C", "0300-0100-0001-0007", "H2D", "Plate"]) {
      if (!facts.includes(want)) throw new Error(`facts lack ${want}: ${facts}`);
    }
    if (facts.includes("0300_0100")) throw new Error(`an HMS code kept its underscores: ${facts}`);
    if (facts.includes("Slice estimate")) throw new Error("an estimate without a linked slice");
    await expand(2);
    if (await page.locator(journalDetail(1)).count()) throw new Error("two entries open at once");
    const linked = (await page.locator(`${journalDetail(2)} .jr-facts`).textContent()).replace(/\s+/g, " ");
    if (!linked.includes("Slice estimate") || !linked.includes("14 m · 3.7 g")) throw new Error(`facts: ${linked}`);
    const pressed = await page.locator(`${journalDetail(2)} .jr-verdict`).evaluateAll((bs) => bs.map((b) => `${b.textContent}:${b.getAttribute("aria-pressed")}`).join("|"));
    if (pressed !== "Good:false|Usable:false|Failed:false") throw new Error(`verdict pills: ${pressed}`);
  });

  await step(run, page, "the verdict and issue tags save, and a failed save puts them back", async () => {
    await expand(1);
    const rate = `${journalDetail(1)} .jr-rate`;
    await withFixtures({ journal_update: { ...JOURNAL_FAILED, verdict: "usable" } }, async () => {
      const before = await countOf("journal_update");
      await page.click(`${journalDetail(1)} .jr-verdict[data-verdict="usable"]`);
      await pastCalls("journal_update", before);
      const args = (await callsOf("journal_update")).at(-1).args;
      if (args.id !== 1 || !sameArgs(args.update, { verdict: "usable" })) throw new Error(JSON.stringify(args));
      await page.waitForSelector(`${journalDetail(1)} .jr-verdict[data-verdict="usable"][aria-pressed="true"]`, { timeout: 5000 });
      if ((await page.locator(`${journalDetail(1)} .jr-verdict[data-verdict="failed"]`).getAttribute("aria-pressed")) !== "false") throw new Error("two verdicts pressed");
      await statusIs(rate, "[SAVED]");
      // The row's own line follows the saved verdict.
      await page.waitForFunction(() => document.querySelector('.journal-page .jr-row[data-entry="1"] .jr-verdict-value')?.textContent === "Usable", null, { timeout: 5000 });
    });
    await withFixtures({ journal_update: { ...JOURNAL_FAILED, verdict: "usable", tags: ["warping", "stringing"] } }, async () => {
      const before = await countOf("journal_update");
      await page.click(`${journalDetail(1)} .jr-tag[data-tag="stringing"]`);
      await pastCalls("journal_update", before);
      const args = (await callsOf("journal_update")).at(-1).args;
      if (!sameArgs(args.update, { tags: ["warping", "stringing"] })) throw new Error(JSON.stringify(args.update));
      await page.waitForSelector(`${journalDetail(1)} .jr-tag[data-tag="stringing"][aria-pressed="true"]`, { timeout: 5000 });
      const label = (await page.locator(`${journalDetail(1)} .jr-tag[data-tag="stringing"]`).textContent()).trim();
      if (label !== "Stringing/Oozing") throw new Error(`the tag reads "${label}", not Analyze's name`);
    });
    await withFixtures({ journal_update: { __reject: "disk I/O error: /Users/me/Library" } }, async () => {
      await page.click(`${journalDetail(1)} .jr-tag[data-tag="z_banding"]`);
      await page.waitForFunction(
        (t) => document.querySelector('.journal-page .jr-detail .nd-state[data-kind="error"]')?.textContent === t,
        `[ERROR: ${SAVE_FAILED}]`,
        { timeout: 5000 }
      );
      if ((await pressedTags(1)) !== "stringing|warping") throw new Error(`a failed tag stayed pressed: ${await pressedTags(1)}`);
      if ((await page.locator(journalDetail(1)).innerText()).includes("disk I/O")) throw new Error("the raw error reached the page");
      await page.click(`${journalDetail(1)} .jr-verdict[data-verdict="good"]`);
      await page.waitForSelector(`${journalDetail(1)} .jr-verdict[data-verdict="usable"][aria-pressed="true"]`, { timeout: 5000 });
      if ((await page.locator(`${journalDetail(1)} .jr-verdict[data-verdict="good"]`).getAttribute("aria-pressed")) !== "false") throw new Error("a failed verdict stayed pressed");
      await statusIs(rate, `[ERROR: ${SAVE_FAILED}]`);
    });
  });

  await step(run, page, "rating saves go out one at a time and the last click is what is stored", async () => {
    const before = await countOf("journal_update");
    const settled = await settledCount("journal_update");
    const rated = { ...JOURNAL_FAILED, tags: ["warping", "stringing"] };
    // Good's answer is held back; Failed, clicked meanwhile, waits for it.
    const seq = [{ __hold: "jr-good", answer: { ...rated, verdict: "good" } }, { ...rated, verdict: "failed" }];
    const verdictPressed = () =>
      page.locator(`${journalDetail(1)} .jr-verdict[aria-pressed="true"]`).evaluateAll((bs) => bs.map((b) => b.dataset.verdict).join("|"));
    await withFixtures({ journal_update: { __sequence: seq } }, async () => {
      await page.click(`${journalDetail(1)} .jr-verdict[data-verdict="good"]`);
      await pastCalls("journal_update", before);
      await page.click(`${journalDetail(1)} .jr-verdict[data-verdict="failed"]`);
      if ((await verdictPressed()) !== "failed") throw new Error(`shown at once: ${await verdictPressed()}`);
      for (let i = 0; i < 3; i++) await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
      if ((await countOf("journal_update")) !== before + 1) throw new Error("a second save went out while the first was out");
      await page.evaluate(() => window.__release("jr-good"));
      await pastCalls("journal_update", before + 1);
      await page.waitForFunction(([c, n]) => window.__ipc.settled.filter((x) => x === c).length >= n, ["journal_update", settled + 2], { timeout: 5000 });
      await statusIs(`${journalDetail(1)} .jr-rate`, "[SAVED]");
    });
    const sent = (await callsOf("journal_update")).slice(before).map((c) => JSON.stringify(c.args.update));
    if (sent.join("|") !== '{"verdict":"good"}|{"verdict":"failed"}') throw new Error(`sent ${sent.join("|")}`);
    // What the last answer stored is what the pills and the row show.
    if ((await verdictPressed()) !== "failed") throw new Error(`settled on ${await verdictPressed()}`);
    await page.waitForFunction(() => document.querySelector('.journal-page .jr-row[data-entry="1"] .jr-verdict-value')?.textContent === "Failed", null, { timeout: 5000 });
  });

  await step(run, page, "a click waiting behind a save still goes out after the row closes", async () => {
    const before = await countOf("journal_update");
    const settled = await settledCount("journal_update");
    const rated = { ...JOURNAL_FAILED, tags: ["warping", "stringing"] };
    const seq = [{ __hold: "jr-close", answer: { ...rated, verdict: "good" } }, { ...rated, verdict: "failed" }];
    const verdictPressed = () =>
      page.locator(`${journalDetail(1)} .jr-verdict[aria-pressed="true"]`).evaluateAll((bs) => bs.map((b) => b.dataset.verdict).join("|"));
    await withFixtures({ journal_update: { __sequence: seq } }, async () => {
      await page.click(`${journalDetail(1)} .jr-verdict[data-verdict="good"]`);
      await pastCalls("journal_update", before);
      await page.click(`${journalDetail(1)} .jr-verdict[data-verdict="failed"]`);
      await collapse(1);
      // Opened again while Good is still out: it shows the click, and a
      // click now doesn't start a second save.
      await expand(1);
      if ((await verdictPressed()) !== "failed") throw new Error(`reopened on ${await verdictPressed()}`);
      if ((await countOf("journal_update")) !== before + 1) throw new Error("a second save went out for the entry");
      await collapse(1);
      await page.evaluate(() => window.__release("jr-close"));
      await page.waitForFunction(([c, n]) => window.__ipc.settled.filter((x) => x === c).length >= n, ["journal_update", settled + 2], { timeout: 5000 });
    });
    const sent = (await callsOf("journal_update")).slice(before).map((c) => JSON.stringify(c.args.update));
    if (sent.join("|") !== '{"verdict":"good"}|{"verdict":"failed"}') throw new Error(`sent ${sent.join("|")}`);
    await expand(1);
    if ((await verdictPressed()) !== "failed") throw new Error(`reopened on ${await verdictPressed()}`);
    await page.waitForFunction(() => document.querySelector('.journal-page .jr-row[data-entry="1"] .jr-verdict-value')?.textContent === "Failed", null, { timeout: 5000 });
  });

  await step(run, page, "notes save when the box loses focus, not while typing, and not unchanged", async () => {
    const text = "Lifted at the front corner. Try a brim.";
    const notes = `${journalDetail(1)} .jr-notes-field`;
    const box = `${journalDetail(1)} .jr-notes`;
    await withFixtures({ journal_update: { ...JOURNAL_FAILED, verdict: "usable", tags: ["warping", "stringing"], notes: text } }, async () => {
      const before = await countOf("journal_update");
      await page.fill(box, text);
      if ((await countOf("journal_update")) !== before) throw new Error("saved while typing");
      await page.locator(box).blur();
      await pastCalls("journal_update", before);
      const args = (await callsOf("journal_update")).at(-1).args;
      if (args.id !== 1 || !sameArgs(args.update, { notes: text })) throw new Error(JSON.stringify(args.update));
      // Focus and leave again at once, before or after the answer: nothing changed.
      await page.click(box);
      await page.locator(box).blur();
      await statusIs(notes, "[SAVED]");
      await page.click(box);
      await page.locator(box).blur();
      if ((await countOf("journal_update")) !== before + 1) throw new Error("an unchanged note was saved again");
    });
    await withFixtures({ journal_update: { __reject: SAVE_FAILED } }, async () => {
      const before = await countOf("journal_update");
      await page.fill(box, `${text} Failed again.`);
      await page.locator(box).blur();
      await statusIs(notes, `[ERROR: ${SAVE_FAILED}]`);
      if ((await page.locator(box).inputValue()) !== `${text} Failed again.`) throw new Error("a failed save threw the text away");
      // The text isn't stored, so the next blur tries again.
      await page.click(box);
      await page.locator(box).blur();
      await pastCalls("journal_update", before + 1);
    });
  });

  await step(run, page, "a reopened notes box follows newer notes and never saves stale text over them", async () => {
    const box = `${journalDetail(1)} .jr-notes`;
    // What the list holds from the notes step (its last save failed).
    const old = "Lifted at the front corner. Try a brim.";
    const settled = await settledCount("journal_update");
    const late = { __hold: "jr-notes-late", answer: { ...JOURNAL_FAILED, notes: "Use a brim." } };
    await withFixtures({ journal_update: late }, async () => {
      const before = await countOf("journal_update");
      await page.fill(box, "Use a brim.");
      // Closing the row takes focus from the box, which sends the save; the
      // row opens again before the answer.
      await page.click(`${journalRow(1)} .jr-row-main`);
      await page.waitForSelector(journalDetail(1), { state: "detached", timeout: 5000 });
      await pastCalls("journal_update", before);
      await expand(1);
      if ((await page.locator(box).inputValue()) !== old) throw new Error("the reopened box doesn't show the list's notes");
      // The user is back in the box when the answer and the reload land.
      await page.focus(box);
      await page.evaluate(() => window.__release("jr-notes-late"));
      await page.waitForFunction(([c, n]) => window.__ipc.settled.filter((x) => x === c).length > n, ["journal_update", settled], { timeout: 5000 });
    });
    // The backend says it changed; the reload brings the saved notes.
    const saved = FIXTURES.journal_list.map((e) => (e.id === 1 ? { ...e, notes: "Use a brim." } : e));
    await withFixtures({ journal_list: saved }, async () => {
      const lists = await countOf("journal_list");
      const answered = await settledCount("journal_list");
      await journalEmit("journal://changed", null);
      await pastCalls("journal_list", lists);
      await page.waitForFunction(([c, n]) => window.__ipc.settled.filter((x) => x === c).length > n, ["journal_list", answered], { timeout: 5000 });
      await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
      if ((await page.locator(box).inputValue()) !== old) throw new Error("the box changed under the user's cursor");
    });
    // Leaving the box with nothing typed takes the newer notes, and saves nothing.
    const before = await countOf("journal_update");
    await page.locator(box).blur();
    await page.waitForFunction((b) => document.querySelector(b)?.value === "Use a brim.", box, { timeout: 5000 });
    await page.click(box);
    await page.locator(box).blur();
    for (let i = 0; i < 2; i++) await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
    if ((await countOf("journal_update")) !== before) throw new Error("leaving the followed box saved again");
  });

  await step(run, page, "note saves go out one at a time and the newest text is what is stored", async () => {
    const box = `${journalDetail(1)} .jr-notes`;
    const notes = `${journalDetail(1)} .jr-notes-field`;
    const before = await countOf("journal_update");
    const settled = await settledCount("journal_update");
    // The first save's answer is held back; the second blur waits for it.
    const seq = [
      { __hold: "jr-notes-first", answer: { ...JOURNAL_FAILED, notes: "first" } },
      { ...JOURNAL_FAILED, notes: "second" },
    ];
    await withFixtures({ journal_update: { __sequence: seq } }, async () => {
      await page.fill(box, "first");
      await page.locator(box).blur();
      await pastCalls("journal_update", before);
      await page.fill(box, "second");
      await page.locator(box).blur();
      for (let i = 0; i < 3; i++) await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
      if ((await countOf("journal_update")) !== before + 1) throw new Error("a second note save went out while the first was out");
      await page.evaluate(() => window.__release("jr-notes-first"));
      await pastCalls("journal_update", before + 1);
      await page.waitForFunction(([c, n]) => window.__ipc.settled.filter((x) => x === c).length >= n, ["journal_update", settled + 2], { timeout: 5000 });
      await statusIs(notes, "[SAVED]");
    });
    const sent = (await callsOf("journal_update")).slice(before).map((c) => c.args.update.notes);
    if (sent.join("|") !== "first|second") throw new Error(`sent ${sent.join("|")}`);
    if ((await page.locator(box).inputValue()) !== "second") throw new Error(`the box settled on ${await page.locator(box).inputValue()}`);
    // The list holds the last answer: a reopened box shows it.
    await collapse(1);
    await expand(1);
    if ((await page.locator(box).inputValue()) !== "second") throw new Error(`the list holds ${await page.locator(box).inputValue()}`);
  });

  await step(run, page, "a photo can be added by picking or dropping it, one upload at a time", async () => {
    await expand(2);
    await page.waitForSelector(`${journalDetail(2)} .jr-photo img`, { timeout: 5000 });
    if ((await page.locator(`${journalDetail(2)} .jr-photo-add`).evaluate((el) => el.tagName)) !== "BUTTON") throw new Error("Add photo is not a button");
    const before = await countOf("journal_add_photo");
    await page.setInputFiles(`${journalDetail(2)} .jr-photo-input`, { name: "print.png", mimeType: "image/png", buffer: png });
    await pastCalls("journal_add_photo", before);
    const args = (await callsOf("journal_add_photo")).at(-1).args;
    if (args.id !== 2 || typeof args.dataBase64 !== "string" || args.dataBase64 !== pngBase64) throw new Error(`args: ${JSON.stringify({ ...args, dataBase64: args.dataBase64?.length })}`);
    await page.waitForFunction(() => document.querySelectorAll('.journal-page .jr-row[data-entry="2"] .jr-photo').length === 2, null, { timeout: 5000 });
    // Photos go up one after the other: two dropped together, and a third
    // dropped while the first is still going up.
    const two = { ...JOURNAL_FINISHED, photos: [...JOURNAL_FINISHED.photos, { id: 8, file_name: "9a1e.jpg", added_at: 0 }] };
    await withFixtures({ journal_add_photo: { __sequence: [{ __hold: "jr-photo-1", answer: two }, two] } }, async () => {
      const sent = await countOf("journal_add_photo");
      const done = await settledCount("journal_add_photo");
      const file = { name: "a.png", type: "image/png", base64: pngBase64 };
      await dropFiles(`${journalDetail(2)} .jr-photos`, [file, { ...file, name: "b.png" }]);
      await pastCalls("journal_add_photo", sent);
      await statusIs(`${journalDetail(2)} .jr-photos`, "[LOADING: Adding the photo]");
      await dropFiles(`${journalDetail(2)} .jr-photos`, [{ ...file, name: "c.png" }]);
      for (let i = 0; i < 3; i++) await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
      if ((await countOf("journal_add_photo")) !== sent + 1) throw new Error("a photo went up before the first answered");
      await page.evaluate(() => window.__release("jr-photo-1"));
      await pastCalls("journal_add_photo", sent + 1);
      await page.waitForFunction(([c, n]) => window.__ipc.settled.filter((x) => x === c).length >= n, ["journal_add_photo", done + 3], { timeout: 5000 });
      if ((await countOf("journal_add_photo")) !== sent + 3) throw new Error(`${(await countOf("journal_add_photo")) - sent} photos went up, not 3`);
      await statusIs(`${journalDetail(2)} .jr-photos`, "");
      if (await page.locator(`${journalDetail(2)} .jr-photos.jr-drop-over`).count()) throw new Error("the drop outline stayed");
    });
    // Each photo and its Remove are named apart.
    await page.waitForFunction(() => document.querySelectorAll('.journal-page .jr-row[data-entry="2"] .jr-photo img').length === 2, null, { timeout: 5000 });
    const alts = await page.locator(`${journalDetail(2)} .jr-photo img`).evaluateAll((is) => is.map((i) => i.alt));
    if (!/^Photo 1 of 2, added \d+ \w{3} \d{4}$/.test(alts[0]) || !alts[1].startsWith("Photo 2 of 2, added ")) throw new Error(`alts: ${alts.join("|")}`);
    const removes = await page.locator(`${journalDetail(2)} .jr-photo-remove`).evaluateAll((bs) => bs.map((b) => b.getAttribute("aria-label")));
    if (removes.join("|") !== "Remove photo 1|Remove photo 2") throw new Error(`remove labels: ${removes.join("|")}`);
  });

  await step(run, page, "photos queued before the detail closes still go up, one at a time", async () => {
    const two = { ...JOURNAL_FINISHED, photos: [...JOURNAL_FINISHED.photos, { id: 8, file_name: "9a1e.jpg", added_at: 0 }] };
    await withFixtures({ journal_add_photo: { __sequence: [{ __hold: "jr-photo-close", answer: two }, two] } }, async () => {
      const sent = await countOf("journal_add_photo");
      const done = await settledCount("journal_add_photo");
      const file = { name: "a.png", type: "image/png", base64: pngBase64 };
      await dropFiles(`${journalDetail(2)} .jr-photos`, [file, { ...file, name: "b.png" }]);
      await pastCalls("journal_add_photo", sent);
      await collapse(2);
      await page.evaluate(() => window.__release("jr-photo-close"));
      await page.waitForFunction(([c, n]) => window.__ipc.settled.filter((x) => x === c).length >= n, ["journal_add_photo", done + 2], { timeout: 5000 });
      if ((await countOf("journal_add_photo")) !== sent + 2) throw new Error(`${(await countOf("journal_add_photo")) - sent} photos went up, not 2`);
    });
    await expand(2);
    await page.waitForFunction(() => document.querySelectorAll('.journal-page .jr-row[data-entry="2"] .jr-photo').length === 2, null, { timeout: 5000 });
    await statusIs(`${journalDetail(2)} .jr-photos`, "");
  });

  await step(run, page, "a photo over 25 MB, or a file that isn't one, is refused before it is sent", async () => {
    const photos = `${journalDetail(2)} .jr-photos`;
    const before = await countOf("journal_add_photo");
    await pickFiles(`${journalDetail(2)} .jr-photo-input`, [{ name: "huge.png", type: "image/png", size: 25 * 1024 * 1024 + 1 }]);
    await statusIs(photos, "[ERROR: That photo is over 25 MB. Choose a smaller one.]");
    if ((await page.locator(`${photos} .jr-status`).getAttribute("data-kind")) !== "error") throw new Error("the refusal is not marked as an error");
    await page.setInputFiles(`${journalDetail(2)} .jr-photo-input`, { name: "notes.txt", mimeType: "text/plain", buffer: Buffer.from("hello") });
    await page.waitForFunction(
      () => document.querySelector('.journal-page .jr-row[data-entry="2"] .nd-state[data-kind="error"]')?.textContent ===
        "[ERROR: That file isn't a photo BambuMate can read. Choose a JPEG, PNG or WebP image.]",
      null,
      { timeout: 5000 }
    );
    if ((await countOf("journal_add_photo")) !== before) throw new Error("a refused file was sent");
  });

  await step(run, page, "a photo that can't be read shows a placeholder and can be removed", async () => {
    for (const unreadable of [{ __reject: "Photo unavailable" }, null]) {
      await withFixtures({ journal_image: unreadable }, async () => {
        await collapse(2);
        await expand(2);
        await page.waitForFunction(
          () => {
            const missing = [...document.querySelectorAll('.journal-page .jr-row[data-entry="2"] .jr-photo-missing')];
            return missing.length === 2 && missing.every((e) => e.textContent === "Photo unavailable");
          },
          null,
          { timeout: 5000 }
        );
        if (await page.locator(`${journalDetail(2)} .jr-photo img`).count()) throw new Error("an image beside the placeholder");
      });
    }
    await withFixtures({ journal_remove_photo: { __reject: SAVE_FAILED } }, async () => {
      await page.locator(`${journalDetail(2)} .jr-photo-remove`).first().click();
      await statusIs(`${journalDetail(2)} .jr-photos`, `[ERROR: ${SAVE_FAILED}]`);
      if ((await page.locator(`${journalDetail(2)} .jr-photo`).count()) !== 2) throw new Error("a photo went though its removal failed");
    });
    const placeholder = await page.locator(`${journalDetail(2)} .jr-photo-missing`).first().getAttribute("aria-label");
    if (!placeholder?.startsWith("Photo 1 of 2, added ")) throw new Error(`the placeholder is named "${placeholder}"`);
    const before = await countOf("journal_remove_photo");
    const left = { ...JOURNAL_FINISHED, photos: [{ id: 8, file_name: "9a1e.jpg", added_at: 0 }] };
    await withFixtures({ journal_remove_photo: left }, async () => {
      await page.locator(`${journalDetail(2)} .jr-photo-remove`).first().click();
      await pastCalls("journal_remove_photo", before);
      const args = (await callsOf("journal_remove_photo")).at(-1).args;
      if (args.id !== 2 || args.photoId !== 7) throw new Error(JSON.stringify(args));
      // Focus goes to the next photo's Remove.
      await page.waitForFunction(() => document.activeElement?.closest('.jr-photo[data-photo="8"]') && document.activeElement.classList.contains("jr-photo-remove"), null, { timeout: 5000 });
    });
    if ((await page.locator(`${journalDetail(2)} .jr-photo-remove`).getAttribute("aria-label")) !== "Remove photo 1") throw new Error("the remaining photo isn't renumbered");
    await page.locator(`${journalDetail(2)} .jr-photo-remove`).click();
    await pastCalls("journal_remove_photo", before + 1);
    await page.waitForFunction(() => document.querySelectorAll('.journal-page .jr-row[data-entry="2"] .jr-photo').length === 0, null, { timeout: 5000 });
    // The last photo gone, focus goes to Add photo.
    await page.waitForFunction(() => document.activeElement?.classList.contains("jr-photo-add"), null, { timeout: 5000 });
  });

  await step(run, page, "Analyze a photo links to Analyze with the entry", async () => {
    const link = page.locator(`${journalDetail(2)} a.jr-analyze`);
    if ((await link.count()) !== 1) throw new Error("no Analyze link while AI is on");
    if ((await link.getAttribute("href")) !== "/analysis?journal=2") throw new Error(`href ${await link.getAttribute("href")}`);
    if ((await link.innerText()).trim().toLowerCase() !== "analyze a photo") throw new Error(`reads "${await link.innerText()}"`);
  });

  await step(run, page, "deleting an entry asks inline, Keep cancels and Delete removes it", async () => {
    await expand(1);
    const before = await countOf("journal_delete");
    await page.click(`${journalDetail(1)} .jr-delete`);
    const confirm = await page.locator(`${journalDetail(1)} .jr-confirm`).innerText();
    if (!confirm.includes("Delete this entry and its photos?")) throw new Error(`confirm reads "${confirm}"`);
    const buttons = await page.locator(`${journalDetail(1)} .jr-confirm button`).allInnerTexts();
    if (buttons.map((b) => b.trim().toLowerCase()).join("|") !== "delete|keep") throw new Error(`buttons ${buttons.join("|")}`);
    if (await page.locator('[role="dialog"], [role="alertdialog"], dialog').count()) throw new Error("the confirmation is a dialog");
    if ((await countOf("journal_delete")) !== before) throw new Error("deleted before confirming");
    await page.waitForFunction(() => document.activeElement?.classList.contains("jr-confirm-keep"), null, { timeout: 5000 });
    await page.click(`${journalDetail(1)} .jr-confirm-keep`);
    await page.waitForSelector(`${journalDetail(1)} .jr-confirm`, { state: "detached", timeout: 5000 });
    await page.waitForFunction(() => document.activeElement?.classList.contains("jr-delete"), null, { timeout: 5000 });
    await withFixtures({ journal_delete: { __reject: SAVE_FAILED } }, async () => {
      await page.click(`${journalDetail(1)} .jr-delete`);
      await page.click(`${journalDetail(1)} .jr-confirm-delete`);
      await page.waitForFunction(
        (t) => document.querySelector('.journal-page .jr-row[data-entry="1"] .nd-state[data-kind="error"]')?.textContent === t,
        `[ERROR: ${SAVE_FAILED}]`,
        { timeout: 5000 }
      );
      if (await page.locator(`${journalDetail(1)} .jr-confirm`).count()) throw new Error("the confirmation stayed after a failed delete");
      await page.waitForFunction(() => document.activeElement?.classList.contains("jr-delete"), null, { timeout: 5000 });
    });
    const failed = await countOf("journal_delete");
    await page.click(`${journalDetail(1)} .jr-delete`);
    await page.click(`${journalDetail(1)} .jr-confirm-delete`);
    await pastCalls("journal_delete", failed);
    if ((await callsOf("journal_delete")).at(-1).args.id !== 1) throw new Error("deleted the wrong entry");
    await page.waitForSelector(journalRow(1), { state: "detached", timeout: 5000 });
    if (await page.locator(`${JR} .jr-detail`).count()) throw new Error("a detail is still open");
    // Focus moves to the row that followed the deleted one.
    await page.waitForFunction(() => document.activeElement?.closest('.jr-row[data-entry="4"]') && document.activeElement.classList.contains("jr-row-main"), null, { timeout: 5000 });
  });

  await step(run, page, "the journal opens the entry a link names and brings it into view", async () => {
    const many = Array.from({ length: 40 }, (_, i) => ({ ...JOURNAL_FINISHED, id: 100 + i, name: `Plate ${i + 1}` }));
    await withFixtures({ journal_list: many }, async () => {
      await page.click('nav.sidebar a[href="/"]');
      await page.evaluate(() => document.querySelector(".content").scrollTo(0, 0));
      await goTo("/printer/journal?entry=139", "/printer/journal");
      await page.waitForSelector(journalDetail(139), { timeout: 15000 });
      await page.waitForFunction(
        () => {
          const row = document.querySelector('.journal-page .jr-row[data-entry="139"]').getBoundingClientRect();
          const view = document.querySelector(".content").getBoundingClientRect();
          return row.top >= view.top - 1 && row.top < view.bottom;
        },
        null,
        { timeout: 5000 }
      );
      if ((await page.locator(`${journalRow(139)} .jr-row-main`).getAttribute("aria-expanded")) !== "true") throw new Error("the linked row isn't marked open");
      // A link followed while the journal is open scrolls there at once.
      await page.evaluate(() => document.querySelector(".content").scrollTo(0, 0));
      await goTo("/printer/journal?entry=110", "/printer/journal");
      await page.waitForSelector(journalDetail(110), { timeout: 5000 });
      await page.waitForFunction(
        () => {
          const row = document.querySelector('.journal-page .jr-row[data-entry="110"]').getBoundingClientRect();
          const view = document.querySelector(".content").getBoundingClientRect();
          return row.top >= view.top - 1 && row.top < view.bottom;
        },
        null,
        { timeout: 5000 }
      );
      // The reveal is used up: a later reload doesn't scroll again.
      await page.evaluate(() => document.querySelector(".content").scrollTo(0, 0));
      const lists = await countOf("journal_list");
      const answered = await settledCount("journal_list");
      await journalEmit("journal://changed", null);
      await pastCalls("journal_list", lists);
      await page.waitForFunction(([c, n]) => window.__ipc.settled.filter((x) => x === c).length > n, ["journal_list", answered], { timeout: 5000 });
      for (let i = 0; i < 3; i++) await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
      const top = await page.evaluate(() => document.querySelector(".content").scrollTop);
      if (top !== 0) throw new Error(`a reload scrolled the journal to ${top}`);
    });
    await page.click('nav.sidebar a[href="/"]');
    await goTo("/printer/journal?entry=2", "/printer/journal");
    await page.waitForSelector(journalDetail(2), { timeout: 15000 });
    // A link followed while the journal is open opens that entry too.
    await goTo("/printer/journal?entry=4", "/printer/journal");
    await page.waitForSelector(journalDetail(4), { timeout: 5000 });
    if (await page.locator(journalDetail(2)).count()) throw new Error("two entries open at once");
    await page.evaluate(() => document.querySelector(".content").scrollTo(0, 0));
  });

  // -- analyze from the journal --------------------------------------------------
  // `/analysis?journal=<id>` fills Analyze from an entry and links the result.
  const AN = ".print-analysis-page";
  const filamentPreset = FIXTURES.list_profiles[1].path;
  const journalNote = `${AN} .an-journal-note`;
  const lastAnalyze = async () => (await callsOf("analyze_print")).at(-1).args.request;
  const analyzeNow = async () => {
    const before = await countOf("analyze_print");
    await page.click("text=Analyze Print");
    await pastCalls("analyze_print", before);
    await page.waitForSelector(".analysis-results", { timeout: 15000 });
    return lastAnalyze();
  };

  await step(run, page, "Analyze a photo opens Analyze with the entry's photo, model and preset", async () => {
    await openJournal();
    await expand(2);
    await page.click(`${journalDetail(2)} .jr-analyze`);
    await page.waitForFunction(() => location.pathname === "/analysis" && location.search === "?journal=2", null, { timeout: 5000 });
    await page.waitForSelector(".analysis-preview img.preview-image", { timeout: 15000 });
    const link = page.locator('.an-journal a[href="/printer/journal?entry=2"]');
    if ((await link.innerText()).trim() !== JOURNAL_FINISHED.name) throw new Error("no link back to the entry");
    if (!(await page.locator(".an-journal").innerText()).startsWith("From the journal:")) throw new Error("the link line doesn't say where it is from");
    await page.waitForFunction((p) => document.querySelector(".profile-select")?.value === p, filamentPreset, { timeout: 5000 });
    if ((await page.locator(".material-selector select").inputValue()) !== "PLA") throw new Error("material not picked");
    if ((await page.locator(".an-model-name").innerText()).trim() !== "cube.stl") throw new Error("model not picked");
    const photos = (await callsOf("journal_image")).filter((c) => c.args.photoId === 7);
    if (!photos.length) throw new Error("the entry's newest photo wasn't asked for");
    const req = await analyzeNow();
    if (req.journal_id !== 2) throw new Error(`journal_id ${req.journal_id}`);
    if (req.profile_path !== filamentPreset || req.material_type !== "PLA") throw new Error(JSON.stringify({ ...req, image_base64: req.image_base64.length }));
    if (!req.image_base64 || req.image_base64.startsWith("data:")) throw new Error("the photo wasn't sent as bare base64");
    await page.click("text=Analyze Another Photo");
    await page.waitForSelector(".drop-zone", { timeout: 5000 });
    if (await page.locator(".an-journal").count()) throw new Error("the journal link outlived a reset");
    // A photo chosen after the reset isn't linked to the entry.
    await page.setInputFiles("#photo-file-input", { name: "print.png", mimeType: "image/png", buffer: png });
    await page.waitForSelector(".analysis-preview", { timeout: 20000 });
    const own = await analyzeNow();
    if (own.journal_id != null) throw new Error(`a photo of its own was linked to entry ${own.journal_id}`);
  });

  await step(run, page, "an entry with no photo still fills in the model, material and preset", async () => {
    await withFixtures({ journal_get: { ...JOURNAL_FINISHED, photos: [] } }, async () => {
      await page.click('nav.sidebar a[href="/"]');
      await goTo("/analysis?journal=2", "/analysis");
      await page.waitForSelector(".an-journal a", { timeout: 15000 });
      await page.waitForSelector(".drop-zone", { timeout: 5000 });
      if (await page.locator(".analysis-preview").count()) throw new Error("a photo was invented");
      await page.setInputFiles("#photo-file-input", { name: "print.png", mimeType: "image/png", buffer: png });
      await page.waitForSelector(".analysis-preview", { timeout: 20000 });
      await page.waitForFunction((p) => document.querySelector(".profile-select")?.value === p, filamentPreset, { timeout: 5000 });
      if ((await page.locator(".material-selector select").inputValue()) !== "PLA") throw new Error("material not picked");
      if ((await page.locator(".an-model-name").innerText()).trim() !== "cube.stl") throw new Error("model not picked");
      const req = await analyzeNow();
      if (req.journal_id !== 2) throw new Error(`journal_id ${req.journal_id}`);
    });
  });

  await step(run, page, "a photo that can't be read leaves the drop zone", async () => {
    await withFixtures({ journal_image: { __reject: "Photo unavailable" } }, async () => {
      await page.click('nav.sidebar a[href="/"]');
      const asked = await countOf("journal_image");
      const answered = await settledCount("journal_image");
      await goTo("/analysis?journal=2", "/analysis");
      await pastCalls("journal_image", asked);
      await settledPast("journal_image", answered);
      await page.waitForSelector(".an-journal a", { timeout: 15000 });
      for (let i = 0; i < 3; i++) await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
      await page.waitForSelector(".drop-zone", { timeout: 5000 });
      if (await page.locator(".analysis-preview").count()) throw new Error("a photo was shown for an unreadable file");
    });
  });

  await step(run, page, "an entry that is gone says so and leaves Analyze usable", async () => {
    await withFixtures({ journal_get: null }, async () => {
      await page.click('nav.sidebar a[href="/"]');
      await goTo("/analysis?journal=99", "/analysis");
      await page.waitForFunction(
        ([sel, text]) => document.querySelector(sel)?.innerText.includes(text),
        [journalNote, "That print isn't in the journal any more."],
        { timeout: 5000 }
      );
      if (await page.locator(".an-journal").count()) throw new Error("a link to a gone entry");
      await page.waitForSelector(".drop-zone", { timeout: 5000 });
      await page.setInputFiles("#photo-file-input", { name: "print.png", mimeType: "image/png", buffer: png });
      await page.waitForSelector(".analysis-preview", { timeout: 20000 });
      const req = await analyzeNow();
      if (req.journal_id != null) throw new Error(`journal_id ${req.journal_id} for a gone entry`);
    });
  });

  await step(run, page, "an older ?journal answer doesn't land over a newer link", async () => {
    const slow = { __hold: "an-journal-stale", answer: JOURNAL_FINISHED };
    await withFixtures({ journal_get: { __sequence: [slow, JOURNAL_GOOD] } }, async () => {
      await page.click('nav.sidebar a[href="/"]');
      const asked = await countOf("journal_get");
      const answered = await settledCount("journal_get");
      await goTo("/analysis?journal=2", "/analysis");
      await pastCalls("journal_get", asked);
      // The link changes in place while entry 2 is still being read.
      await goTo("/analysis?journal=4", "/analysis");
      await pastCalls("journal_get", asked + 1);
      await page.waitForFunction((name) => document.querySelector(".an-journal a")?.textContent === name, JOURNAL_GOOD.name, { timeout: 5000 });
      await page.evaluate(() => window.__release("an-journal-stale"));
      await page.waitForFunction((n) => window.__ipc.settled.filter((x) => x === "journal_get").length >= n, answered + 2, { timeout: 5000 });
      for (let i = 0; i < 3; i++) await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
      const name = (await page.locator(".an-journal a").innerText()).trim();
      if (name !== JOURNAL_GOOD.name) throw new Error(`the older answer landed: "${name}"`);
      if (await page.locator(".analysis-preview").count()) throw new Error("the older entry's photo landed");
      if (await page.locator('.an-journal a[href="/printer/journal?entry=2"]').count()) throw new Error("the older entry is linked");
    });
  });

  await step(run, page, "Try Again after a failed analysis keeps the journal photo and link", async () => {
    await page.click('nav.sidebar a[href="/"]');
    await goTo("/analysis?journal=2", "/analysis");
    await page.waitForSelector(".analysis-preview img.preview-image", { timeout: 15000 });
    await withFixtures({ analyze_print: { __reject: "The analysis service is unavailable." } }, async () => {
      const before = await countOf("analyze_print");
      await page.click("text=Analyze Print");
      await pastCalls("analyze_print", before);
      await page.waitForSelector(".error-state", { timeout: 5000 });
    });
    await page.click("text=Try Again");
    await page.waitForSelector(".analysis-preview img.preview-image", { timeout: 5000 });
    if (!(await page.locator('.an-journal a[href="/printer/journal?entry=2"]').count())) throw new Error("the journal link was dropped");
    const req = await analyzeNow();
    if (req.journal_id !== 2) throw new Error(`journal_id ${req.journal_id} after a retry`);
    if (!req.image_base64) throw new Error("the retry sent no photo");
    // Choosing a different photo is still an explicit reset.
    await page.click("text=Analyze Another Photo");
    await page.waitForSelector(".drop-zone", { timeout: 5000 });
    if (await page.locator(".an-journal").count()) throw new Error("the link outlived a reset");
  });

  await step(run, page, "an entry's prefill doesn't overwrite what the user already chose", async () => {
    const other = { ...JOURNAL_FINISHED, photos: [], slice: { ...JOURNAL_FINISHED.slice, model_path: "/Users/runner/models/other.stl" } };
    await withFixtures({ journal_get: { __hold: "an-journal-late", answer: other } }, async () => {
      await page.click('nav.sidebar a[href="/"]');
      await goTo("/analysis?journal=2", "/analysis");
      // The user gets on while the entry is still being read.
      await page.setInputFiles("#photo-file-input", { name: "print.png", mimeType: "image/png", buffer: png });
      await page.waitForSelector(".analysis-preview", { timeout: 20000 });
      await page.waitForSelector(".profile-select option:nth-child(2)", { state: "attached", timeout: 10000 });
      await page.selectOption(".material-selector select", "PETG");
      await page.selectOption(".profile-select", FIXTURES.list_profiles[0].path);
      await page.click(".an-pick-model");
      await page.waitForSelector(".an-model-name", { timeout: 5000 });
      const answered = await settledCount("journal_get");
      await page.evaluate(() => window.__release("an-journal-late"));
      await page.waitForFunction((n) => window.__ipc.settled.filter((x) => x === "journal_get").length > n, answered, { timeout: 5000 });
      await page.waitForSelector(".an-journal a", { timeout: 5000 });
      for (let i = 0; i < 3; i++) await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
      if ((await page.locator(".an-model-name").innerText()).trim() !== "cube.stl") throw new Error("the entry's model replaced the user's");
      if ((await page.locator(".material-selector select").inputValue()) !== "PETG") throw new Error("the entry's material replaced the user's");
      if ((await page.locator(".profile-select").inputValue()) !== FIXTURES.list_profiles[0].path) throw new Error("the entry's preset replaced the user's");
    });
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
      await goTo("/settings/about");
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
  const discoveryText = (sel, text) => page.waitForFunction(
    ([s, t]) => document.querySelector(s)?.textContent.includes(t), [sel, text], { timeout: 5000 }
  );

  await step(run, page, "discovery shows progress, prevents duplicate scans and accepts a late result", async () => {
    await withFixtures({ printer_discover: { __hold: "discovery", answer: FIXTURES.printer_discover } }, async () => {
      const before = (await callsOf("printer_discover")).length;
      await page.click(".printer-scan");
      await page.waitForSelector(".printer-scan[disabled]");
      await discoveryText(".printer-scan-progress", "seconds remaining");
      await page.waitForFunction(() => document.querySelector(".printer-scan-progress")?.textContent.includes("14 seconds remaining"));
      if (await page.locator(".printer-none, .printer-found-item").count()) throw new Error("premature discovery result");
      if ((await callsOf("printer_discover")).length !== before + 1) throw new Error("duplicate scan");
      await release("discovery");
      await page.waitForSelector(".printer-found-item");
      await page.waitForSelector(".printer-scan:not([disabled])");
      if (await page.locator(".printer-scan-progress").count()) throw new Error("progress did not stop");
    });
  });

  await step(run, page, "discovery errors stay distinct from a healthy empty scan and retry recovers", async () => {
    const error = "Couldn't listen for printer announcements. UDP 2021: permission denied";
    await withFixtures({ printer_discover: { __reject: error } }, async () => {
      await page.click(".printer-scan");
      await waitText(".printer-discovery-error", error);
      if (await page.locator(".printer-none").count()) throw new Error("failure reported as no printers");
    });
    await withFixtures({ printer_discover: { printers: [], warnings: [], listen_seconds: 15 } }, async () => {
      await page.click(".printer-scan");
      await discoveryText(".printer-none", "No printer announcements received in 15 seconds");
      if (await page.locator(".printer-discovery-error").count()) throw new Error("old error remained");
    });
    await withFixtures({ printer_discover: { ...FIXTURES.printer_discover, warnings: ["Cannot listen on UDP 1900: address in use"] } }, async () => {
      await page.click(".printer-scan");
      await page.waitForSelector(".printer-found-item");
      await discoveryText(".printer-discovery-warning", "UDP 1900");
      if (await page.locator(".printer-none, .printer-discovery-error").count()) throw new Error("partial success discarded");
    });
  });

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
    await setFixture("printer_discover", { printers: [
      { ip: "192.168.1.20", serial: PRINTER_SERIAL, name: "Workshop H2D", model: "H2D", conflict: true },
      { ip: "192.168.1.31", serial: "01P00A000000002", name: "", model: "P1S", conflict: false },
    ], warnings: [], listen_seconds: 15 });
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
    await goTo("/settings/about");
    await page.waitForSelector(".about-page", { timeout: 15000 });
    await page.waitForTimeout(800);
    if (run.errors.length !== errors) throw new Error(run.errors.slice(errors).join("; "));
    await page.click('a[href="/settings"]');
    await page.waitForSelector(".printer-settings", { timeout: 15000 });
  });

  await step(run, page, "Reset for clean install empties Settings → Printer", async () => {
    // A saved printer, shown when the section mounts.
    await setFixture("printer_get_config", FIXTURES.printer_save);
    await goTo("/settings/about");
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

  await step(run, page, "with AI off a journal entry offers no Analyze link", async () => {
    await page.click('nav.sidebar a[href="/printer"]');
    await page.click('.nd-tabs a[href="/printer/journal"]');
    await page.waitForSelector('.journal-page .jr-row[data-entry="2"]', { timeout: 15000 });
    if (!(await page.locator('.journal-page .jr-row[data-entry="2"] .jr-detail').count())) await page.click('.journal-page .jr-row[data-entry="2"] .jr-row-main');
    await page.waitForSelector('.journal-page .jr-row[data-entry="2"] .jr-detail .jr-delete', { timeout: 5000 });
    if (await page.locator(".journal-page .jr-analyze").count()) throw new Error("an Analyze link without AI");
    await page.click('a[href="/settings"]');
    await page.waitForSelector(".settings-page", { timeout: 15000 });
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

      await step(run, page, "the STL indicator opens and closes from the keyboard", async () => {
        const badge = page.locator(".stl-badge");
        // Start closed: the steps above leave the list open.
        if ((await badge.getAttribute("aria-expanded")) === "true") await badge.click();
        await page.waitForSelector(".stl-dropdown", { state: "detached", timeout: 5000 });
        const label = await badge.getAttribute("aria-label");
        if (label !== "1 STL file waiting") throw new Error(`aria-label "${label}"`);
        await badge.focus();
        await page.keyboard.press("Enter");
        await page.waitForSelector(".stl-dropdown", { timeout: 5000 });
        if ((await badge.getAttribute("aria-expanded")) !== "true") throw new Error("aria-expanded is not true");
        await page.keyboard.press("Escape");
        await page.waitForSelector(".stl-dropdown", { state: "detached", timeout: 5000 });
        const focused = await page.evaluate(() => document.activeElement?.classList.contains("stl-badge"));
        if (!focused) throw new Error("focus did not return to the STL button");
      });
    });
  }

  // -- health check ----------------------------------------------------------
  await step(run, page, "navigate to Health Check", async () => {
    await openTab("/settings", "/settings/health");
    await page.waitForSelector(".health-page", { timeout: 15000 });
    await page.waitForSelector(".health-results", { timeout: 20000 });
    const items = await page.locator(".health-results .health-item").count();
    if (items !== 6) throw new Error(`expected 6 health rows, got ${items}`);
    // The fixture has Bambu Studio and the profile folder OK, so nothing may
    // fail: unset API keys are optional and must not read as failures.
    const fails = await page.locator(".health-results .status-fail").count();
    if (fails !== 0) throw new Error(`${fails} failed health rows`);
    const word = (await page.locator(".health-hero .nd-hero-number").textContent()).trim();
    if (!/^(ALL GOOD|\d+ ISSUES?)$/.test(word)) throw new Error(`health hero reads "${word}"`);
    // Unset optional keys are known to be unset: a neutral dash, not a "?".
    // The fixture sets only the OpenAI key, so three rows are optional.
    const optional = await page.locator(".health-results .status-optional").allTextContents();
    if (optional.length !== 3 || optional.some((t) => t.trim() !== "\u2013")) {
      throw new Error(`optional badges read ${JSON.stringify(optional)}`);
    }
    const unknown = await page.locator(".health-results .status-unknown").count();
    if (unknown !== 0) throw new Error(`${unknown} unknown badges for keys that are known unset`);
    return word;
  });

  await step(run, page, "a failed health re-run reads through StateText", async () => {
    await withFixtures({ run_health_check: { __reject: "backend gone" } }, async () => {
      await page.locator(".health-page > button.btn-primary").click();
      await page.waitForSelector(".health-error .nd-state[data-kind='error']", { timeout: 10000 });
      const text = (await page.locator(".health-error .nd-state").textContent()).trim();
      if (!text.startsWith("[ERROR: Health check failed") || !text.includes("backend gone")) {
        throw new Error(`error reads "${text}"`);
      }
    });
    // Run it once more, so later steps and the screenshot see a clean page.
    await page.locator(".health-page > button.btn-primary").click();
    await page.waitForSelector(".health-error .nd-state", { state: "detached", timeout: 10000 });
  });

  // Every h2.nd-label is restyled by one `.page h2.nd-label` rule; without it
  // `.page h2` would make them 24px headings.
  await step(run, page, "section headings keep the 11px mono label look", async () => {
    const heading = page.locator(".diagnostics-panel h2.nd-label");
    if ((await heading.textContent()).trim() !== "Self-test diagnostics") throw new Error("diagnostics heading text");
    const css = await heading.evaluate((el) => {
      const c = getComputedStyle(el);
      return { size: c.fontSize, weight: c.fontWeight, transform: c.textTransform, family: c.fontFamily };
    });
    if (css.size !== "11px" || css.transform !== "uppercase" || css.weight !== "400" || !/mono/i.test(css.family)) {
      throw new Error(`diagnostics heading styled ${JSON.stringify(css)}`);
    }
    return JSON.stringify(css);
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
    await goTo("/settings/about");
    await page.waitForSelector(".about-page", { timeout: 15000 });
    const version = (await page.locator(".about-rows .nd-stat-row-value").first().innerText()).trim();
    if (!version.includes("1.3.0")) throw new Error(`version reads "${version}"`);
    return version;
  });

  // update_info is seeded by the startup check, so this renders without the
  // user pressing anything.
  await step(run, page, "update state reports up to date", async () => {
    await page.waitForSelector(".about-up-to-date", { timeout: 20000 });
    await page.screenshot({ path: `flow-${engine}-about.png`, fullPage: false });
    return (await page.locator(".about-up-to-date").first().innerText()).replace(/\s+/g, " ").trim();
  });

  await step(run, page, "a failed update check reads through StateText", async () => {
    await withFixtures({ check_for_updates: { __reject: "offline" } }, async () => {
      await page.locator(".about-update-actions button", { hasText: "Check for Updates" }).click();
      await page.waitForSelector(".about-check-error .nd-state[data-kind='error']", { timeout: 10000 });
      const text = (await page.locator(".about-check-error .nd-state").textContent()).trim();
      if (text !== "[ERROR: Could not check for updates: offline]") throw new Error(`error reads "${text}"`);
    });
  });

  await step(run, page, "About's Links heading keeps the 11px mono label look", async () => {
    const size = await page.locator(".about-links h2.nd-label").evaluate((el) => getComputedStyle(el).fontSize);
    if (size !== "11px") throw new Error(`Links heading is ${size}`);
  });

  // -- hubs ------------------------------------------------------------------
  await step(run, page, "old routes land on their hub tab and Back skips them", async () => {
    for (const [old, tab, selector] of [
      ["/batch", "/filament/batch", ".batch-generate-page"],
      ["/compare", "/profiles/compare", ".profile-diff-page"],
      ["/health", "/settings/health", ".health-page"],
      ["/about", "/settings/about", ".about-page"],
    ]) {
      await page.click('nav.sidebar a[href="/slice"]');
      await page.waitForSelector(".slice-page", { timeout: 15000 });
      await goTo(old, tab);
      await page.waitForFunction((t) => location.pathname === t, tab, { timeout: 5000 });
      await page.waitForSelector(selector, { timeout: 15000 });
      const current = await page.locator('.nd-tabs a[aria-current="page"]').getAttribute("href");
      if (current !== tab) throw new Error(`${old}: the current tab is ${current}`);
      await page.goBack();
      await page.waitForFunction(() => location.pathname === "/slice", null, { timeout: 5000 });
    }
  });

  await step(run, page, "switching tabs updates the URL and Back returns to the previous tab", async () => {
    await page.click('nav.sidebar a[href="/settings"]');
    await page.waitForSelector(".settings-page", { timeout: 15000 });
    await page.click('.nd-tabs a[href="/settings/about"]');
    await page.waitForFunction(() => location.pathname === "/settings/about", null, { timeout: 5000 });
    await page.waitForSelector(".about-page", { timeout: 15000 });
    await page.goBack();
    await page.waitForFunction(() => location.pathname === "/settings", null, { timeout: 5000 });
    await page.waitForSelector(".settings-page", { timeout: 15000 });
    const h1 = (await page.locator("h1.nd-page-title").textContent()).trim();
    if (h1 !== "Settings") throw new Error(`h1 reads "${h1}"`);
  });

  await step(run, page, "activating a tab from the keyboard keeps focus on that tab", async () => {
    // The header must stay mounted across a tab switch, or focus falls to <body>.
    for (const [hub, page_, tab, selector] of [
      ["/printer", ".printer-page", "/printer/journal", ".journal-page"],
      ["/settings", ".settings-page", "/settings/about", ".about-page"],
      ["/filament", ".filament-search-page", "/filament/batch", ".batch-generate-page"],
      ["/profiles", ".profile-management-page", "/profiles/compare", ".profile-diff-page"],
    ]) {
      await page.click(`nav.sidebar a[href="${hub}"]`);
      await page.waitForSelector(page_, { timeout: 15000 });
      await page.evaluate(() => { window.__header = document.querySelector(".nd-page-header"); });
      await page.focus(`.nd-tabs a[href="${tab}"]`);
      await page.keyboard.press("Enter");
      await page.waitForFunction((t) => location.pathname === t, tab, { timeout: 5000 });
      await page.waitForSelector(selector, { timeout: 15000 });
      const focused = await page.evaluate(() => document.activeElement?.getAttribute("href") ?? document.activeElement?.tagName);
      if (focused !== tab) throw new Error(`${hub}: focus is on ${focused} after opening ${tab}`);
      const kept = await page.evaluate(() => window.__header === document.querySelector(".nd-page-header"));
      if (!kept) throw new Error(`${hub}: the header was remounted`);
      const current = await page.locator('.nd-tabs a[aria-current="page"]').getAttribute("href");
      if (current !== tab) throw new Error(`${hub}: the current tab is ${current}`);
    }
  });

  await step(run, page, "an unknown route says so", async () => {
    await goTo("/nope");
    await page.waitForFunction(
      () => document.querySelector(".nd-state")?.textContent === "[EMPTY: Page not found]",
      null,
      { timeout: 5000 }
    );
    // The not-found view is already on screen when an old route is followed:
    // it must still redirect, and Back must not land on the old route.
    await goTo("/batch", "/filament/batch");
    await page.waitForFunction(() => location.pathname === "/filament/batch", null, { timeout: 5000 });
    await page.waitForSelector(".batch-generate-page", { timeout: 15000 });
    await page.goBack();
    await page.waitForFunction(() => location.pathname === "/nope", null, { timeout: 5000 });
    await page.click('nav.sidebar a[href="/"]');
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
    // Every loaded slot links to the Calibrate tab; an empty one has nothing to calibrate.
    const calHref = (label) => slotCard(label).locator(".pr-calibrate-link").getAttribute("href");
    if ((await calHref("A2")) !== "/filament/calibrate?slot=A2") throw new Error(`A2 Calibrate link: ${await calHref("A2")}`);
    if ((await calHref("A3")) !== "/filament/calibrate?slot=A3") throw new Error(`A3 Calibrate link: ${await calHref("A3")}`);
    if ((await slotCard("B4").locator(".pr-calibrate-link").count()) !== 0) throw new Error("an empty slot offers Calibrate");
    // Ten links, ten names: each says which slot it is for, and starts with its visible text.
    const calNames = await page.locator(".pr-slot .pr-calibrate-link").evaluateAll((as) => as.map((a) => [a.closest(".pr-slot").dataset.label, a.getAttribute("aria-label"), a.textContent.trim()]));
    if (calNames.length === 0 || new Set(calNames.map((n) => n[1])).size !== calNames.length) throw new Error(`Calibrate names are not distinct: ${JSON.stringify(calNames)}`);
    for (const [label, name, text] of calNames) if (name !== `Calibrate ${label}` || !name.startsWith(text)) throw new Error(`${label} Calibrate link is named "${name}"`);
    // The link sits beside the card's button, never inside it.
    if ((await slotCard("A2").locator(".pr-slot-main a").count()) !== 0) throw new Error("a link is nested in the card button");
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

  const detectedText = async (label) =>
    (await slotCard(label).locator(".pr-detected-text").innerText()).replace(/\s+/g, " ").trim();
  const a3Detected = (want) =>
    page.waitForFunction(
      (t) => document.querySelector('.pr-slot[data-label="A3"] .pr-detected-text')?.innerText.trim() === t,
      want,
      { timeout: 5000 }
    );

  await step(run, page, "slot cards name the detected profile, or say it isn't installed", async () => {
    if ((await detectedText("A2")) !== "Detected: Bambu PLA Basic") throw new Error(`A2 reads "${await detectedText("A2")}"`);
    const title = await slotCard("A2").locator(".pr-detected-text").getAttribute("title");
    if (title !== "Bambu PLA Basic @BBL H2D") throw new Error(`A2 tooltip "${title}"`);
    const spoken = (await slotCard("A2").locator(".pr-detected .nd-visually-hidden").textContent()).trim();
    if (spoken !== "Detected: Bambu PLA Basic @BBL H2D") throw new Error(`A2 label "${spoken}"`);
    if ((await detectedText("Ext-L")) !== "GFA01 — not installed") throw new Error(`Ext-L reads "${await detectedText("Ext-L")}"`);
    if ((await slotCard("Ext-L").locator(".pr-use-detected").count()) !== 0) throw new Error("Ext-L offers Use this");
    if ((await slotCard("B2").locator(".pr-use-detected").count()) !== 0) throw new Error("an assigned slot offers Use this");
    if ((await slotCard("A3").locator(".pr-use-detected").innerText()).trim() !== "Use this") throw new Error("A3 offers nothing");
    if ((await slotCard("B4").locator(".pr-detected").count()) !== 0) throw new Error("an empty slot shows a line");
    // No ID but a material: the material alone, never a guess.
    await emitEvent("printer://state", withSlot("A3", { tray: { ...PRINTER_VIEW.slots[2].tray, tray_info_idx: "" }, detected: null }));
    await a3Detected("PETG, no profile ID");
    // A raw ID of "0" is no ID either.
    await emitEvent("printer://state", withSlot("A3", { tray: { ...PRINTER_VIEW.slots[2].tray, tray_info_idx: "0" }, detected: null }));
    await a3Detected("PETG, no profile ID");
    // The ID names presets for this printer, but none for the nozzle in use.
    await emitEvent("printer://state", withSlot("A3", { detected: null, other_nozzle: 0.6 }));
    await a3Detected("No preset for the 0.6 mm nozzle");
    if ((await slotCard("A3").locator(".pr-detected").getAttribute("data-detected")) !== "other-nozzle") throw new Error("not marked other-nozzle");
    if ((await slotCard("A3").locator(".pr-use-detected").count()) !== 0) throw new Error("other nozzle offers Use this");
    await emitEvent("printer://state", PRINTER_VIEW);
    await a3Detected("Detected: Generic PETG");
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

  await step(run, page, "the printer page has the shared header and lists HMS codes seen earlier", async () => {
    const title = (await page.locator('.hub[data-hub="printer"] h1.nd-page-title').innerText()).trim();
    if (title !== "Printer") throw new Error(`page title reads "${title}"`);
    const code = PRINTER_VIEW.errors[0].code;
    // While active, the code sits in the active list and not under Earlier.
    if ((await page.locator(".pr-earlier-item", { hasText: code }).count()) !== 0) throw new Error("active code listed under Earlier");
    // Cleared: the history keeps it, the active list no longer does.
    await emitEvent("printer://state", { ...PRINTER_VIEW, errors: [] });
    await page.waitForSelector(".pr-earlier-item", { timeout: 5000 });
    const earlier = await page.locator(".pr-earlier-item").allInnerTexts();
    if (!earlier.some((t) => t.includes(code))) throw new Error(`Earlier lacks ${code}: ${earlier.join(" | ")}`);
    if ((await page.locator(".pr-error").count()) !== 0) throw new Error("cleared errors still active");
    // Reported again: back in the active list, gone from Earlier.
    await emitEvent("printer://state", PRINTER_VIEW);
    await page.waitForFunction((c) => [...document.querySelectorAll(".pr-error-code")].some((e) => e.textContent === c), code, {
      timeout: 5000,
    });
    if ((await page.locator(".pr-earlier-item", { hasText: code }).count()) !== 0) throw new Error("active code still under Earlier");
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

  await step(run, page, "Use this assigns the detected profile and opens no steps", async () => {
    await emitEvent("printer://state", PRINTER_VIEW);
    await a3Detected("Detected: Generic PETG");
    const matched = withSlot("A3", { assigned_preset: A3_DETECTED, assigned_filament_id: "GFG99", status: "matches" });
    await withFixtures({ printer_assign_slot: matched }, async () => {
      const before = (await callsOf("printer_assign_slot")).length;
      await slotCard("A3").locator(".pr-use-detected").click();
      await page.waitForFunction(
        (n) => window.__ipc.calls.filter((c) => c.cmd === "printer_assign_slot").length > n,
        before,
        { timeout: 5000 }
      );
      const last = (await callsOf("printer_assign_slot")).at(-1);
      if (!sameArgs(last.args, { amsId: 0, trayId: 2, presetPath: A3_DETECTED_PATH })) throw new Error(JSON.stringify(last.args));
      await page.waitForFunction(
        () => document.querySelector('.pr-slot[data-label="A3"] .pr-status')?.innerText.trim() === "✓ Set",
        null,
        { timeout: 5000 }
      );
      if ((await slotCard("A3").locator(".pr-steps").count()) !== 0) throw new Error("the steps opened");
      if ((await page.locator(".pr-picker").count()) !== 0) throw new Error("the picker opened");
      if ((await slotCard("A3").locator(".pr-use-detected").count()) !== 0) throw new Error("Use this still offered");
    });
    await emitEvent("printer://state", PRINTER_VIEW);
    await page.waitForFunction(
      () => document.querySelector('.pr-slot[data-label="A3"] .pr-status')?.innerText.trim() === "Not set",
      null,
      { timeout: 5000 }
    );
  });

  await step(run, page, "a different assignment shows Yours and Detected and keeps the steps", async () => {
    await emitEvent("printer://state", withSlot("A3", A3_ASSIGNED));
    await a3Detected("Yours: Polymaker PolyLite PLA · Detected: Generic PETG");
    if ((await badgeOf("A3")) !== "Set on printer") throw new Error(`A3 shows "${await badgeOf("A3")}"`);
    if ((await slotCard("A3").locator(".pr-steps").count()) !== 1) throw new Error("the steps went away");
    if ((await slotCard("A3").locator(".pr-use-detected").count()) !== 0) throw new Error("Use this offered over an assignment");
    await emitEvent("printer://state", PRINTER_VIEW);
    await a3Detected("Detected: Generic PETG");
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
    if ((await page.locator('.hub[data-hub="printer"] :is(b, i, u)').count()) !== 0) {
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
    // The picker's backdrop covers the rail, so follow a link the way one elsewhere in the app would.
    await goTo("/settings/about");
    await page.waitForSelector(".about-page", { timeout: 15000 });
    await page.waitForTimeout(800);
    if ((await callsOf("printer_assign_slot")).length !== before + 1) throw new Error("assign was not sent");
    if (run.errors.length !== errors) throw new Error(run.errors.slice(errors).join("; "));
  });

  // -- home cockpit -----------------------------------------------------------
  // textContent, not innerText: the hero label and the footer are uppercased by CSS.
  const homeMode = (mode) => page.waitForSelector(`.home-page[data-mode="${mode}"]`, { timeout: 5000 });
  const homeText = (sel) => page.locator(sel).first().evaluate((el) => el.textContent.replace(/\s+/g, " ").trim());
  const RUNNING_CLEAN = { ...PRINTER_VIEW, errors: [] };
  const FINISHED = { ...RUNNING_CLEAN, state: { ...PRINTER_VIEW.state, gcode_state: "FINISH", mc_percent: 100 } };

  await step(run, page, "home leads with a printer error", async () => {
    await setFixture("printer_view", PRINTER_VIEW);
    await page.click('nav.sidebar a[href="/"]');
    await emitEvent("printer://state", PRINTER_VIEW);
    await homeMode("attention");
    const hero = await homeText(".home-page .nd-hero");
    for (const want of ["ERROR", "Needs attention · 0300_400C", "Look up 0300_400C on the Bambu wiki."]) {
      if (!hero.includes(want)) throw new Error(`hero lacks "${want}": ${hero}`);
    }
  });

  await step(run, page, "What does this mean? puts the error in the agent composer", async () => {
    const starts = (await callsOf("agent_start")).length + (await callsOf("agent_send")).length;
    await page.click(".hm-ask");
    await page.waitForSelector(".agent-drawer.open", { timeout: 5000 });
    await page.waitForFunction(
      () => document.querySelector(".ag-input")?.value.includes("0300_400C"),
      null,
      { timeout: 5000 }
    );
    // Nothing is sent until the user presses send.
    if ((await callsOf("agent_start")).length + (await callsOf("agent_send")).length !== starts) {
      throw new Error("the question was sent without the user");
    }
    await page.fill(".ag-input", "");
    // The open drawer covers the toggle, so close it with its own button.
    await page.click(".ag-close");
    await page.waitForSelector(".agent-drawer.open", { state: "detached", timeout: 5000 });
  });

  await step(run, page, "home shows the print in progress", async () => {
    await emitEvent("printer://state", RUNNING_CLEAN);
    await homeMode("printing");
    const number = await homeText(".home-page .nd-hero-number");
    if (number !== "6") throw new Error(`hero number "${number}"`);
    const hero = await homeText(".home-page .nd-hero");
    if (!hero.includes("9 h 09 m left · Layer 1 / 200 · Nozzle 245 °C · Bed 70 °C")) throw new Error(hero);
    const on = await page.locator(".home-page .nd-seg-on").count();
    if (on !== 1) throw new Error(`${on} segments filled for 6%`);
  });

  await step(run, page, "an HMS code during a print keeps the print and names the code", async () => {
    // BambuMate can't tell an informational HMS code from a fatal one.
    await emitEvent("printer://state", { ...PRINTER_VIEW, errors: [PRINTER_VIEW.errors[0]] });
    await homeMode("printing");
    const label = await homeText(".home-page .nd-hero-label");
    if (!label.includes("HMS 0300_0100_0001_0007")) throw new Error(`label reads "${label}"`);
    const number = await homeText(".home-page .nd-hero-number");
    if (number !== "6") throw new Error(`hero number "${number}"`);
    if ((await page.locator(".home-page .nd-segbar").count()) !== 1) throw new Error("progress bar hidden");
    const name = await page.locator(".home-page .nd-segbar").getAttribute("aria-label");
    if (name !== "Print progress") throw new Error(`progress bar is named "${name}"`);
    // An HMS code with nothing printing needs attention.
    await emitEvent("printer://state", { ...PRINTER_VIEW, state: { ...PRINTER_VIEW.state, gcode_state: "IDLE" }, errors: [PRINTER_VIEW.errors[0]] });
    await homeMode("attention");
    await emitEvent("printer://state", RUNNING_CLEAN);
    await homeMode("printing");
  });

  await step(run, page, "home marks the slot in use with text, not only a colour", async () => {
    await emitEvent("printer://state", { ...RUNNING_CLEAN, state: { ...RUNNING_CLEAN.state, tray_now: 2 } });
    await homeMode("printing");
    const slot = page.locator('.hm-slot[data-slot-label="A3"][data-in-use="signal"]');
    await slot.waitFor({ timeout: 5000 });
    const marker = (await slot.locator(".hm-in-use").textContent()).trim();
    if (marker !== "In use") throw new Error(`marker reads "${marker}"`);
    if ((await page.locator(".hm-in-use").count()) !== 1) throw new Error("more than one slot marked in use");
    await emitEvent("printer://state", RUNNING_CLEAN);
    await page.waitForFunction(() => document.querySelector(".hm-in-use") === null, null, { timeout: 5000 });
  });

  await step(run, page, "home AMS slots open the preset picker", async () => {
    const slots = await page.locator(".hm-slot").count();
    if (slots !== PRINTER_SLOTS.length) throw new Error(`${slots} slots, expected ${PRINTER_SLOTS.length}`);
    const homeCal = await page.locator('.hm-slot[data-slot-label="A3"] .pr-calibrate-link').getAttribute("href");
    if (homeCal !== "/filament/calibrate?slot=A3") throw new Error(`Home A3 Calibrate link: ${homeCal}`);
    const homeNames = await page.locator(".hm-slot .pr-calibrate-link").evaluateAll((as) => as.map((a) => [a.closest(".hm-slot").dataset.slotLabel, a.getAttribute("aria-label")]));
    if (homeNames.length === 0 || new Set(homeNames.map((n) => n[1])).size !== homeNames.length) throw new Error(`Home Calibrate names are not distinct: ${JSON.stringify(homeNames)}`);
    for (const [label, name] of homeNames) if (name !== `Calibrate ${label}`) throw new Error(`Home ${label} Calibrate link is named "${name}"`);
    await page.click('.hm-slot[data-slot-label="A3"] .hm-slot-main');
    await page.waitForSelector(".pr-picker", { timeout: 5000 });
    await page.click(".pr-picker-cancel");
    await page.waitForSelector(".pr-picker", { state: "detached", timeout: 5000 });
  });

  await step(run, page, "home is ready after a print finishes and offers to rate it", async () => {
    const ended = { ...JOURNAL_FINISHED, ended_at: Math.floor(Date.now() / 1000) };
    await withFixtures({ journal_last: ended }, async () => {
      await emitEvent("printer://state", FINISHED);
      await emitEvent("journal://changed", null);
      await homeMode("idle");
      const number = await homeText(".home-page .nd-hero-number");
      if (number !== "READY") throw new Error(`hero number "${number}"`);
      await page.waitForFunction(
        (n) => document.querySelector(".home-page .nd-hero")?.textContent.includes(`Last print finished just now · ${n}`),
        JOURNAL_FINISHED.name,
        { timeout: 5000 }
      );
      const rate = page.locator(".home-page .hm-rate");
      if ((await rate.innerText()).trim() !== "Rate your last print") throw new Error("no rate link");
      if ((await rate.getAttribute("href")) !== "/printer/journal?entry=2") throw new Error("rate link goes elsewhere");
    });
    await withFixtures({ journal_last: { ...JOURNAL_FINISHED, verdict: "good" } }, async () => {
      await emitEvent("journal://changed", null);
      await page.waitForSelector(".home-page .hm-rate", { state: "detached", timeout: 5000 });
    });
    await emitEvent("journal://changed", null);
    await page.waitForSelector(".home-page .hm-rate", { timeout: 5000 });
    await page.click(".home-page .hm-rate");
    await page.waitForSelector('.journal-page .jr-row[data-entry="2"] .jr-detail', { timeout: 15000 });
    // Home re-reads the printer on mount; the event puts the finished print back.
    await page.click('nav.sidebar a[href="/"]');
    await emitEvent("printer://state", FINISHED);
    await homeMode("idle");
  });

  await step(run, page, "home names the newest sliced job's filament until it is loaded", async () => {
    const gear = sliceResult();
    gear.filaments = [
      { ...gear.filaments[0], preset: "Bambu PETG HF @BBL H2C", filament_type: "PETG", filament_id: "GFG02", material: "petg", color: "#1F7A3D" },
    ];
    gear.plates[0].filaments = [{ ...gear.plates[0].filaments[0], filament_type: "PETG", color: "#1F7A3D" }];
    await page.evaluate(
      (v) => window.__emit("slicer://job", v),
      sliceJob(900, { state: "done", result: gear, cached: false }, { model_name: "Gear.stl", filament: "Bambu PETG HF @BBL H2C" })
    );
    const line = (text) =>
      page.waitForFunction(
        (t) => document.querySelector(".home-page .hm-ams-line")?.textContent.trim() === t,
        text,
        { timeout: 5000 }
      );
    // A3 holds PETG with another profile.
    await line("Gear needs PETG · different profile in A3");
    if ((await page.locator(".home-page .hm-ams-line").getAttribute("href")) !== "/slice") throw new Error("the line doesn't link to Slice");
    if ((await page.locator(".home-page .nd-hero").count()) !== 1) throw new Error("a second hero");
    const without = (label, patch) => ({ ...FINISHED, slots: FINISHED.slots.map((s) => (s.label === label ? { ...s, ...patch, tray: { ...s.tray, ...(patch.tray ?? {}) } } : s)) });
    await emitEvent("printer://state", without("A3", { tray: { empty: true, tray_type: "", tray_info_idx: "" }, material: null, detected: null, status: "empty" }));
    await line("Gear needs PETG · not loaded");
    // Loaded exactly: the line goes.
    await emitEvent("printer://state", without("A3", { tray: { tray_info_idx: "GFG02" } }));
    await page.waitForSelector(".home-page .hm-ams-line", { state: "detached", timeout: 5000 });
    // While printing there is no idle hero, so no line.
    await emitEvent("printer://state", RUNNING_CLEAN);
    await homeMode("printing");
    if ((await page.locator(".home-page .hm-ams-line").count()) !== 0) throw new Error("line shown while printing");
    await emitEvent("printer://state", FINISHED);
    await homeMode("idle");
    await line("Gear needs PETG · different profile in A3");
  });

  await step(run, page, "home never shows a stale percentage after a disconnect", async () => {
    await emitEvent("printer://state", RUNNING_CLEAN);
    await homeMode("printing");
    await emitEvent("printer://connection", { state: "unreachable" });
    await homeMode("no-printer");
    const hero = await homeText(".home-page .nd-hero");
    if (!hero.includes("[DISCONNECTED]")) throw new Error(hero);
    const number = await homeText(".home-page .nd-hero-number");
    if (number !== String(FIXTURES.list_profiles.length)) throw new Error(`hero number "${number}"`);
    if ((await page.locator(".hm-ams").count()) !== 0) throw new Error("AMS shown while disconnected");
    await emitEvent("printer://connection", { state: "connected" });
    await homeMode("printing");
  });

  await step(run, page, "home footer shows recent work and sync status", async () => {
    // Earlier steps (Health repair) may have changed these answers; start from the defaults
    // and remount Home so it reads them.
    await setFixture("list_unsynced_presets", FIXTURES.list_unsynced_presets);
    await setFixture("latest_generated_preset", FIXTURES.latest_generated_preset);
    await page.click('nav.sidebar a[href="/printer"]');
    await page.click('nav.sidebar a[href="/"]');
    await page.waitForSelector(".hm-recent span", { timeout: 5000 });
    await page.waitForSelector(".hm-sync a.hm-sync-warn", { timeout: 5000 });
    const recent = await homeText(".hm-recent");
    if (!recent.startsWith("Recent · Polymaker PETG Red · ")) throw new Error(`recent reads "${recent}"`);
    const sync = page.locator(".hm-sync a.hm-sync-warn");
    const text = (await sync.textContent()).trim();
    if (text !== "2 presets not syncing") throw new Error(`sync reads "${text}"`);
    if ((await sync.getAttribute("href")) !== "/settings/health") throw new Error("sync link goes elsewhere");
  });

  await step(run, page, "a failed scan shows inline and the rest of Home renders", async () => {
    await withFixtures({ list_unsynced_presets: { __reject: "scan failed" } }, async () => {
      await page.click('nav.sidebar a[href="/printer"]');
      await page.click('nav.sidebar a[href="/"]');
      await page.waitForFunction(
        () => document.querySelector(".hm-sync .nd-state")?.textContent === "[ERROR: scan failed]",
        null,
        { timeout: 5000 }
      );
      if ((await page.locator(".home-page .nd-hero").count()) !== 1) throw new Error("hero missing");
    });
  });

  await step(run, page, "home with no printer leads with profiles and offers to connect", async () => {
    // The previous step left Home showing a failed scan; remount it to re-read.
    await page.click('nav.sidebar a[href="/printer"]');
    await page.click('nav.sidebar a[href="/"]');
    await emitEvent("printer://state", PRINTER_UNCONFIGURED);
    await homeMode("no-printer");
    const label = await homeText(".home-page .nd-hero-label");
    if (label !== "BambuMate") throw new Error(`label reads "${label}"`);
    // The count is user presets and the detail claims only what the scan saw.
    await page.waitForFunction(
      () => /\d+\s*presets/.test(document.querySelector(".home-page .nd-hero")?.textContent ?? "")
        && /not syncing|No sync problems found/.test(document.querySelector(".home-page .nd-hero")?.textContent ?? ""),
      null,
      { timeout: 5000 }
    );
    if ((await page.locator(".hm-connect").getAttribute("href")) !== "/settings#printer") throw new Error("connect link");
    await emitEvent("printer://state", PRINTER_VIEW);
    await homeMode("attention");
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
    if (model !== "gpt-6.1-sol") throw new Error(`default model is "${model}", expected "gpt-6.1-sol"`);
    if (await page.locator(".ag-effort").inputValue() !== "medium") throw new Error("reasoning default is not medium");
    return model;
  });

  await step(run, page, "sending starts a session and a turn", async () => {
    await page.fill(".ag-input", "Why is my PETG stringing?");
    await page.press(".ag-input", "Enter");
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "agent_send"), null, { timeout: 5000 });
    const start = await called("agent_start");
    if (start[0].args.provider !== "codex" || start[0].args.model !== "gpt-6.1-sol" || start[0].args.effort !== "medium") {
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

  await step(run, page, "explicit Codex model and reasoning choices survive reopening and start the next chat", async () => {
    await page.selectOption(".ag-model", "gpt-test");
    await page.selectOption(".ag-effort", "high");
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "set_preference" && c.args.key === "agent_codex_effort" && c.args.value === "high"));
    await page.click(".ag-close");
    await page.click(".agent-toggle");
    await page.waitForFunction(() => document.querySelector(".ag-model")?.value === "gpt-test" && document.querySelector(".ag-effort")?.value === "high", null, { timeout:5000 });
    const base = (await called("agent_start")).length;
    await page.fill(".ag-input", "Explain this slice.");
    await page.press(".ag-input", "Enter");
    await page.waitForFunction((n) => window.__ipc.calls.filter((c) => c.cmd === "agent_start").length > n, base);
    const start = (await called("agent_start")).at(-1).args;
    if (start.model !== "gpt-test" || start.effort !== "high") throw new Error(JSON.stringify(start));
  });

  run.unknown = await page.evaluate(() => [...new Set(window.__ipc.unknown)]).catch(() => []);

  const wizard = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  wizard.on("pageerror", (e) => run.errors.push(`pageerror: ${e.message}`));
  wizard.on("console", (m) => { if (m.type() === "error") run.errors.push(`console.error: ${m.text()}`); });
  await wizard.addInitScript(installTauriMock, {
    ...FIXTURES,
    check_setup_complete: { bambu_studio_path: null, ai_provider: null, has_api_key: false, setup_complete: false },
  });
  await step(run, wizard, "setup distinguishes analysis providers from agent chat without requesting agent auth", async () => {
    await wizard.goto(baseUrl);
    await wizard.waitForSelector(".wizard-overlay");
    const counter = (await wizard.locator(".wizard-counter").textContent()).trim();
    if (counter !== "01 / 05") throw new Error(`counter reads "${counter}"`);
    if ((await wizard.locator("h1.wizard-question").count()) !== 1) throw new Error("expected one wizard question h1");
    await wizard.click(".wizard-footer .btn-primary");
    await wizard.locator(".wizard-mode-card", { hasText: "Use Analysis AI" }).click();
    await wizard.click(".wizard-footer .btn-primary");
    await wizard.getByRole("heading", { name: "Analysis Provider", exact: true }).waitFor();
    const note = await wizard.locator(".wizard-agent-note").innerText();
    for (const text of ["Agent panel", "Codex", "Claude", "does not configure automatic extraction or Print Analysis"]) {
      if (!note.includes(text)) throw new Error(`missing explanation: ${text}`);
    }
    if (await wizard.locator(".wizard-provider-card", { hasText: "Codex" }).count()) throw new Error("CLI chat advertised as analysis provider");
    await wizard.screenshot({ path: `flow-${engine}-setup-analysis.png` });
  });
  await step(run, wizard, "deferring analysis reports save errors, persists and leaves agent chat available", async () => {
    await wizard.evaluate(() => { window.__fixtures.set_preference = { __sequence: [{ __reject: "disk full" }, null] }; });
    await wizard.getByRole("button", { name: "Configure analysis later" }).click();
    await wizard.waitForFunction(() => document.querySelector(".wizard-error")?.textContent.includes("disk full"));
    if (!(await wizard.locator(".wizard-overlay").count())) throw new Error("setup closed before choice was saved");
    await wizard.getByRole("button", { name: "Configure analysis later" }).click();
    await wizard.waitForSelector(".wizard-overlay", { state: "detached" });
    await wizard.reload();
    await wizard.waitForSelector("nav.sidebar");
    if (await wizard.locator(".wizard-overlay").count()) throw new Error("setup reopened without an analysis key");
    await wizard.click(".agent-toggle");
    await wizard.waitForSelector(".agent-drawer.open");
    const starts = await wizard.evaluate(() => window.__ipc.calls.filter((c) => c.cmd === "agent_start" || c.cmd === "store_api_key"));
    if (starts.length) throw new Error("deferring analysis started inference or stored a key");
  });
  run.unknown.push(...await wizard.evaluate(() => [...new Set(window.__ipc.unknown)]).catch(() => []));
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
