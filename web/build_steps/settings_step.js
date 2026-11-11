// The Settings step: the build config. The networks and the options of OPTIONS (core.js), as the keys of the config of `duckosm build`
// (docs/reference/configuration.md). The step gives them as the text of a config.
import { MODES, OPTIONS, esc, settingsYaml } from "./core.js";

/**
 * initSettingsStep({ root, onChange, checkFile })  // root: the element that gets the step; onChange: optional, a setting is new
 * It returns { values(), yaml(), set(values) }. set(values) takes the values of a saved area (core.js parseSettings); a key that is missing keeps its default. values() is { modes: [...], <key>: bool }; yaml() is the text of the config for them.
 */
export function initSettingsStep({ root, onChange = () => {}, checkFile = null }) {
	root.classList.add("bs", "bs-settings");
	const opt = (attrs, name, note = "") => `<label class="rs-opt"><input type="checkbox" ${attrs}><span><b>${esc(name)}</b>${note ? `<small>${esc(note)}</small>` : ""}</span></label>`;
	// as the card Build of Studio: the networks in a row, the base-map layers, and the other options in a folded box
	const main = OPTIONS.filter((o) => o.key === "build_features"), more = OPTIONS.filter((o) => o.key !== "build_features");
	const row = (o) => opt(`name="opt" value="${o.key}"${o.on ? " checked" : ""}`, o.name, o.note);
	root.innerHTML = `<div class="bgroup"><div class="rs-cap">Networks</div>
		<div class="rs-opts rs-opts-row" role="group" aria-label="The networks">${MODES.map((m) => opt(`name="mode" value="${m}" checked`, m[0].toUpperCase() + m.slice(1))).join("")}</div></div>
		<div class="bgroup"><div class="rs-cap">Base map</div><div class="rs-opts" role="group" aria-label="The base map">${main.map(row).join("")}</div></div>
		<details class="drow"><summary>More options</summary><div class="rs-opts" role="group" aria-label="The other options of the build">${more.map(row).join("")}</div></details>
		${checkFile ? `<details class="drow"><summary>Fixes</summary><label class="rs-cap" for="bs-fixes">Path of a fixes file (.yaml) on this computer</label>
			<input id="bs-fixes" type="text" data-ss="fixes" autocomplete="off" spellcheck="false" placeholder="/data/osm_fixes.yaml"><p class="rs-field-msg" role="status" data-ss="fixes-msg"></p></details>` : ""}
		<details class="bs-config drow"><summary>Config of the build</summary><pre data-ss="yaml"></pre></details>`;
	let fixes = "", timer = 0;
	const values = () => ({ ...(fixes ? { fixes } : {}), modes: [...root.querySelectorAll('[name="mode"]:checked')].map((i) => i.value),
		...Object.fromEntries([...root.querySelectorAll('[name="opt"]')].map((i) => [i.value, i.checked])) });
	const yaml = () => settingsYaml(values());
	const show = () => { try { root.querySelector('[data-ss="yaml"]').textContent = yaml(); } catch (e) { root.querySelector('[data-ss="yaml"]').textContent = e.message; } };
	const set = (v) => {
		for (const i of root.querySelectorAll('[name="mode"]')) i.checked = !v.modes || v.modes.includes(i.value);
		for (const i of root.querySelectorAll('[name="opt"]')) i.checked = v[i.value] ?? OPTIONS.find((o) => o.key === i.value).on;
		const f = root.querySelector('[data-ss="fixes"]');
		if (f) { f.value = v.fixes ?? ""; f.dispatchEvent(new Event("input")); }
		show(); onChange();
	};
	// the fixes file: the server checks that the file is there; the key goes into the config only then
	root.querySelector('[data-ss="fixes"]')?.addEventListener("input", (e) => {
		clearTimeout(timer);
		fixes = "";
		const msg = root.querySelector('[data-ss="fixes-msg"]'), path = e.target.value.trim();
		msg.textContent = ""; msg.classList.remove("err");
		show(); onChange();
		if (!path) return;
		timer = setTimeout(async () => {
			try { await checkFile(path); if (e.target.value.trim() === path) { fixes = path; msg.textContent = "Found."; } } catch (err) { msg.textContent = err.message; msg.classList.add("err"); }
			show(); onChange();
		}, 250);
	});
	root.addEventListener("change", () => { show(); onChange(); });
	show();
	return { values, yaml, set };
}
