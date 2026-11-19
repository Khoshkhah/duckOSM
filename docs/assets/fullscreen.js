// Full screen for the embedded maps (iframe) and the pictures of the docs. A map gets a "Full screen" button above it: a click on the map
// acts in the map. A picture gets no button (Kaveh, 2026-11-06: a large button over each picture is too much): a click on the picture opens
// it full screen, with the zoom-in pointer and a tooltip.
// The theme can load a page with no full page load (navigation.instant), so a picture click is caught on the document, and the map
// buttons are added again after each page change.
const full = (el) => {
  if (el.requestFullscreen) el.requestFullscreen();
  else window.open(el.src, "_blank");              // no fullscreen API (iPhone Safari): its own tab
};

if (!window.dkFullClick) window.dkFullClick = document.addEventListener("click", (e) => {
  const img = e.target.closest && e.target.closest('.md-content img[src*="images/"]');
  if (img) full(img);
}) || true;  // once, also if the theme runs this script again on a page change

function addButtons() {
  for (const img of document.querySelectorAll('.md-content img[src*="images/"]')) img.title ||= "Click to see the picture full screen";
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
