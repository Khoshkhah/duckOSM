// Full screen for the embedded maps (iframe) and the pictures of the docs. A map gets a "Full screen" button above it: a click on the map
// acts in the map. A picture gets no button (Kaveh, 2026-11-06: a large button over each picture is too much): a click on the picture opens
// it full screen, with the zoom-in pointer and a tooltip.
document.addEventListener("DOMContentLoaded", () => {
  for (const f of document.querySelectorAll('.md-content iframe, .md-content img[src*="images/"]')) {
    const full = () => {
      if (f.requestFullscreen) f.requestFullscreen();
      else window.open(f.src, "_blank");            // no fullscreen API (iPhone Safari): its own tab
    };
    if (f.tagName === "IMG") { f.title ||= "Click to see the picture full screen"; f.addEventListener("click", full); continue; }
    const bar = document.createElement("div"), b = document.createElement("button");
    bar.className = "dk-frame-bar";
    b.type = "button"; b.className = "dk-fullscreen";
    b.innerHTML = '<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path fill="currentColor" ' +
      'd="M3 3h7v2H5v5H3zm11 0h7v7h-2V5h-5zM3 14h2v5h5v2H3zm16 0h2v7h-7v-2h5z"/></svg> Full screen';
    b.addEventListener("click", full);
    bar.appendChild(b); f.before(bar);
  }
});
