// The Settings step: the build config. The networks and the options of OPTIONS (core.js), as the keys of the config of `duckosm build`
// (docs/reference/configuration.md). The step gives them as the text of a config.
import { MODES, OPTIONS, esc, settingsYaml } from "./core.js";

/**
 * initSettingsStep({ root, onChange })  // root: the element that gets the step; onChange: optional, a setting is new
 * It returns { values(), yaml(), set(values) }. set(values) takes the values of a saved area (core.js parseSettings); a key that is missing keeps its default. values() is { modes: [...], <key>: bool }; yaml() is the text of the config for them.
 */
export function initSettingsStep({ root, onChange = () => {} }) {
	root.classList.add("bs", "bs-settings");
	const opt = (attrs, name, note = "") => `<label class="rs-opt"><input type="checkbox" ${attrs}><span><b>${esc(name)}</b>${note ? `<small>${esc(note)}</small>` : ""}</span></label>`;
	root.innerHTML = `<div class="rs-cap">Networks</div>
		<div class="rs-opts rs-opts-row" role="group" aria-label="The networks">${MODES.map((m) => opt(`name="mode" value="${m}" checked`, m[0].toUpperCase() + m.slice(1))).join("")}</div>
		<div class="rs-cap">Options</div>
		<div class="rs-opts" role="group" aria-label="The options of the build">${OPTIONS.map((o) => opt(`name="opt" value="${o.key}"${o.on ? " checked" : ""}`, o.name, o.note)).join("")}</div>
		<details class="bs-config"><summary>Config of the build</summary><pre data-ss="yaml"></pre></details>`;
	const values = () => ({ modes: [...root.querySelectorAll('[name="mode"]:checked')].map((i) => i.value),
		...Object.fromEntries([...root.querySelectorAll('[name="opt"]')].map((i) => [i.value, i.checked])) });
	const yaml = () => settingsYaml(values());
	const show = () => { try { root.querySelector('[data-ss="yaml"]').textContent = yaml(); } catch (e) { root.querySelector('[data-ss="yaml"]').textContent = e.message; } };
	const set = (v) => {
		for (const i of root.querySelectorAll('[name="mode"]')) i.checked = !v.modes || v.modes.includes(i.value);
		for (const i of root.querySelectorAll('[name="opt"]')) i.checked = v[i.value] ?? OPTIONS.find((o) => o.key === i.value).on;
		show(); onChange();
	};
	root.addEventListener("change", () => { show(); onChange(); });
	show();
	return { values, yaml, set };
}
