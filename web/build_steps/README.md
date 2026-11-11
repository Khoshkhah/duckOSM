# The build steps

Parts of the page of [`duckosm studio`](../../docs/guides/studio.md). duckOSM Studio (the browser app) can load the same parts. The steps are in the order of
the page: Area, OSM data, Settings, [Ground](../ground_step/README.md), Build and Result.

| File | What it is |
|---|---|
| `core.js` | The logic, with no DOM: the polygons and the area of GeoJSON, `extractsOf` and `covering` (the Geofabrik extracts that cover an area), `settingsYaml`, `slug`, `progressText`. |
| `area_step.js` | `initAreaStep`: a place name, a box on the map, or a GeoJSON file. |
| `data_step.js` | `initDataStep`: a Geofabrik extract that covers the area, or the path of a `.osm.pbf` file on this computer. |
| `settings_step.js` | `initSettingsStep`: the networks and the options, as the text of a config. |
| `build_step.js` | `initBuildStep`: the Build button and the progress of the build. |
| `result_step.js` | `initResultStep`: what the build made, and the link to the map of the new area. |
| `steps.css` | The look of the steps. Every rule starts with `.bs`. |
| `steps_units.mjs` | The unit tests: `node web/build_steps/steps_units.mjs` (the CI job `test` runs them). |

Each step is `initXStep({ root, ... })`. The host gives the root element and what the step cannot know: the map, the server, roadstyle's parts. The
options of each function are in the comment above it. The steps use roadstyle's parts as CSS classes (`rs-tabs`, `rs-tab`, `rs-opts`, `rs-opt`, `rs-cap`,
`rs-link`, `rs-field-msg`, `rs-status`), and `initAreaStep` calls `rs.search` and `rs.drop` (given as `rs: { search, drop }`).

```js
import { initAreaStep } from "./build_steps/area_step.js";
const area = initAreaStep({ root, map, rs: { search: rsSearch, drop: rsDrop }, onChange });
area.area();     // null, or { name, geometry, polygons, bbox, km2, from }
```

The host of `duckosm studio` is `rust/duckosm-edit/page/studio.js`: it shows how the steps join (the area gives its box to the Ground step and its polygons to the
OSM data step, and the Build step sends it all to the server). Link `steps.css` and `../ground_step/ground_step.css`.
