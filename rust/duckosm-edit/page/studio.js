// duckosm studio: the host of the build steps (web/build_steps/, web/ground_step/) in roadstyle's dashboard. The page is the page writer's standard
// dashboard (the server made it); this script puts each step into the panel as a card, as duckOSM Studio does (rs.sections.add with `step`), and gives the
// steps the map, the server (the download, the build) and roadstyle's parts. The steps are parts of duckOSM that Studio loads too.
(() => {
  const $ = (id) => document.getElementById(id);
  const api = async (path, init) => {
    const r = await fetch("api/" + path, init);
    if (!r.ok) throw new Error((await r.text()) || r.statusText);
    return r;
  };

  async function start() {
    const rs = window.rs, map = window.map;
    for (const href of ["parts/ground_step/ground_step.css", "parts/build_steps/steps.css"]) document.head.append(Object.assign(document.createElement("link"), { rel: "stylesheet", href }));
    // the page has no roads, so roadstyle's first view is a point at 0, 0: start with Europe, as Studio does
    const home = () => map.jumpTo({ center: [10, 50], zoom: 3, pitch: 0, bearing: 0 });
    map.loaded() ? home() : map.once("load", home);
    const [{ initAreaStep }, { initDataStep }, { initSettingsStep }, { initBuildStep }, { initResultStep }, { extractsOf }, { initGroundStep }] = await Promise.all([
      import("./parts/build_steps/area_step.js"), import("./parts/build_steps/data_step.js"), import("./parts/build_steps/settings_step.js"),
      import("./parts/build_steps/build_step.js"), import("./parts/build_steps/result_step.js"), import("./parts/build_steps/core.js"), import("./parts/ground_step/ground_step.js")]);
    // one card for each step: roadstyle's section with a round number, a title and a chevron
    const card = (n, label) => {
      const el = Object.assign(document.createElement("div"), { className: "step-body", id: `step-${n}` });
      rs.sections.add({ id: `step${n}`, label, step: n, fold: true, el });
      return el;
    };
    const roots = Object.fromEntries([[1, "Area"], [2, "OSM data"], [3, "Settings"], [4, "Ground"], [5, "Build"], [6, "Result"]].map(([n, l]) => [n, card(n, l)]));
    // the panel covers a part of the map: the area is fitted to the rest (a sheet at the bottom on a phone)
    const padding = () => {
      if (document.documentElement.classList.contains("rs-phone")) return { top: 30, left: 30, right: 30, bottom: Math.round(innerHeight * 0.5) };
      const r = $("sb").getBoundingClientRect(), left = r.left < innerWidth / 2;                // the panel is at the left (the default) or at the right (Settings)
      return { top: 40, bottom: 40, left: left ? r.right + 30 : 40, right: left ? 90 : innerWidth - r.left + 30 };      // 90: the toolbox at the other edge
    };
    // the page has no roads yet: roadstyle's check for a map with no road features is not for it
    new MutationObserver(() => document.getElementById("rs-diag")?.remove()).observe(document.body, { childList: true });
    const settings = initSettingsStep({ root: roots[3], onChange: () => build.refresh() });
    const area = initAreaStep({ root: roots[1], map, rs: { search: rs.search, drop: rs.drop }, padding, onChange: () => { notes(); ground.refresh(); data.refresh(); build.refresh(); } });
    let extracts = null;
    const data = initDataStep({ root: roots[2], area: () => area.area(),
      extracts: () => (extracts ??= api("geofabrik").then((r) => r.json()).then(extractsOf).catch((e) => { extracts = null; throw e; })),
      checkFile: async (path) => (await (await api("file", { method: "POST", body: path })).json()).size, onChange: () => build.refresh() });
    const ground = await initGroundStep({ root: roots[4], catalog: "parts/data/dem_sources.json", bbox: () => area.area()?.bbox ?? null, drop: rs.drop,
      status: (t) => rs.sections.note("step5", t), onChange: () => build.refresh() });
    const result = initResultStep({ root: roots[6], href: "edit/" });
    const build = initBuildStep({ root: roots[5],
      ready: () => {
        if (!area.area()) return "Choose the area in step 1.";
        if (!data.choice()) return "Choose the OSM data in step 2.";
        try { settings.yaml(); } catch (e) { return e.message; }
        return "";
      },
      run: async () => {
        result.clear(); rs.sections.done("step6", false);
        const a = area.area(), dem = await ground.dem();
        if (dem) await api(`dem/${a.name}`, { method: "PUT", body: dem.bytes });
        await api("build", { method: "POST", body: JSON.stringify({ name: a.name, boundary: a.geometry, data: data.choice(), settings: settings.yaml(),
          dem: dem && { product: dem.product, vertical_datum: dem.vertical_datum, license: dem.license, not_reference: dem.not_reference } }) });
      },
      poll: async () => (await api("job")).json(),
      onDone: (job) => { result.show(job); rs.sections.done("step5", true); rs.sections.done("step6", true); roots[6].scrollIntoView({ block: "nearest" }); } });
    // as in Studio: a card that waits for the area says so under its title
    const notes = () => { for (const id of ["step2", "step4"]) rs.sections.note(id, area.area() ? "" : "after step 1"); };
    notes();
    window.studio = { area, data, settings, ground, build, result, map };           // for the tests
    window.studioReady = true;
  }
  const go = () => start().catch((e) => { console.error(e); document.body.insertAdjacentHTML("beforeend", `<p id="edit-error" role="alert">duckosm studio: ${String(e.message).replace(/</g, "&lt;")}</p>`); });
  window.rsReady ? go() : document.addEventListener("rs:ready", go, { once: true });
})();
