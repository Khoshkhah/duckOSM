// duckosm edit (docs/design/edit_page.md): the host of duckOSM's fixes editor (web/fixes_editor/) on roadstyle's standard dashboard. The page
// is roadstyle's (the server made it from the database); this script puts the editor into the dashboard's panel and gives it what the host
// gives in duckOSM Studio: the database (`duck`: the server runs each query on the database, read only), the map, js-yaml and the hooks. The
// rules go to the fixes file on disk (PUT /api/fixes) with each change. Build again (POST /api/build) builds the database again with the file,
// and the page loads again with the new network.
(() => {
  const $ = (id) => document.getElementById(id);
  const esc = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;");
  const api = async (path, init) => {
    const r = await fetch("api/" + path, init);
    if (!r.ok) throw new Error((await r.text()) || r.statusText);
    return r;
  };
  const script = (src) => new Promise((ok, no) => document.head.append(Object.assign(document.createElement("script"), { src, crossOrigin: "anonymous", onload: ok, onerror: () => no(new Error("cannot load " + src)) })));
  let tip = null;
  function status(text, ms = 4000) {
    const el = $("fix-status");
    if (!el) return;
    el.textContent = text; el.hidden = !text;
    clearTimeout(tip);
    if (ms > 0) tip = setTimeout(() => { el.hidden = true; }, ms);
  }

  async function start() {
    const [info] = await Promise.all([api("info").then((r) => r.json()), script("https://cdn.jsdelivr.net/npm/js-yaml@4.1.0/dist/js-yaml.min.js")]);
    document.head.append(Object.assign(document.createElement("link"), { rel: "stylesheet", href: "fixes_editor/fixes_panel.css" }));
    // the editor is in roadstyle's editor frame in the dashboard's panel (rs.frame): the page is the editor, so the frame has no back button. The
    // title line has the name of the fixes file, and the head row has the line of state under the tabs.
    const frame = (opts) => {
      const f = window.rs.frame({ ...opts, panel: $("sb"), back: null });
      f.top.insertAdjacentHTML("beforeend", `<span class="small muted" title="${esc(info.fixes)}">${esc(info.fixes.split(/[\\/]/).pop())}</span>`);
      f.head.insertAdjacentHTML("beforeend", '<p id="fix-status" class="small" role="status" hidden></p>');
      return f;
    };
    const { initFixesPanel } = await import("./fixes_editor/fixes_panel.js");
    const duck = {
      tables: info.tables,
      loadSpatial: async () => {},                                    // the server loaded it
      query: async (sql) => {
        const rows = await (await api("query", { method: "POST", body: sql })).json();
        return { toArray: () => rows.map((o) => ({ toJSON: () => o })) };
      },
    };
    // the rules as the file has them (the panel's text), so that only a change writes the file; `blocked`: the file is not valid YAML, so the
    // page does not write it (a write would lose it)
    let saved = null, ready = false, blocked = "", queue = Promise.resolve();
    const save = () => {
      if (!ready || blocked) return;
      const t = panel.text() ?? "";
      if (t === saved) return;
      saved = t;
      queue = queue.then(() => api("fixes", { method: "PUT", body: t })).then(() => status(`Saved in ${info.fixes.split(/[\\/]/).pop()}.`, 2500))
        .catch((e) => status("The fixes file was not saved: " + e.message, 15000));
    };
    const roadIdOfRow = (row) => window.rsGetProps?.([row])?.[0]?.osm_id ?? window.RS_SIDE?.props?.[row]?.osm_id;
    async function onBuild() {
      await queue;
      const b = $("fix-apply");
      b.disabled = true; b.classList.add("busy");
      status("Building again with the fixes file. This can take some time.", 0);
      try {
        const r = await (await api("build", { method: "POST" })).json();
        if (!r.ok) throw new Error(r.error);
        try { sessionStorage.setItem("duckosm-edit-built", r.text); } catch { /* no storage */ }
        location.reload();                                       // the server has the page of the new network
      } catch (e) {
        status("The build failed: " + e.message, 0);
        b.disabled = false; b.classList.remove("busy");
      }
    }
    const panel = await initFixesPanel({ frame, duck, map: window.map, yaml: window.jsyaml, status, roadIdOfRow, areaKey: () => "edit:" + info.db,
      onChange: save, onBuild, buildOff: () => info.build_off, changesNote: `Each change is saved in <code>${esc(info.fixes)}</code> at once.` });
    if (info.exists) {
      const text = await (await api("fixes")).text();
      try { window.jsyaml.load(text); } catch (e) { blocked = e.message.split("\n")[0]; }
      if (blocked) status(`The fixes file is not valid YAML: ${blocked}. The page does not change it. Correct it, then load the page again.`, 0);
      else panel.load(text);
    }
    saved = panel.text() ?? "";
    ready = true;
    await panel.open();
    // the template's hide button hides the panel: the editor closes too (nothing of it stays selected on the map); show opens it again
    $("sb-hide")?.addEventListener("click", () => panel.close());
    $("sb-show")?.addEventListener("click", () => panel.open());
    try {
      const built = sessionStorage.getItem("duckosm-edit-built");
      if (built) { sessionStorage.removeItem("duckosm-edit-built"); status("Built again: " + built.split("\n").slice(0, 2).join(" "), 8000); }
    } catch { /* no storage */ }
    window.fixesPanel = panel;                                   // for the tests (tests/test_edit_page.py)
  }
  const go = () => start().catch((e) => { console.error(e); document.body.insertAdjacentHTML("beforeend", `<p id="edit-error" role="alert">duckosm edit: ${esc(e.message)}</p>`); });
  window.rsReady ? go() : document.addEventListener("rs:ready", go, { once: true });
})();
