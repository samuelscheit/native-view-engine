// Theme scenarios for the hydrated docs site (imported by run_docs_e2e.mjs):
//  1. First paint matches the OS appearance (all four), before hydration: the SSR appearance
//     overrides paint what the client paints after hydrating (no dark flash), with 0 mismatches.
//  2. Live OS appearance changes mid-session (light <-> dark, prefers-contrast: more).
//  3. The System / Light / Dark control pins a mode that survives SPA popstate and a reload
//     (persisted, applied before first paint), and System clears it.

const APPEARANCES = [
  { scheme: "light", contrast: "no-preference" },
  { scheme: "dark", contrast: "no-preference" },
  { scheme: "light", contrast: "more" },
  { scheme: "dark", contrast: "more" },
];

function emulate(cdp, scheme, contrast) {
  return cdp.send("Emulation.setEmulatedMedia", {
    features: [
      { name: "prefers-color-scheme", value: scheme },
      { name: "prefers-contrast", value: contrast },
    ],
  });
}

// Paint of the shell landmarks, read in-page (`root` = docs-root).
const PAINT_EXPR = `(() => {
  const css = (sel, prop) => { const el = document.querySelector(sel); return el ? getComputedStyle(el)[prop] : null; };
  return {
    body: getComputedStyle(document.body).backgroundColor,
    root: css('[data-testid="docs-root"]', "backgroundColor"),
    nav: css('[data-testid="docs-navbar"]', "backgroundColor"),
    h1: css("#app h1", "color"),
  };
})()`;

const CAPTURE_FIRST_PAINT = `
  window.__NVE_FIRST_PAINT__ = null;
  document.addEventListener("DOMContentLoaded", () => {
    window.__NVE_FIRST_PAINT__ = { hydrated: !!window.__NVE_DOCS__, paint: ${PAINT_EXPR} };
  });
`;

async function hydrated(cdp, label) {
  await cdp.waitFor(`!!window.__NVE_DOCS__ || !!window.__NVE_DOCS_ERROR__`, 12000, `${label} hydration`);
  const err = await cdp.eval(`window.__NVE_DOCS_ERROR__ || null`);
  if (err) throw new Error(`${label}: hydration error ${err}`);
}

