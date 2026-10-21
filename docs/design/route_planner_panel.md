# Design: a new side panel for the route planner

Status: built (2026-10-20). Mockup: `reports/planner_panel_mockup.png`. The dots of the start and the end follow the colours of the two pins on the map, not green and red, because green and red are the colours of the walk and drive legs.

## Problem

The panel of `duckosm route-map` (`src/duckosm/templates/planner.html`) looks like a plain form.
The mode is a drop-down list, the weights are radio buttons, and the radius and speed are number
boxes in the first screen. The result is a long block of text. The start and the end have no
visible place in the panel. The page has no dark theme and no phone layout.

## Change

Only the markup and the CSS of the panel change. The routing code does not change.

1. **Header.** The title, and one line with the area name and the hint.
2. **Mode.** A segmented control (Walk, Cycle, Drive, and the combined modes) in place of the
   drop-down list. The `<select id="rm-mode">` stays in the page, hidden, so tests and scripts keep
   working.
3. **Weight.** A second segmented control: Fastest, Shortest.
4. **Start and end.** A card with two rows, each with a coloured dot, the label and the name of the
   nearest road. A swap button exchanges the two points.
5. **Summary.** One card: the total time in large type and the distance next to it. Under it, a bar
   that shows the legs in proportion to their time, in the colour of each mode, and a legend. The
   number of changes of mode is a small chip.
6. **Steps.** A timeline: a round icon in the colour of the leg, the text of the step, the distance
   under it. A click flies the map to the step, as now. The text of each step stays equal to
   `duckosm.directions()`.
7. **Details.** The radius, the walking speed, the list of `edge_id` and the clicked road go into a
   closed `<details>` block at the end.
8. **Themes and size.** CSS variables for colour. The panel is light by default. A button in the header switches to a dark theme, and the page keeps the choice in `localStorage`. Under
   700 px the panel becomes a sheet at the bottom of the page, and the map keeps the full width.

## Not in scope

- The Dijkstra code, the data (`RM`) and `route_map.py`.
- The ids that tests use: `rm-mode`, `rm-result`, `rm-radius`, `rm-speed`, `window.rmSteps`.
- Any library. The panel stays plain HTML, CSS and JavaScript.

## Check

- `tests/test_maps.py` passes with no change.
- A screenshot of the page in Chromium at 1300 px and at 390 px width, light and dark.
- The docs maps are drawn again, so the live page shows the new panel.
