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
      ["Geofabrik", "The extracts of Geofabrik that cover the whole area, the smallest first. The build downloads the one that you pick into the folder of the area."],
      ["File", "The path of a .osm.pbf file on this computer. The build cuts it to the area."]] },
    4: { title: "Ground", intro: "The height of the ground (a DEM). It is optional.", terms: [
      ["Sources", "The DEM sources that cover the area, best first. Choose the source of your file: the product, the vertical datum and the licence are the source's."],
      ["File", "Drop the GeoTIFF of the DEM (EPSG:4326, 32-bit floats). The ground id is its SHA-256."],
      ["AWS Terrain Tiles", "A DEM that needs no file. It is a surface model and never the reference ground of the area."]] },
    5: { title: "Build", intro: "The settings and the build, on this computer.", terms: [
      ["Networks", "The networks to build: driving, walking and cycling."],
      ["Base map", "The layers of the base map (water, land use, buildings, ...)."],
      ["Build", "Builds the area into a .duckdb file in its folder. The steps show while it runs."]] },
    6: { title: "The result", intro: "What the build made: the networks, and the files with the folder where they are saved.", terms: [] },
    7: { title: "Visualization", intro: "The roads of the new area on the map, with the panel Edit OSM data (the page of `duckosm edit`).", terms: [
      ["Open the map when the build ends", "The page opens the map by itself, after the build. Turn it off to stay here."],
      ["Back", "The panel Edit OSM data has a back button to these steps."]] },
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
    for (const [n, label] of [[1, "Choose an area"], [2, "The area"], [3, "The data"], [4, "Ground"], [5, "Build"], [6, "The result"], [7, "Visualization"]]) {
      const el = (roots[n] = body());
      const c = rs.sections.add({ id: `step${n}`, label, step: n, fold: true, el });
      c.classList.add("step");
      c.dataset.n = n;
      c.querySelector(".rs-sec-title").insertAdjacentHTML("afterend", '<span class="after"></span>');
      c.querySelector(":scope > .rs-sec-hd > svg")?.before(rs.help.button(c, { ...HELP[n], button: `Help: ${label}` }));
      cards.push(c);
    }
    roots[5].append(Object.assign(document.createElement("div"), { id: "settings-box", className: "step-body" }), Object.assign(document.createElement("div"), { id: "build-box", className: "step-body" }));

    // the areas on the disk (the folder of the server): as in Studio, a card with a row for each area, with Open (the area in the steps, its result and its map) and Delete
    const saved = body();
    const areasCard = rs.sections.add({ id: "areas", label: "Your areas", fold: true, el: saved, before: "step1" });
    const folderBox = body(), listBox = body();
    saved.append(folderBox, listBox);
    initFolderStep({ root: folderBox, folder: async () => (await (await api("info")).json()).out, list: async (path) => post("folders", path),
      choose: async (path, create) => { await post("folder", JSON.stringify({ path, create })); await paintAreas(); where(); }, onChange: () => rs.sections.note("areas", "") });
    const paintAreas = async () => {
      const list = await (await api("areas")).json();
      listBox.innerHTML = list.length ? "" : '<p class="small muted">No area in this folder yet.</p>';
      listBox.insertAdjacentHTML("beforeend", list.map((a) => {
        const modes = parseSettings(a.config).modes ?? [];
        return `<div class="rs-opt saved-area" data-name="${esc(a.name)}"><span><strong>${esc(a.name)}</strong>
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
    let loadedArea = null;                       // an area that Load chose: the hover shows nothing until the area is deselected (Unselect, or another area)
    const overArea = (e) => { if (loadedArea && area.area()?.name === loadedArea) return; loadedArea = null; const n = e.target.closest(".saved-area")?.dataset.name; if (n && n !== hoveredArea) { hoveredArea = n; showArea(n); } };
    saved.addEventListener("mouseover", overArea);
    saved.addEventListener("focusin", overArea);
    saved.addEventListener("mouseleave", () => { if (hoveredArea) area.showPlace(null); hoveredArea = null; });
    saved.addEventListener("click", async (e) => {
      const b = e.target.closest("button[data-do]"), name = b?.closest(".saved-area")?.dataset.name;
      if (!name) return;
      b.disabled = true;
      try {
        if (b.dataset.do === "delete") {
          if (!confirm(`Delete the area ${name} and all the files in its folder? This cannot be undone.`)) { b.disabled = false; return; }
          await api("delete", { method: "POST", body: name });
          if (area.area()?.name === name) { built = false; result.clear(); viz.ready(false); }
          outlines.delete(name); hoveredArea = null; area.showPlace(null);
          await paintAreas(); refreshSteps();
          rs.sections.note("areas", `${name} is deleted.`);
          return;
        }
        // Open: the area in the steps (boundary, settings), its result in step 6 and its map in step 7, as after a build; from there the person can change a step and build again
        const label = b.textContent;
        b.innerHTML = '<span class="gs-spin" aria-hidden="true"></span> Opening…';
        rs.sections.note("areas", `Opening ${name}: the server opens its database. A large area takes some seconds.`);
        try { await api("open", { method: "POST", body: name }); } finally { b.textContent = label; }
        const a = await (await api("area/" + encodeURIComponent(name))).json();
        hoveredArea = null; area.showPlace(null);
        area.set(a.boundary, a.name, "saved");
        loadedArea = a.name;
        settings.set(parseSettings(a.config));
        built = true; builtName = a.name; result.show({ ...a, text: "", seconds: null }); viz.ready(true); refreshSteps();
        autoFold(cards[5], false); autoFold(cards[6], false);
        rs.sections.note("areas", `${a.name} is open. Open its map in step 7, or change a step and build again.`);
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
    const area = initAreaStep({ root: roots[1], detail: roots[2], map, rs: parts, padding, buffer: (geometry, metres) => post("buffer", JSON.stringify({ geometry, metres })), onChange: () => { if (built && area.area()?.name !== builtName) { built = false; result.clear(); viz.ready(false); } ground.refresh(); data.refresh(); build.refresh(); refreshSteps(); } });      // another area: the result of the old one goes
    const zoomBtn = Object.assign(rs.sections.button("step2", { icon: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5M11 8v6M8 11h6"/></svg>', title: "Zoom to the area", onClick: () => area.zoom() }), { disabled: true });
    let extracts = null;
    const data = initDataStep({ root: roots[3], rs: parts, map, padding, zoomToArea: () => area.zoom(), area: () => area.area(),
      extracts: () => (extracts ??= api("geofabrik").then((r) => r.json()).then(extractsOf).catch((e) => { extracts = null; throw e; })),
      checkFile: async (path) => (await (await api("file", { method: "POST", body: path })).json()).size, onChange: () => { build.refresh(); refreshSteps(); where(); },
      files: { list: async () => (await api("pbf")).json(), get: (url) => api("pbfget", { method: "POST", body: url }) } });
    const demReq = (source, bbox) => JSON.stringify({ source, bbox });
    const demFiles = { list: (source, bbox) => post("demfiles", demReq(source, bbox)), get: (source, bbox) => post("demget", demReq(source, bbox)) };
    const ground = await initGroundStep({ root: roots[4], catalog: "parts/data/dem_sources.json", bbox: () => area.area()?.bbox ?? null, drop: rs.drop, files: demFiles,
      status: (t) => rs.sections.note("step4", t), onChange: () => { build.refresh(); refreshSteps(); where(); } });
    const settings = initSettingsStep({ root: $("settings-box"), onChange: () => build.refresh(), checkFile: async (path) => (await post("file", path)).size });
    const result = initResultStep({ root: roots[6] });
    const viz = initVisualizationStep({ root: roots[7], href: "edit/" });
    let built = false, builtName = null, auto = false, touched = false, current = null;
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
        const a = area.area(), dem = await ground.dem();
        if (dem?.bytes) await api(`dem/${a.name}`, { method: "PUT", body: dem.bytes });      // a dropped file; the files of the folder stay on the disk
        await api("build", { method: "POST", body: JSON.stringify({ name: a.name, boundary: a.geometry, data: data.choice(), settings: settings.yaml(),
          dem: dem && { product: dem.product, vertical_datum: dem.vertical_datum, license: dem.license, not_reference: dem.not_reference, ...(dem.files ? { source: dem.source, files: dem.files } : {}) } }) });
      },
      poll: async () => (await api("job")).json(),
      onDone: (job) => { built = true; builtName = job.name; paintAreas(); result.show(job); viz.built(); refreshSteps(); autoFold(cards[5], false); autoFold(cards[6], false); } });

    // ---- where the files are: the folder of the area (loading, downloading and saving), shown in the cards ----
    const place = (card, first) => { const p = Object.assign(document.createElement("p"), { className: "where" }); first ? card.prepend(p) : card.append(p); return p; };
    const [where2, where3, where5] = [place(roots[2]), place(roots[3]), place(roots[5], true)];
    async function where() {
      const out = (await (await api("info")).json()).out, sep = out.includes("\\") ? "\\" : "/", a = area.area(), c = data.choice(), code = (t) => `<code>${esc(t)}</code>`;
      const dir = a ? `${out}${sep}${a.name}${sep}` : "";
      where2.innerHTML = a ? `Saved in ${code(dir)}` : "";
      where3.innerHTML = !a ? "" : c?.kind === "geofabrik" ? `In use: ${code(c.path)}` : c?.kind === "file" ? `Read from ${code(c.path)}` : `The data is saved in ${code(dir)}`;
      const dem = ground.choice().id ? ", dem.tif" : "";
      where5.innerHTML = a ? `The build writes ${code(`${dir}${a.name}.duckdb`)}, config.yaml, boundary.geojson${dem} and osm_fixes.yaml (your changes) in that folder.` : "";
    }
    where();

    // ---- the flow of the steps, as in Studio (refreshSteps): what each step needs ("" when ready, else the reason), when it is done, and the one open step ----
    const STEP = {
      1: { ready: () => "", done: () => !!area.area() },
      2: { ready: () => (area.area() ? "" : "after step 1"), done: () => !!area.area() },
      3: { ready: () => (area.area() ? "" : "after step 1"), done: () => !!data.choice() },
      4: { ready: () => (area.area() ? "" : "after step 1"), done: () => !!ground.choice().id, optional: true },      // optional: the page does not open it by itself
      5: { ready: () => (area.area() && data.choice() ? "" : "after step 3"), done: () => built },
      6: { ready: () => (built ? "" : "after step 5"), done: () => built },
      7: { ready: () => (built ? "" : "after step 5"), done: () => false, optional: true },      // optional: the page opens the map by itself, or the person does
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
      zoomBtn.disabled = !area.area();
      const next = cards.find((c) => !STEP[c.dataset.n].optional && !STEP[c.dataset.n].ready() && !STEP[c.dataset.n].done());
      if (next && next !== current) {
        current = next;
        autoFold(next, false);
        // the area card (the name) stays open beside the data step; the data and the ground stay open beside the build
        // card 1 stays open when the area is hexagons: the person clicks more cells there (as in Studio)
        const keep = (c) => (next.dataset.n === "3" && c.dataset.n === "2") || (next.dataset.n === "5" && ["3", "4"].includes(c.dataset.n)) || (c.dataset.n === "1" && area.area()?.from === "h3");
        if (!touched) for (const c of cards) if (c !== next) autoFold(c, !keep(c));
      }
    }
    refreshSteps();
    paintAreas().catch(() => {});
    window.studio = { area, data, settings, ground, build, result, viz, map };           // for the tests
    window.studioReady = true;
  }
  const go = () => start().catch((e) => { console.error(e); document.body.insertAdjacentHTML("beforeend", `<p id="edit-error" role="alert">duckosm studio: ${String(e.message).replace(/</g, "&lt;")}</p>`); });
  window.rsReady ? go() : document.addEventListener("rs:ready", go, { once: true });
})();
