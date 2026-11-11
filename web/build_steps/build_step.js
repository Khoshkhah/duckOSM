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
	root.innerHTML = `<button type="button" class="sb-primary" data-bs="run" disabled>Build</button><p class="small muted" data-bs="why"></p>
		<progress data-bs="bar" max="1" hidden></progress><p class="rs-status" role="status" aria-live="polite" data-bs="status"></p>
		<p class="rs-field-msg err" role="alert" data-bs="error" hidden></p>
		<details data-bs="log" hidden><summary>Steps</summary><ol class="bs-steps" data-bs="steps"></ol></details>`;
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
		q("steps").innerHTML = (job.steps ?? []).map((s) => `<li>${esc(s)}</li>`).join("");
	}

	async function watch() {
		for (;;) {
			const job = await poll();
			show(job);
			if (job.state === "done") { q("status").textContent = `Built in ${Math.round(job.seconds)} s.`; return onDone(job); }
			if (job.state === "error") { q("error").hidden = false; q("error").textContent = job.error; return; }
			await new Promise((ok) => setTimeout(ok, every));
		}
	}

	q("run").addEventListener("click", async () => {
		running = true;
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
