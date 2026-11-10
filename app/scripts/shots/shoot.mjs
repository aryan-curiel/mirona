// Takes the docs screenshots (docs/images/guide/), the README/landing page shot
// (docs/images/meeting.png) and the tour GIF (docs/images/demo.gif, needs ffmpeg) from the real
// UI over demo data (mock.ts). From app/: `pnpm shots` (once before:
// `npx playwright-core install chromium`). `pnpm shots meeting tour` retakes only those.
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";
import { createServer } from "vite";

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "../../..");
const out = join(repo, "docs/images/guide");
mkdirSync(out, { recursive: true });

// Fresh demo data: its dates are relative to today.
writeFileSync(join(here, "demo.json"), execFileSync("cargo", ["run", "-q", "-p", "mirona-store", "--example", "demo_json"], { cwd: repo }));

const server = await createServer({ root: join(here, "../.."), server: { port: 1430, strictPort: true }, logLevel: "error" });
await server.listen();
const browser = await chromium.launch();
const base = "http://localhost:1430/scripts/shots/index.html";

const store = (page, state) => page.evaluate((s) => window.miStore.setState(s), state);
const settle = (page) => page.waitForTimeout(600);
const MAIN = [1440, 920]; // the windows' sizes in src-tauri

async function open(shot, extra = {}) {
  const page = await browser.newPage({ viewport: { width: shot.size[0], height: shot.size[1] }, deviceScaleFactor: shot.scale ?? 1.5, reducedMotion: "reduce", ...extra });
  page.on("pageerror", (e) => console.error(e.message));
  await page.goto(`${base}?${shot.query ?? "window=main"}`);
  await page.waitForLoadState("networkidle");
  // Headless Chromium drops backdrop blur under an opacity animation: show dialogs settled.
  await page.addStyleTag({ content: ".mi-scrim, .mi-pop { animation: none !important; }" });
  await settle(page);
  return page;
}

/** name → window size, query, and what to do before the shot. */
const SHOTS = {
  meeting: { size: MAIN },
  hero: { size: MAIN, scale: 4 / 3, file: "../meeting.png" },
  inspector: { size: MAIN, go: (p) => p.evaluate(() => window.miStore.getState().selectSegment("c6")) },
  "name-voices": { size: MAIN, query: "window=main&unnamed=1", async go(p) {
    await p.getByRole("button", { name: "Name them" }).click();
    await p.evaluate(() => document.activeElement?.blur());
  } },
  "meeting-summary": { size: MAIN, go: (p) => store(p, { tab: "summary" }) },
  "meeting-notes": { size: MAIN, go: (p) => store(p, { tab: "notes" }) },
  recording: { size: MAIN, async go(p) {
    const m = await p.evaluate(() => window.miStore.getState().meeting);
    const levels = Array.from({ length: 2 }, (_, i) => 0.35 + i * 0.3);
    await store(p, { recording: { meetingId: m.id, elapsedMs: 754_000, levels, drafts: [] } });
  } },
  "all-meetings": { size: MAIN, go: (p) => store(p, { view: "all" }) },
  search: { size: MAIN, async go(p) {
    const box = p.getByRole("searchbox").or(p.getByPlaceholder(/search/i)).first();
    await box.click();
    await box.fill("insurer");
    await p.keyboard.press("Enter");
  } },
  "action-items": { size: MAIN, go: (p) => store(p, { view: "actions" }) },
  people: { size: MAIN, go: (p) => store(p, { view: "people" }) },
  "agent-activity": { size: MAIN, go: (p) => store(p, { view: "activity" }) },
  consent: { size: MAIN, go: (p) => store(p, { consent: { call: false } }) },
  "call-banner": { size: MAIN, go: (p) => store(p, { call: { app: "Zoom", title: "Claims Desk weekly sync", tag: "project:Alder Restoration" } }) },
  ...Object.fromEntries(
    ["general", "privacy", "models", "tags", "agents", "advanced", "about"].map((t) => [`settings-${t}`, { size: [1040, 760], query: `window=settings&tab=${t}` }]),
  ),
  ...Object.fromEntries(
    [0, 1, 2, 3].map((s) => [`onboarding-${s + 1}`, { size: [980, 660], query: `window=onboarding&step=${s}${s === 1 ? "&models=missing" : ""}` }]),
  ),
  tray: { size: [304, 264], query: "window=tray" },
};

const only = process.argv.slice(2);
for (const [name, shot] of Object.entries(SHOTS)) {
  if (only.length && !only.includes(name)) continue;
  const page = await open(shot);
  if (shot.go) await shot.go(page);
  await settle(page);
  const file = shot.file ?? `${name}.jpg`;
  await page.screenshot({ path: join(out, file), ...(file.endsWith(".jpg") && { quality: 85 }) });
  console.log(join("docs/images/guide", file));
  await page.close();
}

// The tour: inspect a line, the summary, action items, search. Recorded, then a GIF.
if (!only.length || only.includes("tour")) {
  const dir = mkdtempSync(join(tmpdir(), "mirona-tour-"));
  const size = { width: 1440, height: 920 };
  const page = await open({ size: MAIN, scale: 1 }, { recordVideo: { dir, size } });
  const pause = (ms) => page.waitForTimeout(ms);
  await pause(1200);
  await page.getByText("let's ship it Monday").click();
  await pause(2200);
  await page.getByRole("tab", { name: "Summary" }).or(page.getByRole("button", { name: "Summary", exact: true })).first().click();
  await pause(2200);
  await page.getByRole("button", { name: /^Action items/ }).first().click();
  await pause(2200);
  const box = page.getByPlaceholder(/search all/i);
  await box.click();
  await box.pressSequentially("insurer", { delay: 90 });
  await page.keyboard.press("Enter");
  await pause(2500);
  const video = await page.video().path();
  await page.close();
  const gif = join(out, "../demo.gif");
  const vf = "fps=6,scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=96[p];[b][p]paletteuse=dither=bayer:bayer_scale=4";
  execFileSync("ffmpeg", ["-y", "-loglevel", "error", "-ss", "0.8", "-i", video, "-vf", vf, gif]);
  rmSync(dir, { recursive: true });
  console.log("docs/images/demo.gif");
}
await browser.close();
await server.close();
