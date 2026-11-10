# The Ground step

A part of the page of [duckOSM Studio](https://github.com/Khoshkhah/duckosm-studio): it suggests the DEM of an area and gives it to the build. The design is
[docs/design/ground_step.md](../../docs/design/ground_step.md). The sources are in [`src/duckosm/data/dem_sources.json`](../../src/duckosm/data/dem_sources.json).

| File | What it is |
|---|---|
| `ground_core.js` | The logic, with no DOM: `sourcesFor`, `inspectGeoTiff`, `writeGeoTiff`, `terrariumGeoTiff`. |
| `ground_step.js` | The step: `initGroundStep`. |
| `ground_step.css` | The look of the step. Every rule starts with `.gs`. |
| `ground_units.mjs` | The unit tests: `node web/ground_step/ground_units.mjs` (the CI job `test` runs them). |

```js
import { initGroundStep } from "./ground_step/ground_step.js";

const step = await initGroundStep({
	root,      // the element that gets the step
	catalog,   // dem_sources.json: the URL, or the array of its `sources`
	bbox,      // () gives [west, south, east, north] of the area, or null
	status,    // optional: status(text, ms)
	onChange,  // optional: the choice or the file is new
	loadTile,  // optional: (z, x, y) gives the RGBA bytes of an AWS tile (for tests)
});
step.refresh();           // call it when the area changes
const dem = await step.dem();   // null, or { name, bytes, product, vertical_datum, license, not_reference }; an Error says why a file cannot be used
```

Give the fields of `dem` to the build: `Params.dem_bytes = (name, bytes)`, `dem_product`, `dem_vertical_datum`, `dem_license`, `dem_not_reference`.
Link `ground_step.css`. The page needs roadstyle's parts (`rs-opts`, `rs-opt`, `rs-cap`, `rs-link`, `rs-field-msg`) and a button class `pick`.
