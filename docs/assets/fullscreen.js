// Full screen for the embedded maps (iframe) and the diagrams (mermaid) of the docs. Each one gets a "Full screen" button
// above it. A map in full screen acts as the map. A diagram opens in a window over the page, where the wheel zooms, a drag
// moves it, and Esc or the close button closes it (Kaveh, 2026-11-24). The pictures have no zoom and no full screen (Kaveh,
// 2026-11-22: no zoom pointer on a picture that does not zoom).
// The theme can load a page with no full page load (navigation.instant), so the buttons are added again after each page change.
const ICON = '<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path fill="currentColor" ' +
  'd="M3 3h7v2H5v5H3zm11 0h7v7h-2V5h-5zM3 14h2v5h5v2H3zm16 0h2v7h-7v-2h5z"/></svg> Full screen';
const PAN_ZOOM = "https://cdn.jsdelivr.net/npm/svg-pan-zoom@3.6.2/dist/svg-pan-zoom.min.js";

const full = (el) => {
  if (el.requestFullscreen) el.requestFullscreen();
  else window.open(el.src, "_blank");              // no fullscreen API (iPhone Safari): its own tab
};

function button(before, onClick) {
  const bar = document.createElement("div"), b = document.createElement("button");
  bar.className = "dk-frame-bar";
  b.type = "button"; b.className = "dk-fullscreen"; b.innerHTML = ICON;
  b.addEventListener("click", onClick);
  bar.appendChild(b); before.before(bar);
}

const hasBar = (el) => el.previousElementSibling && el.previousElementSibling.classList.contains("dk-frame-bar");

// The theme draws each diagram in a closed shadow root, and its source is then gone from the page. The source comes from
// the HTML of the page: the n-th <pre class="mermaid"> there is the n-th diagram here.
let sources = null;
async function diagramSource(n) {
  if (!sources || sources.url !== location.pathname) {
    const html = await (await fetch(location.pathname)).text();
    const doc = new DOMParser().parseFromString(html, "text/html");
    sources = { url: location.pathname, list: [...doc.querySelectorAll(".md-content pre.mermaid")].map((p) => p.textContent) };
  }
  return sources.list[n];
}

let panZoomLoaded = null;
const loadPanZoom = () => panZoomLoaded || (panZoomLoaded = new Promise((ok, fail) => {
  const s = document.createElement("script");
  s.src = PAN_ZOOM; s.onload = ok; s.onerror = fail; document.head.appendChild(s);
}));

let drawn = 0;
async function openDiagram(n) {
  const pop = document.createElement("div");
  pop.className = "dk-diagram-pop";
  pop.innerHTML = '<button type="button" class="dk-fullscreen dk-diagram-close">Close (Esc)</button><div class="dk-diagram-svg"></div>';
  document.body.appendChild(pop);
  const close = () => { pop.remove(); document.removeEventListener("keydown", onKey); };
  const onKey = (e) => { if (e.key === "Escape") close(); };
  document.addEventListener("keydown", onKey);
  pop.querySelector(".dk-diagram-close").addEventListener("click", close);
  const box = pop.querySelector(".dk-diagram-svg");
  try {
    const [text] = await Promise.all([diagramSource(n), loadPanZoom()]);
    box.innerHTML = (await window.mermaid.render("dk-diagram-" + drawn++, text)).svg;
    const svg = box.querySelector("svg");
    svg.removeAttribute("height"); svg.style.width = "100%"; svg.style.height = "100%"; svg.style.maxWidth = "none";
    window.svgPanZoom(svg, { controlIconsEnabled: true, fit: true, center: true, minZoom: 0.3, maxZoom: 40, zoomScaleSensitivity: 0.3 });
  } catch (e) {
    box.textContent = "The diagram cannot open: " + (e.message || e);
  }
}

function addButtons() {
  for (const f of document.querySelectorAll(".md-content iframe")) {
    if (!hasBar(f)) button(f, () => full(f));
  }
  document.querySelectorAll(".md-content div.mermaid, .md-content pre.mermaid").forEach((d, n) => {
    if (hasBar(d)) return;
    button(d, () => openDiagram(n));
    d.addEventListener("dblclick", () => openDiagram(n));
  });
}

// The theme draws the content after this script runs, and again on each page of its instant loading. So addButtons runs
// at the start and after each change of the page; it skips what has a button already, so its own change stops there.
let queued = false;
const start = () => {
  addButtons();
  new MutationObserver(() => {
    if (queued) return;
    queued = true;
    requestAnimationFrame(() => { queued = false; addButtons(); });
  }).observe(document.body, { childList: true, subtree: true });
};
if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start); else start();
