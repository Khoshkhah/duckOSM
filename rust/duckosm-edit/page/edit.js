// duckosm edit (docs/design/edit_page.md): the host of duckOSM's fixes editor (web/fixes_editor/) on roadstyle's standard dashboard. The page
// is roadstyle's (the server made it from the database); this script puts the editor into the dashboard's panel (in roadstyle's editor frame)
// and gives it what the host gives in duckOSM Studio: the database (`duck`: the server runs each query on the database, read only), the map,
// the dashboard, js-yaml and the hooks. The rules go to the fixes file on disk (PUT /api/fixes) with each change. Build again (POST /api/build)
// builds the database again with the file, and the page loads again with the new network. The back button of the editor shows the
// dashboard's own panel; its card "Edit OSM data" opens the editor again.
(() => {
  const $ = (id) => document.getElementById(id);
  const esc = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;");
  const api = async (path, init) => {
    const r = await fetch("api/" + path, init);
    if (!r.ok) throw new Error((await r.text()) || r.statusText);
    return r;
  };
  const script = (src) => new Promise((ok, no) => document.head.append(Object.assign(document.createElement("script"), { src, crossOrigin: "anonymous", onload: ok, onerror: () => no(new Error("cannot load " + src)) })));
  // the status line: in the editor's bar, over the buttons of the tab (the element is kept: the frame is out of the page while it is closed)
  const el = Object.assign(document.createElement("p"), { id: "fix-status", className: "small", hidden: true });
  el.setAttribute("role", "status");
  let tip = null;
  function status(text, ms = 4000) {
    el.textContent = text; el.hidden = !text;
    clearTimeout(tip);
    if (ms > 0) tip = setTimeout(() => { el.hidden = true; }, ms);
  }

  async function start() {
    const [info] = await Promise.all([api("info").then((r) => r.json()), script("https://cdn.jsdelivr.net/npm/js-yaml@4.1.0/dist/js-yaml.min.js")]);
    document.head.append(Object.assign(document.createElement("link"), { rel: "stylesheet", href: "fixes_editor/fixes_panel.css" }));
    const { initFixesPanel } = await import("./fixes_editor/fixes_panel.js");
    const duck = {
      tables: info.tables,
      loadSpatial: async () => {},                                    // the server loaded it
      query: async (sql) => {
        const rows = await (await api("query", { method: "POST", body: sql })).json();
        // a JSON column (ST_AsGeoJSON) is an object in the server's rows; DuckDB-Wasm's Arrow table gives it as text, and the editor parses the text
        for (const o of rows) for (const k in o) if (o[k] !== null && typeof o[k] === "object" && !Array.isArray(o[k])) o[k] = JSON.stringify(o[k]);
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
    // the rows of the map's roads of an OSM way (the highlight sets of the editor): an index of the road table, made once; rsQuery when the page has no RS_SIDE
    let byWay = null;
    const roadRows = (id) => {
      const props = window.RS_SIDE?.props;
      if (!byWay && props?.length) { byWay = new Map(); props.forEach((p, i) => { const k = Number(p?.osm_id); byWay.has(k) ? byWay.get(k).push(i) : byWay.set(k, [i]); }); }
      return byWay ? byWay.get(Number(id)) ?? [] : window.rsQuery?.((p) => Number(p.osm_id) === Number(id)) ?? [];
    };
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
    const file = info.fixes.split(/[\\/]/).pop();
    // the card of the dashboard's panel that opens the editor (after the back button); its note is the number of changes
    window.rs.sections.add({ id: "fixes", label: "Edit OSM data", note: file, open: () => panel.open(), before: window.rs.sections.list()[0] });
    const onChange = (summary, n) => { window.rs.sections.note("fixes", n ? summary : `No changes in ${file}.`); save(); };
    const panel = await initFixesPanel($("sb"), { duck, map: window.map, dashboard: window.rs, yaml: window.jsyaml, status, roadIdOfRow, roadRows,
      areaKey: () => "edit:" + info.db, onChange, onBuild, buildOff: () => info.build_off, backLabel: "Map",
      changesNote: `Each change is saved in <code>${esc(info.fixes)}</code> at once.` });
    // the frame's title line has the name of the fixes file, and the bar has the status line
    panel.frame.top.insertAdjacentHTML("beforeend", `<span class="fix-file small muted" title="${esc(info.fixes)}">${esc(file)}</span>`);
    panel.frame.bar.prepend(el);
    if (info.exists) {
      const text = await (await api("fixes")).text();
      try { window.jsyaml.load(text); } catch (e) { blocked = e.message.split("\n")[0]; }
      if (blocked) status(`The fixes file is not valid YAML: ${blocked}. The page does not change it. Correct it, then load the page again.`, 0);
      else panel.load(text);
    }
    saved = panel.text() ?? "";
    ready = true;
    await panel.open();
    // the template's hide button hides the panel: the editor closes too (nothing of it stays selected on the map); show opens it again if it was open
    let wasOpen = false;
    $("sb-hide")?.addEventListener("click", () => { wasOpen = panel.isOpen(); panel.close(); });
    $("sb-show")?.addEventListener("click", () => { if (wasOpen) panel.open(); });
    try {
      const built = sessionStorage.getItem("duckosm-edit-built");
      if (built) { sessionStorage.removeItem("duckosm-edit-built"); status("Built again: " + built.split("\n").slice(0, 2).join(" "), 8000); }
    } catch { /* no storage */ }
    window.fixesPanel = panel;                                   // for the tests (tests/test_edit_page.py)
  }
  const go = () => start().catch((e) => { console.error(e); document.body.insertAdjacentHTML("beforeend", `<p id="edit-error" role="alert">duckosm edit: ${esc(e.message)}</p>`); });
  window.rsReady ? go() : document.addEventListener("rs:ready", go, { once: true });
})();
