// The Build step: the button, and the progress of the build. The host starts the build and reads its state; the step shows the download, the steps and
// the result or the error. The state of a build is { state: "idle" | "downloading" | "building" | "done" | "error", steps: [...], bytes, total, seconds, text, error }.
import { esc, progressText } from "./core.js";

/**
 * initBuildStep({
 *   root,     // the element that gets the step
 *   ready,    // () gives "" when the build can start, else the reason (a text)
 *   run,      // () starts the build (a promise; throws an Error when the host refuses it)
 *   poll,     // () gives a promise of the state of the build
 *   onDone,   // optional: (job) the build ended well
 *   every,    // optional: the time between two reads of the state, in ms (default 500)
 * })
 * It returns { refresh(), busy() }. refresh() reads `ready` again: call it when a step before changes.
 */
export function initBuildStep({ root, ready, run, poll, onDone = () => {}, every = 500 }) {
	root.classList.add("bs", "bs-build");
	root.innerHTML = `<div class="build-go"><button type="button" class="sb-primary" data-bs="run" disabled>Build<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12h14M13 6l6 6-6 6"/></svg></button><p class="why" data-bs="why"></p></div>
		<progress data-bs="bar" max="1" hidden></progress><p class="rs-status" role="status" aria-live="polite"><span class="bs-spin" data-bs="spin" hidden aria-hidden="true"></span><span data-bs="status"></span></p>
		<p class="rs-field-msg err" role="alert" data-bs="error" hidden></p>
		<details class="drow" data-bs="log" hidden><summary>Time of each part</summary><table class="bs-steplog" data-bs="steps"><thead><tr><th>Part</th><th>Time (s)</th></tr></thead><tbody></tbody><tfoot><tr><td>Total</td><td data-bs="total"></td></tr></tfoot></table></details>`;
	const q = (k) => root.querySelector(`[data-bs="${k}"]`);
	let running = false;

	function refresh() {
		const why = running ? "" : ready();
		q("run").disabled = running || !!why;
		q("why").textContent = why;
	}

	function show(job) {
		q("status").textContent = progressText(job);
		const bar = q("bar");
		bar.hidden = !(job.state === "downloading" && job.total);
		if (!bar.hidden) { bar.max = job.total; bar.value = job.bytes; }
		q("log").hidden = !job.steps?.length;
		// the time of each part, with its name, while the build runs and after it: from its start to the start of the next part. The part that runs now has the time so far, and a mark.
		const at = job.step_secs ?? [], steps = job.steps ?? [], live = job.state === "building" || job.state === "downloading";
		q("steps").tBodies[0].innerHTML = steps.map((st, i) => {
			const running = live && i === steps.length - 1, sec = (at[i + 1] ?? job.seconds ?? 0) - (at[i] ?? 0);
			return `<tr${running ? ' class="run" aria-current="step"' : ""}><td>${esc(st)}${running ? ' <span class="bs-run">running</span>' : ""}</td><td>${Math.max(0, sec).toFixed(1)}</td></tr>`;
		}).join("");
		q("total").textContent = Number(job.seconds ?? 0).toFixed(1);
		if (live && q("log").dataset.opened !== "1") { q("log").open = true; q("log").dataset.opened = "1"; }      // open while the build runs: the person sees the time of each part
	}

	async function watch() {
		let lost = 0, first = 0;                                            // the page tells when the server does not answer: running is not the same as frozen
		for (;;) {
			let job;
			try { job = await poll(); lost = 0; } catch (e) {
				first ||= Date.now(); lost++;
				q("spin").hidden = true;
				q("status").textContent = `No answer from the server for ${Math.round((Date.now() - first) / 1000)} s. The page tries again. Is the command still running?`;
				if (lost > 120) throw e;
				await new Promise((ok) => setTimeout(ok, every));
				continue;
			}
			first = 0;
			q("spin").hidden = !(job.state === "building" || job.state === "downloading");           // a turning mark: the page is alive and the build runs
			show(job);
			if (job.state === "done") { q("spin").hidden = true; q("status").textContent = `Built in ${Math.round(job.seconds)} s.`; return onDone(job); }
			if (job.state === "error") { q("spin").hidden = true; q("error").hidden = false; q("error").textContent = job.error; return; }
			await new Promise((ok) => setTimeout(ok, every));
		}
	}

	q("run").addEventListener("click", async () => {
		running = true;
		q("log").dataset.opened = "";
		q("error").hidden = true; q("status").textContent = "Starting…";
		refresh();
		try {
			await run();
			await watch();
		} catch (e) {
			q("error").hidden = false; q("error").textContent = e.message;
			q("status").textContent = "";
		}
		running = false;
		refresh();
	});
	refresh();
	return { refresh, busy: () => running };
}
