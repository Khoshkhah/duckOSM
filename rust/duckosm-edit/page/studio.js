// duckosm studio: the host of the build steps (web/build_steps/, web/ground_step/) in roadstyle's dashboard. The page is the page writer's standard
// dashboard (the server made it); this script puts each step into the panel as a card, the way duckOSM Studio does (rs.sections.add with `step`, the line
// "after step N" of a card that is not ready, a check mark for a step that is done, one open step, the "?" of each card), and gives the steps the map, the server
// (the download, the build) and roadstyle's parts. The steps are parts of duckOSM that Studio loads too. The cards are the cards of Studio, except step 4 (Ground),
// which is new, and the settings, which are in the card Build as in Studio.
(() => {
  const $ = (id) => document.getElementById(id);
  const api = async (path, init) => {
    const r = await fetch("api/" + path, init);
    if (!r.ok) throw new Error((await r.text()) || r.statusText);
    return r;
  };

  // what the "?" of each card says (the form of Studio's CARD_HELP)
  const HELP = {
    1: { title: "Choose an area", intro: "Search, draw or open a file to make the area.", terms: [
      ["Search", "Type the name of a place. The page finds its boundary."],
      ["Draw", "Press the button, then drag a box on the map."],
      ["File", "Open a GeoJSON file with the polygons of the area."],
      ["What is here?", "Click a point on the map to see the areas around it."]],
      note: 'Boundaries: <a href="https://nominatim.openstreetmap.org" target="_blank" rel="noopener">Nominatim</a>. Data © OpenStreetMap contributors.' },
    2: { title: "The area", intro: "The area that the page builds.", terms: [
      ["Name", "The name of the area. It is the name of its folder and of its database file."],
      ["Numbers", "The size of the area in km², its polygons and its vertices."],
      ["Boundary", "Saves the boundary as a GeoJSON file."],
      ["Zoom button", "Moves the map to the area."]] },
    3: { title: "The data", intro: "The OpenStreetMap data of the area.", terms: [
      ["Geofabrik", "The extracts of Geofabrik that cover the whole area, the smallest first. Download puts one into the folder of the areas; then Use it."],
      ["File", "The path of a .osm.pbf file on this computer. The build cuts it to the area."]] },
    4: { title: "Build", intro: "The settings and the build, on this computer.", terms: [
      ["Networks", "The networks to build: driving, walking and cycling."],
      ["Base map", "The layers of the base map (water, land use, buildings, ...)."],
      ["Build", "Builds the area into a .duckdb file in its folder. Then the page shows the roads and the layers of the area on its map."]] },
    5: { title: "The result", intro: "What the build made: the networks, and the files with the folder where they are saved.", terms: [] },
    6: { title: "Ground", intro: "The height of the ground (a DEM) of a built area. It is optional.", terms: [
      ["Sources", "The DEM sources that cover the area, best first. Download puts the files of a source into the folder; the product, the vertical datum and the licence are the source's."],
      ["File", "Drop the GeoTIFF of the DEM (EPSG:4326, 32-bit floats). The ground id is its SHA-256."],
      ["Add the ground", "Builds the area again with the DEM: the same OSM data and settings, and the heights of the nodes and the edges."]] },
    7: { title: "Edit OSM data", intro: "Fix the OSM data of the area: the panel Edit OSM data of duckOSM, over the map of the area.", terms: [
      ["Back", "The panel has a back button to these steps."]] },
  };

  async function start() {
    const rs = window.rs, map = window.map;
    for (const href of ["parts/ground_step/ground_step.css", "parts/build_steps/steps.css"]) document.head.append(Object.assign(document.createElement("link"), { rel: "stylesheet", href }));
    const title = document.querySelector("#sb .sb-top h2");        // the title with a line under it, as in Studio
    if (title) { const wrap = document.createElement("div"); title.replaceWith(wrap); wrap.append(title, Object.assign(document.createElement("p"), { textContent: "Make the data of an area on this computer" })); }
    // the "?" of the panel title (roadstyle's help window), as in Studio
    rs.help.set({ title: "duckOSM Studio", button: "Help: how this page works",
      intro: "This page makes the data of an area on this computer. The server of this page builds the area: the page itself shows the steps. The files stay on this computer, in the folder of the areas.",
      terms: [["1 Choose an area", "Search for a place, draw shapes on the map, open a GeoJSON file, or click hexagons."], ["2 The area", "Check the name and the size of the area, and set a buffer."],
        ["3 The data", "Select the OpenStreetMap data: a Geofabrik extract, or a file on this computer."], ["4 Ground", "Select the DEM (the height of the ground), or none."],
        ["5 Build", "Select the networks and the options, and press Build."], ["6 The result", "The networks and the files that the build made."], ["7 Visualization", "The map of the new area, and the panel Edit OSM data."]],
      note: 'Documentation: <a href="https://khoshkhah.github.io/duckOSM/guides/studio/" target="_blank" rel="noopener">Make the data of an area on your computer</a>' });
    const [{ initAreaStep }, { initDataStep }, { initSettingsStep }, { initBuildStep }, { initResultStep }, { initFolderStep }, { initVisualizationStep }, { extractsOf, parseSettings, esc, bytes }, { initGroundStep }] = await Promise.all([
      import("./parts/build_steps/area_step.js"), import("./parts/build_steps/data_step.js"), import("./parts/build_steps/settings_step.js"),
      import("./parts/build_steps/build_step.js"), import("./parts/build_steps/result_step.js"), import("./parts/build_steps/folder_step.js"), import("./parts/build_steps/visualization_step.js"), import("./parts/build_steps/core.js"), import("./parts/ground_step/ground_step.js")]);

    // ---- the cards: one for each step, as in Studio (a round number, the title, "?", a chevron) ----
    const cards = [];
    const body = () => Object.assign(document.createElement("div"), { className: "step-body" });
    const roots = {};
    for (const [n, label] of [[1, "Choose an area"], [2, "The area"], [3, "The data"], [4, "Build"], [5, "The result"], [6, "Ground"], [7, "Edit OSM data"]]) {
      const el = (roots[n] = body());
      const c = rs.sections.add({ id: `step${n}`, label, step: n, fold: true, el });
      c.classList.add("step");
      c.dataset.n = n;
      c.querySelector(".rs-sec-title").insertAdjacentHTML("afterend", '<span class="after"></span>');
      c.querySelector(":scope > .rs-sec-hd > svg")?.before(rs.help.button(c, { ...HELP[n], button: `Help: ${label}` }));
      cards.push(c);
    }
    roots[4].append(Object.assign(document.createElement("div"), { id: "settings-box", className: "step-body" }), Object.assign(document.createElement("div"), { id: "build-box", className: "step-body" }));
    const groundPart = body(), groundGo = body();
    roots[6].append(groundPart, groundGo);

    // the areas on the disk (the folder of the server): as in Studio, a card with a row for each area, with Open (the area in the steps, its result and its map) and Delete
    const saved = body();
    const areasCard = rs.sections.add({ id: "areas", label: "Your areas", fold: true, el: saved, before: "step1" });
    const folderBox = body(), listBox = body();
    saved.append(folderBox, listBox);
    initFolderStep({ root: folderBox, folder: async () => (await (await api("info")).json()).out, list: async (path) => post("folders", path),
      choose: async (path, create) => { await post("folder", JSON.stringify({ path, create })); await paintAreas(); where(); }, onChange: () => rs.sections.note("areas", "") });
    let openName = null, pageArea = null;      // pageArea: the area of this page (the server opened it), set at the end of start()
    const markOpen = (name) => { openName = name; for (const r of saved.querySelectorAll('input[name="saved-area"]')) r.checked = r.value === name; };
    // Open: the server opens the area and makes its page (the steps over the map of the area, a blank base map, or the kept page), then this page loads again
    // and shows it. b: the button that shows "Opening…"
    async function openArea(name, b = null) {
      if (b) b.innerHTML = '<span class="gs-spin" aria-hidden="true"></span> Opening…';
      rs.sections.note("areas", `Opening ${name}…`);
      await api("open", { method: "POST", body: name });
      for (let i = 0; i < 600; i++) {                                     // the page of a large area is made once (Burnaby: about 13 s), then kept
        const info = await (await api("info")).json();
        if (info.edit && info.area === name) return location.assign(`./?area=${encodeURIComponent(name)}`);
        await new Promise((r) => setTimeout(r, 300));
      }
      throw new Error(`${name} did not open: see the server's messages.`);
    }
    // the area that this page shows (the server opened it): its boundary and settings in the steps, its row selected, its result (step 5) and its map (step 7)
    async function showOpen(name, job) {
      const a = await (await api("area/" + encodeURIComponent(name))).json();
      area.set(a.boundary, a.name, "saved");
      area.filled(false);                                                  // its own map shows: only the line of the boundary
      loadedArea = a.name;
      settings.set(parseSettings(a.config));
      markOpen(a.name);
      hasDem = /^elevation:/m.test(a.config);
      built = true; builtName = a.name;
      result.show(job?.state === "done" && job.name === a.name ? job : { ...a, text: "", seconds: null });
      viz.ready(true); refreshSteps();
      autoFold(cards[4], false);
      rs.sections.note("areas", `${a.name} is open: its roads and layers are on the map. Edit it in step 7, or change a step and build again.`);
    }
    const paintAreas = async () => {
      const list = await (await api("areas")).json();
      listBox.innerHTML = list.length ? "" : '<p class="small muted">No area in this folder yet.</p>';
      listBox.insertAdjacentHTML("beforeend", list.map((a) => {
        const modes = parseSettings(a.config).modes ?? [];
        // the area that is open in the steps keeps its row selected (roadstyle's style of a chosen row: a hidden radio that is checked)
        return `<div class="rs-opt saved-area" data-name="${esc(a.name)}"><input type="radio" name="saved-area" value="${esc(a.name)}" hidden tabindex="-1"${a.name === openName ? " checked" : ""}><span><strong>${esc(a.name)}</strong>
          <small>${new Date(a.modified * 1000).toLocaleString()} · ${bytes(a.size)}${a.dem ? " · with a DEM" : ""}</small>
          <span class="sa-nets">${modes.map((m) => `<span class="rs-chip">${esc(m)}</span>`).join("")}</span>
          <span class="sa-actions"><button type="button" class="sb-primary" data-do="open" title="Open this area: its boundary and settings go into the steps, its result into step 6, and step 7 opens its map">Open</button><button type="button" class="sb-danger" data-do="delete" title="Delete the folder of this area and all its files">Delete</button></span></span></div>`;
      }).join(""));
    };
    // a saved area under the mouse (or the keyboard) shows its boundary on the map, as a place of the search list does; the boundaries are read once
    const outlines = new Map();
    const showArea = async (name) => {
      if (!outlines.has(name)) outlines.set(name, api("area/" + encodeURIComponent(name)).then((r) => r.json()).then((a) => a.boundary, () => null));
      const boundary = await outlines.get(name);
      if (hoveredArea === name && boundary) area.showPlace(boundary, name);
    };
    let hoveredArea = null;
    let loadedArea = null;                       // an area that Open chose: the hover shows nothing until the area is deselected (Unselect, or another area)
    const overArea = (e) => { if (loadedArea && area.area()?.name === loadedArea) return; loadedArea = null; const n = e.target.closest(".saved-area")?.dataset.name; if (n && n !== hoveredArea) { hoveredArea = n; showArea(n); } };
    saved.addEventListener("mouseover", overArea);
    saved.addEventListener("focusin", overArea);
    saved.addEventListener("mouseleave", () => { if (hoveredArea) area.showPlace(null); hoveredArea = null; });
    saved.addEventListener("click", async (e) => {
      let b = e.target.closest("button[data-do]");
      const row = e.target.closest(".saved-area");
      if (!b && row && !e.target.closest("button, a, input")) b = row.querySelector('[data-do="open"]');      // a click on the card is Open
      const name = b?.closest(".saved-area")?.dataset.name;
      if (!name || b.disabled) return;
      b.disabled = true;
      try {
        if (b.dataset.do === "delete") {
          if (!confirm(`Delete the area ${name} and all the files in its folder? This cannot be undone.`)) { b.disabled = false; return; }
          await api("delete", { method: "POST", body: name });
          if (name === pageArea) return location.assign("./");
          if (area.area()?.name === name) { built = false; result.clear(); viz.ready(false); }
          outlines.delete(name); hoveredArea = null; area.showPlace(null);
          await paintAreas(); refreshSteps();
          rs.sections.note("areas", `${name} is deleted.`);
          return;
        }
        // Open: the area in the steps (its boundary, settings and result) and its visualization: the map page of the area (blank base map), in this tab.
        // Its back button comes back here, and the page shows the same area again (the server keeps it)
        await openArea(name, b);
        return;
      } catch (err) {
        rs.sections.note("areas", err.message);
      }
      b.disabled = false;
    });

    // ---- the steps ----
    // the panel covers a part of the map: the area is fitted to the rest (a sheet at the bottom on a phone)
    const padding = () => {
      if (document.documentElement.classList.contains("rs-phone")) return { top: 30, left: 30, right: 30, bottom: Math.round(innerHeight * 0.5) };
      const r = $("sb").getBoundingClientRect(), left = r.left < innerWidth / 2;                // the panel is at the left (the default) or at the right (Settings)
      return { top: 40, bottom: 40, left: left ? r.right + 30 : 40, right: left ? 90 : innerWidth - r.left + 30 };      // 90: the toolbox at the other edge
    };
    const parts = { search: rs.search, drop: rs.drop, tabs: rs.tabs, toggle: rs.toggle, kpi: rs.kpi, number: rs.number };
    const post = async (path, body) => (await api(path, { method: "POST", body })).json();
    const area = initAreaStep({ root: roots[1], detail: roots[2], map, rs: parts, padding, buffer: (geometry, metres) => post("buffer", JSON.stringify({ geometry, metres })), onChange: () => { if (pageArea && !area.area()) { location.assign("./"); return; } if (openName && area.area()?.name !== openName) { markOpen(null); area.filled(true); } if (built && area.area()?.name !== builtName) { built = false; result.clear(); viz.ready(false); } ground.refresh(); data.refresh(); build.refresh(); refreshSteps(); } });      // another area: the result of the old one goes
    const zoomBtn = Object.assign(rs.sections.button("step2", { icon: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5M11 8v6M8 11h6"/></svg>', title: "Zoom to the area", onClick: () => area.zoom() }), { disabled: true });
    let extracts = null;
    const data = initDataStep({ root: roots[3], rs: parts, map, padding, zoomToArea: () => area.zoom(), area: () => area.area(),
      extracts: () => (extracts ??= api("geofabrik").then((r) => r.json()).then(extractsOf).catch((e) => { extracts = null; throw e; })),
      checkFile: async (path) => (await (await api("file", { method: "POST", body: path })).json()).size, onChange: () => { build.refresh(); refreshSteps(); where(); },
      files: { list: async () => (await api("pbf")).json(), get: (url) => api("pbfget", { method: "POST", body: url }) } });
    const demReq = (source, bbox) => JSON.stringify({ source, bbox });
    const demFiles = { list: (source, bbox) => post("demfiles", demReq(source, bbox)), get: (source, bbox) => post("demget", demReq(source, bbox)) };
    const ground = await initGroundStep({ root: groundPart, catalog: "parts/data/dem_sources.json", bbox: () => area.area()?.bbox ?? null, drop: rs.drop, files: demFiles,
      status: (t) => rs.sections.note("step6", t), onChange: () => { groundBuild?.refresh(); refreshSteps(); } });
    const settings = initSettingsStep({ root: $("settings-box"), onChange: () => build.refresh(), checkFile: async (path) => (await post("file", path)).size });
    const result = initResultStep({ root: roots[5] });
    const viz = initVisualizationStep({ root: roots[7], href: "edit/", text: "Open the editor", note: "The panel Edit OSM data of duckOSM, over the map of this area. Each change goes into the fixes file of the area (osm_fixes.yaml), and Build again builds the area with it. Its back button comes back to these steps." });
    let built = false, builtName = null, auto = false, touched = false, current = null, hasDem = false, groundBuild = null;
    const autoFold = (c, folded) => { auto = true; try { rs.sections.fold(c.dataset.rsSection, folded); } finally { auto = false; } };
    const build = initBuildStep({ root: $("build-box"),
      ready: () => {
        if (!area.area()) return "Choose the area in step 1.";
        if (!data.choice()) return "Choose the OSM data in step 3: Download an extract, or give a file.";
        try { settings.yaml(); } catch (e) { return e.message; }
        return "";
      },
      run: async () => {
        built = false; result.clear(); viz.ready(false); refreshSteps();
        const a = area.area();
        await api("build", { method: "POST", body: JSON.stringify({ name: a.name, boundary: a.geometry, data: data.choice(), settings: settings.yaml(), dem: null }) });      // the ground is step 6
      },
      poll: async () => (await api("job")).json(),
      onDone: (job) => { built = true; builtName = job.name; openName = job.name; result.show(job); refreshSteps(); location.assign(`./?area=${encodeURIComponent(job.name)}`); } });      // the page of the new area: the steps over its map

    // step 6, Ground: the DEM of a built area. "Add the ground" builds the area again from its own config (the same OSM file and settings) with the DEM.
    groundBuild = initBuildStep({ root: groundGo, label: "Add the ground",
      ready: () => (!built ? "Build the area first (step 4)." : !ground.choice().id ? "Choose the source of the ground." : !ground.choice().file && !byFallback() ? "Download the files of the source, or drop a file." : ""),
      run: async () => {
        const a = area.area(), dem = await ground.dem();
        if (!dem) throw new Error("Choose the ground: download the files of a source, or drop a file.");
        if (dem.bytes) await api(`dem/${a.name}`, { method: "PUT", body: dem.bytes });
        await api("ground", { method: "POST", body: JSON.stringify({ name: a.name,
          dem: { product: dem.product, vertical_datum: dem.vertical_datum, license: dem.license, not_reference: dem.not_reference, ...(dem.files ? { source: dem.source, files: dem.files } : {}) } }) });
      },
      poll: async () => (await api("job")).json(),
      onDone: () => location.assign(`./?area=${encodeURIComponent(area.area().name)}`) });
    const byFallback = () => ground.choice().id === "aws-terrain-tiles";

    // ---- where the files are: the folder of the area (loading, downloading and saving), shown in the cards ----
    const place = (card, first) => { const p = Object.assign(document.createElement("p"), { className: "where" }); first ? card.prepend(p) : card.append(p); return p; };
    const [where2, where3, where5] = [place(roots[2]), place(roots[3]), place(roots[4], true)];
    async function where() {
      const out = (await (await api("info")).json()).out, sep = out.includes("\\") ? "\\" : "/", a = area.area(), c = data.choice(), code = (t) => `<code>${esc(t)}</code>`;
      const dir = a ? `${out}${sep}${a.name}${sep}` : "";
      where2.innerHTML = a ? `Saved in ${code(dir)}` : "";
      where3.innerHTML = !a ? "" : c?.kind === "geofabrik" ? `In use: ${code(c.path)}` : c?.kind === "file" ? `Read from ${code(c.path)}` : `The data is saved in ${code(dir)}`;
      where5.innerHTML = a ? `The build writes ${code(`${dir}${a.name}.duckdb`)}, config.yaml, boundary.geojson and osm_fixes.yaml (your changes) in that folder.` : "";
    }
    where();

    // ---- the flow of the steps, as in Studio (refreshSteps): what each step needs ("" when ready, else the reason), when it is done, and the one open step ----
    const STEP = {
      1: { ready: () => "", done: () => !!area.area() },
      2: { ready: () => (area.area() ? "" : "after step 1"), done: () => !!area.area() },
      3: { ready: () => (area.area() ? "" : "after step 1"), done: () => !!data.choice() },
      4: { ready: () => (area.area() && data.choice() ? "" : "after step 3"), done: () => built },
      5: { ready: () => (built ? "" : "after step 4"), done: () => built },
      6: { ready: () => (built ? "" : "after step 4"), done: () => hasDem, optional: true },      // optional: an area has a DEM only when the person adds one
      7: { ready: () => (built ? "" : "after step 4"), done: () => false, optional: true },
    };
    // the person opened or folded a card: the next step does not fold the others
    document.addEventListener("rs:sectionchange", (e) => {
      if (auto || !(e.detail?.state === "fold" || e.detail?.state === "unfold") || !cards.some((c) => c.dataset.rsSection === e.detail.id)) return;
      const c = cards.find((x) => x.dataset.rsSection === e.detail.id);
      if (c.classList.contains("off") && e.detail.state === "unfold") return autoFold(c, true);     // "Expand all" does not open a card that is not ready
      touched = true;
    });
    function refreshSteps() {
      for (const c of cards) {
        const s = STEP[c.dataset.n], why = s.ready(), done = s.done();
        c.classList.toggle("off", !!why);
        c.querySelector(".rs-sec-title").disabled = !!why;
        c.querySelector(".after").textContent = why;
        rs.sections.done(c.dataset.rsSection, !!done && !why);
        if (why && !c.classList.contains("folded")) autoFold(c, true);      // a card that is not ready keeps only its title
      }
      groundBuild?.refresh();
      zoomBtn.disabled = !area.area();
      const next = cards.find((c) => !STEP[c.dataset.n].optional && !STEP[c.dataset.n].ready() && !STEP[c.dataset.n].done());
      if (next && next !== current) {
        current = next;
        autoFold(next, false);
        // the area card (the name) stays open beside the data step; the data and the ground stay open beside the build
        // card 1 stays open when the area is hexagons: the person clicks more cells there (as in Studio)
        const keep = (c) => (next.dataset.n === "3" && c.dataset.n === "2") || (next.dataset.n === "4" && c.dataset.n === "3") || (c.dataset.n === "1" && area.area()?.from === "h3");
        if (!touched) for (const c of cards) if (c !== next) autoFold(c, !keep(c));
      }
    }
    refreshSteps();
    // the page of an open area (after Open, a build, or the back button of the editor): the area comes into the steps
    // the area of this page: the address names it (/?area=<name>) and the server has it open; a new visit (/) has no area
    const want = new URLSearchParams(location.search).get("area"), info0 = await (await api("info")).json();
    pageArea = want && info0.area === want && info0.edit ? want : null;
    if (want && !pageArea) history.replaceState(null, "", "./");
    paintAreas().then(async () => { if (pageArea) await showOpen(pageArea, await (await api("job")).json()); }).catch((e) => rs.sections.note("areas", e.message));
    window.studio = { area, data, settings, ground, build, result, viz, map };           // for the tests
    window.studioReady = true;
  }
  const go = () => start().catch((e) => { console.error(e); document.body.insertAdjacentHTML("beforeend", `<p id="edit-error" role="alert">duckosm studio: ${String(e.message).replace(/</g, "&lt;")}</p>`); });
  window.rsReady ? go() : document.addEventListener("rs:ready", go, { once: true });
})();
