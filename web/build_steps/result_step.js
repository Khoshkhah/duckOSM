// The Result step: what the build made, and the way to the map of the new area (roadstyle's dashboard with the panel Edit OSM data).
import { bytes, esc } from "./core.js";

/**
 * initResultStep({ root })  // root: the element that gets the step
 * It returns { show(job), clear() }. show(job) takes the state of a finished build ({ name, db, text, seconds }).
 */
export function initResultStep({ root, href }) {
	root.classList.add("bs", "bs-result");
	root.innerHTML = `<p class="small muted" data-rs="none">The result of the build comes here.</p>
		<div data-rs="done" hidden><p class="small" data-rs="line"></p><p class="small muted">Saved in <code class="bs-path" data-rs="dir"></code></p><table class="rs-table bs-files" data-rs="files"><thead><tr><th>File</th><th>Size</th></tr></thead><tbody></tbody></table><table class="rs-table bs-nets" data-rs="nets"><thead><tr><th>Network</th><th>Roads</th><th>Nodes</th></tr></thead><tbody></tbody></table><pre class="bs-summary" data-rs="text"></pre>
			</div>`;
	const q = (k) => root.querySelector(`[data-rs="${k}"]`);
	return {
		show(job) {
			q("none").hidden = true; q("done").hidden = false;
			q("line").innerHTML = `<b>${esc(job.name)}</b> in <code>${esc(job.db)}</code>, built in ${Math.round(job.seconds)} s.`;
			// the networks of the build, from its summary (the other lines are the log of the build)
			const rows = [...String(job.text ?? "").matchAll(/^(\w+): ([\d,]+) edges, ([\d,]+) nodes/gm)];
			q("nets").hidden = !rows.length;
			q("nets").tBodies[0].innerHTML = rows.map((m) => `<tr><td>${esc(m[1][0].toUpperCase() + m[1].slice(1))}</td><td>${esc(m[2])}</td><td>${esc(m[3])}</td></tr>`).join("");
			q("dir").textContent = job.dir ?? "";
			q("files").hidden = !job.files?.length;
			q("files").tBodies[0].innerHTML = (job.files ?? []).map((f) => `<tr><td>${esc(f.name)}</td><td>${esc(bytes(f.size))}</td></tr>`).join("");
			q("text").textContent = String(job.text ?? "").split("\n").filter((l) => !/^\w+: [\d,]+ edges, /.test(l)).join("\n");
			q("text").hidden = !q("text").textContent;
		},
		clear() { q("none").hidden = false; q("done").hidden = true; },
	};
}
