// The Result step: what the build made, and the way to the map of the new area (roadstyle's dashboard with the panel Edit OSM data).
import { esc } from "./core.js";

/**
 * initResultStep({ root, href })  // root: the element that gets the step; href: the address of the map page of the new area (the host's)
 * It returns { show(job), clear() }. show(job) takes the state of a finished build ({ name, db, text, seconds }).
 */
export function initResultStep({ root, href }) {
	root.classList.add("bs", "bs-result");
	root.innerHTML = `<p class="small muted" data-rs="none">The result of the build comes here.</p>
		<div data-rs="done" hidden><p class="small" data-rs="line"></p><pre class="bs-summary" data-rs="text"></pre>
			<a class="sb-primary bs-open" data-rs="open">Open the map and edit OSM data</a></div>`;
	const q = (k) => root.querySelector(`[data-rs="${k}"]`);
	q("open").href = href;
	return {
		show(job) {
			q("none").hidden = true; q("done").hidden = false;
			q("line").innerHTML = `<b>${esc(job.name)}</b> in <code>${esc(job.db)}</code>, built in ${Math.round(job.seconds)} s.`;
			q("text").textContent = job.text;
		},
		clear() { q("none").hidden = false; q("done").hidden = true; },
	};
}