export async function runThemeScenarios({ createPageCdp, debugPort, origin, shotsDir, path, assert }) {
  console.log("==> Theme: first paint per OS appearance, live OS changes, persisted user mode");

  // 1. First paint == hydrated paint for every OS appearance, on the landing and a docs page.
  for (const route of ["/", "/docs/guides/ui-components/"]) {
    for (const { scheme, contrast } of APPEARANCES) {
      const label = `${route} ${scheme}/${contrast}`;
      const { cdp } = await createPageCdp(debugPort);
      await cdp.setViewport(1440, 900, false, 1);
      await emulate(cdp, scheme, contrast);
      await cdp.send("Page.addScriptToEvaluateOnNewDocument", { source: CAPTURE_FIRST_PAINT });
      await cdp.navigate(`${origin}${route}`);
      await hydrated(cdp, label);
      const first = await cdp.eval(`window.__NVE_FIRST_PAINT__`);
      assert(first && !first.hydrated, `${label}: first paint must be captured before hydration`);
      await cdp.waitFor(`!document.querySelector("style[data-nve-docs-appearance]")`, 4000, `${label} override removal`);
      const after = await cdp.eval(PAINT_EXPR);
      for (const key of ["body", "root", "nav", "h1"]) {
        assert(first.paint[key] === after[key], `${label}: ${key} flashed ${first.paint[key]} -> ${after[key]}`);
      }
      const info = await cdp.eval(`({ m: window.__NVE_DOCS__.mismatches, r: window.__NVE_DOCS__.replaced_nodes, dark: window.__NVE_DOCS__.dark_mode })`);
      assert(info.m === 0 && info.r === 0, `${label}: mismatches=${info.m} replaced=${info.r}`);
      assert(info.dark === (scheme === "dark"), `${label}: dark_mode=${info.dark}`);
      if (route === "/" && contrast === "no-preference") {
        await cdp.capturePng(path.join(shotsDir, `theme-first-paint-${scheme}.png`));
      }
      assert(cdp.consoleErrors.length === 0, `${label} console errors: ${cdp.consoleErrors.join(" | ")}`);
      cdp.close();
    }
  }
  console.log("    ✓ First paint matches all 4 OS appearances before hydration (0 mismatches, no flash)");

  // 2. Live OS changes mid-session.
  const { cdp } = await createPageCdp(debugPort);
  await cdp.setViewport(1440, 900, false, 1);
  await emulate(cdp, "dark", "no-preference");
  await cdp.navigate(`${origin}/`);
  await hydrated(cdp, "live OS");
  const darkRoot = (await cdp.eval(PAINT_EXPR)).root;
  await emulate(cdp, "light", "no-preference");
  await cdp.waitFor(`window.__NVE_DOCS__.dark_mode === false`, 4000, "live OS -> light");
  const lightRoot = (await cdp.eval(PAINT_EXPR)).root;
  assert(lightRoot === "rgb(246, 246, 248)", `live light canvas, got ${lightRoot}`);
  await emulate(cdp, "light", "more");
  await cdp.waitFor(`${PAINT_EXPR}.root !== ${JSON.stringify(lightRoot)}`, 4000, "live prefers-contrast: more");
  await emulate(cdp, "dark", "no-preference");
  await cdp.waitFor(`${PAINT_EXPR}.root === ${JSON.stringify(darkRoot)}`, 4000, "live OS -> dark");
  console.log("    ✓ Live OS appearance changes re-theme mid-session (light, increased contrast, dark)");

  // 3. Pinned user mode: SPA popstate + reload keep it; System clears it.
  await cdp.eval(`document.querySelector('[data-testid="docs-theme-mode-light"]').click()`);
  await cdp.waitFor(`window.__NVE_DOCS__.theme_mode === "light" && window.__NVE_DOCS__.dark_mode === false`, 4000, "pin light");
  const stored = await cdp.eval(`({ ls: localStorage.getItem("nve-docs-theme-mode"), attr: document.documentElement.getAttribute("data-theme-mode") })`);
  assert(stored.ls === "light" && stored.attr === "light", `pinned mode persisted: ${JSON.stringify(stored)}`);
  await cdp.eval(`window.__THEME_ALIVE__ = 1; document.querySelector('[data-testid="docs-section-docs"]').click()`);
  await cdp.waitFor(`location.pathname.endsWith("/docs")`, 5000, "SPA nav to /docs");
  await cdp.eval(`history.back()`);
  await cdp.waitFor(`!location.pathname.endsWith("/docs") && !!document.querySelector('[data-testid="landing-page"]')`, 5000, "popstate back to /");
  assert((await cdp.eval(`window.__THEME_ALIVE__`)) === 1, "popstate must not reload");
  const afterPop = await cdp.eval(`({ mode: window.__NVE_DOCS__.theme_mode, dark: window.__NVE_DOCS__.dark_mode, root: ${PAINT_EXPR}.root })`);
  assert(afterPop.mode === "light" && !afterPop.dark && afterPop.root === lightRoot, `popstate kept the pinned mode: ${JSON.stringify(afterPop)}`);

  await cdp.send("Page.addScriptToEvaluateOnNewDocument", { source: CAPTURE_FIRST_PAINT });
  await cdp.navigate(`${origin}/`);
  await hydrated(cdp, "reload with pinned light");
  const reloaded = await cdp.eval(`({ first: window.__NVE_FIRST_PAINT__, mode: window.__NVE_DOCS__.theme_mode, m: window.__NVE_DOCS__.mismatches })`);
  assert(reloaded.first && !reloaded.first.hydrated && reloaded.first.paint.root === lightRoot, `reload first paint honors the pinned mode on a dark OS: ${JSON.stringify(reloaded.first)}`);
  assert(reloaded.mode === "light" && reloaded.m === 0, `reload restored the pinned mode: ${JSON.stringify(reloaded)}`);
  await cdp.capturePng(path.join(shotsDir, "theme-pinned-light-on-dark-os.png"));

  await cdp.eval(`document.querySelector('[data-testid="docs-theme-mode-system"]').click()`);
  await cdp.waitFor(`window.__NVE_DOCS__.theme_mode === "system" && window.__NVE_DOCS__.dark_mode === true`, 4000, "back to System");
  const cleared = await cdp.eval(`({ ls: localStorage.getItem("nve-docs-theme-mode"), attr: document.documentElement.getAttribute("data-theme-mode") })`);
  assert(cleared.ls === null && cleared.attr === null, `System clears the pinned mode: ${JSON.stringify(cleared)}`);
  assert(cdp.consoleErrors.length === 0, `theme console errors: ${cdp.consoleErrors.join(" | ")}`);
  assert(cdp.pageErrors.length === 0, `theme page errors: ${cdp.pageErrors.join(" | ")}`);
  cdp.close();
  console.log("    ✓ Pinned Light survives popstate and reload (applied before first paint); System clears it");
}
