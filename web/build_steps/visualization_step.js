// The Visualization step: the roads of the new area on the map, with the panel Edit OSM data. The map is a page of the host (roadstyle's dashboard of the new database), so the step
// is a button that opens it, and a choice that opens it by itself when the build ends (off at the start; the host keeps the choice). The host keeps the open area, so its
// page of the steps shows the same area again when the person comes back from the map.

/**
 * initVisualizationStep({
 *   root,        // the element that gets the step
 *   href,        // the address of the map page of the new area
 *   open,        // optional: (href) opens it (default: the browser goes there)
 *   auto,        // optional: the choice "Open the map when the build ends" at the start (default false)
 *   keep,        // optional: { get(), set(on) }: where the host keeps the choice (default: localStorage)
 *   delay,       // optional: ms between the end of the build and the opening (default 1500); a click on Cancel, or the choice off, stops it
 * })
 * It returns { ready(on), built(), auto() }. ready(on) enables the button; built() is called when a build ended well: it opens the map after `delay` when the choice is on.
 */
export function initVisualizationStep({ root, href, open = (h) => { location.href = h; }, auto = false, keep = null, delay = 1500 }) {
	root.classList.add("bs", "bs-viz");
	const store = keep ?? { get: () => { try { const v = localStorage.getItem("duckosm-studio-open-map-tab"); return v === null ? auto : v === "1"; } catch { return auto; } },
		set: (on) => { try { localStorage.setItem("duckosm-studio-open-map-tab", on ? "1" : "0"); } catch { /* no storage */ } } };
	root.innerHTML = `<p class="small muted" data-vz="note">The roads of the area on the map, with the panel Edit OSM data. Each change goes into the fixes file of the area, and Build again builds the area again with it.</p>
		<label class="rs-opt"><input type="checkbox" data-vz="auto"><span><b>Open the map when the build ends</b></span></label>
		<a class="sb-primary bs-open" data-vz="open" aria-disabled="true">Open the map and edit OSM data</a>
		<p class="rs-status" role="status" aria-live="polite" data-vz="status"></p>`;
	const q = (k) => root.querySelector(`[data-vz="${k}"]`);
	q("auto").checked = store.get();
	q("auto").addEventListener("change", () => { store.set(q("auto").checked); if (!q("auto").checked) stop(); });
	let timer = 0;
	const stop = () => { clearTimeout(timer); timer = 0; q("status").textContent = ""; };
	function ready(on) {
		q("open").toggleAttribute("href", on); if (on) q("open").href = href;
		q("open").setAttribute("aria-disabled", String(!on));
		if (!on) stop();
	}
	q("open").addEventListener("click", (e) => { if (q("open").getAttribute("aria-disabled") === "true") e.preventDefault(); else stop(); });
	ready(false);
	return {
		ready,
		auto: () => q("auto").checked,
		built() {
			ready(true);
			if (!q("auto").checked) return;
			q("status").innerHTML = 'Opening the map… <button type="button" class="rs-link" data-vz="cancel">Cancel</button>';
			q("cancel").addEventListener("click", stop);
			timer = setTimeout(() => open(href), delay);
		},
	};
}
