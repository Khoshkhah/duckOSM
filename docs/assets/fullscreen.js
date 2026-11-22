// Full screen for the embedded maps (iframe) of the docs: each map gets a "Full screen" button above it; a click on the map acts in the
// map. The pictures have no zoom and no full screen (Kaveh, 2026-11-22: no zoom pointer on a picture that does not zoom).
// The theme can load a page with no full page load (navigation.instant), so the buttons are added again after each page change.
const full = (el) => {
  if (el.requestFullscreen) el.requestFullscreen();
  else window.open(el.src, "_blank");              // no fullscreen API (iPhone Safari): its own tab
};

function addButtons() {
  for (const f of document.querySelectorAll(".md-content iframe")) {
    if (f.previousElementSibling && f.previousElementSibling.classList.contains("dk-frame-bar")) continue;
    const bar = document.createElement("div"), b = document.createElement("button");
    bar.className = "dk-frame-bar";
    b.type = "button"; b.className = "dk-fullscreen";
    b.innerHTML = '<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path fill="currentColor" ' +
      'd="M3 3h7v2H5v5H3zm11 0h7v7h-2V5h-5zM3 14h2v5h5v2H3zm16 0h2v7h-7v-2h5z"/></svg> Full screen';
    b.addEventListener("click", () => full(f));
    bar.appendChild(b); f.before(bar);
  }
}

// the theme's document$ gives each page, also with instant loading; without it, the one page load
const start = () => (window.document$ && window.document$.subscribe ? window.document$.subscribe(addButtons) : addButtons());
if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start); else start();
