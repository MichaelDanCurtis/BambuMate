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
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

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
