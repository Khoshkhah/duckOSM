// The folder of the areas: where the areas are saved and listed. It shows the folder, and a picker to choose another: a path (typed, or reached by the folders that the host lists).
// A browser cannot give the path of a folder, so the host (a server on this computer) lists the folders. Part of the card "Your areas".

/**
 * initFolderStep({
 *   root,       // the element that gets the step
 *   folder,     // () gives a promise of the folder now (a path)
 *   list,       // (path) gives a promise of { path, parent, dirs: [names] }; throws an Error that says why a path is not a folder ("" lists the folder now)
 *   choose,     // (path, create) uses the folder (a promise); `create`: make it when it is not there
 *   onChange,   // optional: the folder is new
 * })
 * It returns { refresh() }.
 */
export function initFolderStep({ root, folder, list, choose, onChange = () => {} }) {
	root.classList.add("bs", "bs-folder");
	root.innerHTML = `<p class="small muted">Folder of the areas: the areas are saved and listed here.</p>
		<div class="area-acts"><code class="bs-path" data-fs="path"></code><button type="button" data-fs="change">Change…</button></div>
		<div data-fs="picker" hidden>
			<input type="text" data-fs="input" aria-label="Path of a folder" autocomplete="off" spellcheck="false">
			<div class="rs-opts" data-fs="dirs" role="group" aria-label="The folders inside"></div>
			<p class="rs-field-msg" role="status" data-fs="msg"></p>
			<div class="area-acts"><button type="button" data-fs="up">Up</button><button type="button" class="sb-primary" data-fs="use">Use this folder</button></div>
		</div>`;
	const q = (k) => root.querySelector(`[data-fs="${k}"]`);
	let shown = "", missing = false, asked = 0;                                             // asked: the number of the last look; an older answer that comes late is not used
	const msg = (t, bad = false) => { q("msg").textContent = t; q("msg").classList.toggle("err", bad); };

	async function look(path) {
		const n = ++asked;
		try {
			const r = await list(path);
			if (n !== asked) return;
			shown = r.path; missing = false;
			q("input").value = r.path;
			q("up").disabled = !r.parent; q("up").dataset.to = r.parent ?? "";
			q("dirs").innerHTML = r.dirs.map((d) => `<button type="button" class="rs-opt" data-dir="${d.replace(/"/g, "&quot;")}"><span><b></b></span></button>`).join("");
			[...q("dirs").children].forEach((b, i) => { b.querySelector("b").textContent = r.dirs[i]; });
			q("use").textContent = "Use this folder";
			msg(r.dirs.length ? "" : "No folder inside.");
		} catch (e) {
			if (n !== asked) return;
			missing = true;                                                                      // a path that is not there: the person can make it
			q("dirs").innerHTML = ""; q("use").textContent = "Create and use this folder";
			msg(e.message, true);
		}
	}
	const paint = async () => { q("path").textContent = await folder(); };
	q("change").addEventListener("click", () => { q("picker").hidden = !q("picker").hidden; if (!q("picker").hidden) look(""); });
	q("up").addEventListener("click", () => look(q("up").dataset.to));
	q("dirs").addEventListener("click", (e) => { const b = e.target.closest("[data-dir]"); if (b) look(`${shown.replace(/[\\/]$/, "")}${shown.includes("\\") ? "\\" : "/"}${b.dataset.dir}`); });
	q("input").addEventListener("keydown", (e) => { if (e.key === "Enter") look(q("input").value); });
	q("input").addEventListener("change", () => look(q("input").value));
	q("use").addEventListener("click", async () => {
		try {
			await choose(q("input").value.trim(), missing);
			q("picker").hidden = true;
			await paint();
			onChange();
		} catch (e) { msg(e.message, true); }
	});
	paint();
	return { refresh: paint };
}
