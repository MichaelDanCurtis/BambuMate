// WCAG contrast of the design tokens in both themes, in real WebKit and
// Chromium. Reads the computed custom properties from style/tokens.css, so it
// checks what the app actually ships.
import { chromium, webkit } from "playwright";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const repoRoot = resolve(process.argv[2] ?? "../..");
const css = readFileSync(resolve(repoRoot, "style/tokens.css"), "utf8").replace(/@font-face\s*{[^}]*}/g, "");

const TEXT = [
  ["--nd-text-primary", 4.5],
  ["--nd-text-secondary", 4.5],
  ["--nd-text-display", 4.5],
  ["--nd-signal", 4.5],
  ["--nd-warning", 4.5],
  ["--nd-accent", 4.5],
  ["--nd-interactive", 4.5],
  ["--nd-text-disabled", 3],
];
const BACKGROUNDS = ["--nd-black", "--nd-surface", "--nd-surface-raised"];
const THEMES = ["dark", "bambu"];

function luminance(hex) {
  const n = hex.replace("#", "");
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(n.slice(i, i + 2), 16) / 255);
  const lin = (c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

function contrast(a, b) {
  const [l1, l2] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (l1 + 0.05) / (l2 + 0.05);
}

async function run(browserType, name) {
  const browser = await browserType.launch();
  const page = await browser.newPage();
  await page.setContent(`<!doctype html><html><head><style>${css}</style></head><body></body></html>`);
  const failures = [];
  for (const theme of THEMES) {
    const vars = await page.evaluate(
      ({ theme, names }) => {
        document.documentElement.setAttribute("data-theme", theme);
        const cs = getComputedStyle(document.documentElement);
        return Object.fromEntries(names.map((n) => [n, cs.getPropertyValue(n).trim()]));
      },
      { theme, names: [...TEXT.map(([n]) => n), ...BACKGROUNDS] }
    );
    for (const [fg, min] of TEXT) {
      for (const bg of BACKGROUNDS) {
        const f = vars[fg];
        const b = vars[bg];
        if (!/^#[0-9a-fA-F]{6}$/.test(f) || !/^#[0-9a-fA-F]{6}$/.test(b)) {
          failures.push(`${theme}: ${fg}=${f || "(unset)"} on ${bg}=${b || "(unset)"} is not a #rrggbb token`);
          continue;
        }
        const ratio = contrast(f, b);
        const ok = ratio >= min;
        console.log(`  ${name} ${ok ? "OK  " : "FAIL"} ${theme} ${fg} on ${bg}: ${ratio.toFixed(2)}:1 (min ${min})`);
        if (!ok) failures.push(`${theme}: ${fg} on ${bg} = ${ratio.toFixed(2)}:1 < ${min}`);
      }
    }
  }
  await browser.close();
  return failures;
}

const failures = [...(await run(webkit, "webkit")), ...(await run(chromium, "chromium"))];
if (failures.length) {
  console.error(`\n${failures.length} contrast failure(s):\n` + failures.join("\n"));
  process.exit(1);
}
console.log("\nAll token contrasts pass.");
