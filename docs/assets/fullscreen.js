// A "Full screen" button above every embedded map (iframe) in the docs.
document.addEventListener("DOMContentLoaded", () => {
  for (const f of document.querySelectorAll(".md-content iframe")) {
    const bar = document.createElement("div"), b = document.createElement("button");
    bar.className = "dk-frame-bar";
    b.type = "button"; b.className = "dk-fullscreen";
    b.innerHTML = '<svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true"><path fill="currentColor" ' +
      'd="M3 3h7v2H5v5H3zm11 0h7v7h-2V5h-5zM3 14h2v5h5v2H3zm16 0h2v7h-7v-2h5z"/></svg> Full screen';
    b.addEventListener("click", () => {
      if (f.requestFullscreen) f.requestFullscreen();
      else window.open(f.src, "_blank");            // no fullscreen API (iPhone Safari): its own tab
    });
    bar.appendChild(b); f.before(bar);
  }
});
