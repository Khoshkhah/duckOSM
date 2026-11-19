# Questions to ask the AI agent

The full list of questions for **Ask the data** on the page of `duckosm ask`. The manual page
[Questions to ask the AI agent](../guides/ask-questions.md) shows the best of them. Each question uses tables and columns
that the Monaco sample has.

## Roads of the area

Tables: `driving.edges`, `walking.edges`, `cycling.edges`. The roads of the result show on the map.

- Which one-way roads have no name?
- What are the 10 longest primary roads?
- Which roads have a speed limit above 50 km/h?
- Which roads have more than 3 lanes?
- How many edges and how many km of road does each network have?
- What are the 10 street names with the most km of road?
- What are the 10 shortest edges?
- What is the average speed limit of each road class?
- How many km of road does each class have in the walking network?
- Which roads have the longest travel time?

## Road details

Columns of `driving.edges`: `surface`, `bridge`, `tunnel`, `junction`, `access`, `service`, `layer`. An empty value means that OSM has no tag.

- How many km of road does each surface have?
- Which roads have no surface information?
- How many roads are bridges, and how many are tunnels?
- Which roads are in a roundabout?
- Which roads have a restricted access, and what is the restriction?
- What kinds of service roads are there, and how many km has each kind?
- Which roads are below the ground level (a layer below 0)?

## Junctions and turns

Tables: `driving.nodes`, `driving.turn_restrictions`, `driving.edge_graph` (one row for each legal move to the next edge). A result with the column `geom` of the nodes shows the nodes on the map.

- How many turn restrictions are there of each type?
- Show the junctions that have a turn restriction.
- Which roads start a turn restriction?
- Which 10 nodes have the most roads that start there?
- Show the nodes where four or more roads start.
- Which 10 roads have the most legal turns after them?

## Traffic signals and crossings

Table: `features.traffic`. `kind` is `traffic_signals` or `crossing`. `tags` has the OSM tags, for example `tags['crossing']`.

- How many traffic signals and how many crossings are there?
- Show the traffic signals on the map.
- Show the crossings that have traffic signals.
- How many crossings of each type are there?
- Show the crossings that have tactile paving.
- Show the crossings that have a pedestrian island.

## Buildings and walls

Tables: `features.buildings` and `main.building_faces` (`face_type`, `computed_type`, `edited`). The walls need a build with `options.building_faces: true` ([The walls of the buildings](../concepts/building-walls.md)).

- How many walls of each type do the buildings have?
- Which buildings have no front wall?
- Which buildings have more than 10 walls?
- What are the 100 longest walls?
- How many buildings are there of each kind?
- Which buildings have a name?
- What are the 20 largest buildings by area?
- What is the total area of the buildings of each kind?
- Which buildings have the most walls?

## Levels, heights and roofs

Tags of `features.buildings`: `tags['building:levels']`, `tags['height']`, `tags['roof:shape']`. Many buildings do not have these tags. A question counts only the buildings that have the tag.

- Which 10 buildings have the most levels?
- How many buildings have a height, and how many have a number of levels?
- Show the buildings that are taller than 30 m.
- What roof shapes do the buildings have, and how many has each shape?
- Color the buildings with more than 10 levels in red and the others in grey.
- What is the average number of levels of the apartment buildings?

## Walls and streets

Columns of `main.building_faces`: `length_m`, `distance_m` (to the street that the wall faces), `street_name`, `type_reason` (why the wall has its type).

- What is the average length of the walls of each type?
- Show the walls that are longer than 20 m.
- Which streets do the most front walls face?
- Show the walls that are more than 30 m from the nearest street.
- Why do the walls have their type? Count the walls for each reason.

## My changes

Table: `extra.edits`, the changes of the fixes file (`kind`, `type`, `osm_id`, `what`, `changed`, `note`). Without a fixes file, the table is empty. See [Edit OSM data](../guides/edit-osm-data.md).

- Which objects did I change, and when?
- Which walls did I change by hand?
- What did I change in the last day?
- How many changes of each kind do I have?
- Which changes have a note?
- Which turn restrictions did I add?
- Which new roads did I add?
- Show the roads that I changed on the map.
- Show the buildings that have a wall that I changed.

## Shapes on the map

The page draws a result that has a geometry column. The table shows the first 1000 rows; the map shows all rows. A wall gets the color of its `face_type`. A column `color` (a CSS color) sets the color of a shape.

- Show all buildings on the map.
- Show the walls of all buildings, with a color for each wall type.
- Show the dead-end nodes of the driving network.
- Show all nodes of the driving network on the map.
- Show only the front walls of the buildings.
- Color the walls that I changed by hand in red and the others in grey.
- Show the buildings that have a name.
- Color the buildings by their kind.
- Show the traffic signals in red and the crossings in blue.
- Show the 50 longest walls in red.
- Show the parking areas.
- Show the bus stops.

## Roads: color and focus

Ask colors roads, or keeps some roads and fades the others. It uses the road classes of the page, for example `primary` and `residential`.

- Color the roads by their class.
- Show only the primary roads and fade the others.
- Color the one-way roads red and the two-way roads grey.
- Color the roads that have a speed limit above 50 km/h.
- Color the roads by their surface.
- Color the bridges blue and the tunnels brown.
- Show only the roundabouts and fade the other roads.
- Keep the service roads and hide the others.
- Color the roads by their number of lanes.
- Color the roads longer than 100 m red.
- Zoom to the longest road.

## H3 cells and the boundary

Table: `extra.boundary_h3`, the H3 cells of the boundary at resolutions 0 to 11. The page calculates it, because DuckDB has no H3 functions here. A result with a column `h3_cell` shows as hexagons. The nodes have `h3_cell` at resolution 8.

- Show the smallest H3 cell that contains the whole boundary.
- Show the H3 cells of the boundary at resolution 9.
- How many nodes are in each H3 cell?
- Make the H3 cells more transparent.
- How many H3 cells cover the boundary at each resolution?
- Show the 10 H3 cells with the most nodes.
- Show the H3 cells that have more than 20 nodes.
- Show the H3 cells of the boundary at resolution 8 that have no node.
- Show the H3 cell with the most nodes.

## Water, 3D and base maps

Ask can show and hide the layers, change the base map, turn on the 3D view and move the camera. Layers of the page are, for example, water, buildings, parking, railways, traffic signals and bus stops.

- Turn off the water and the waterways layers.
- Switch the base map to satellite.
- Turn on the 3D view and tilt the map.
- Switch to the dark base map.
- Turn on the traffic signals and the crossings layers.
- Turn on the parking layer.
- Turn on the bus stops layer.
- Turn off the buildings layer.
- Switch to the light gray base map.
- Rotate the map to the east and tilt it by 60 degrees.

## Look of the shapes

The tool `set_shapes` changes the opacity of the shapes of a result (H3 cells, buildings, walls, nodes). A new result keeps the look. The map layers have a different tool: see *Water, 3D and base maps*.

- Show all the buildings and make them half transparent.
- Make the shapes solid.
- Make the shapes almost invisible.
- Go back to the default look of the shapes.

## Charts of the data

Ask draws a chart of the result in the data dock: bar, line, histogram, scatter or pie. A pie has a maximum of 6 slices.

- Draw a bar chart of the km of road in each class.
- Draw a histogram of the road lengths.
- Show a pie chart of the one-way and the two-way roads.
- Show a pie chart of the surfaces of the roads.
- Draw a histogram of the speed limits.
- Draw a scatter chart of the length and the travel time of the roads.
- Draw a bar chart of the number of buildings of each kind.
- Draw a histogram of the number of levels of the buildings.
- Draw a bar chart of the number of walls of each type.
- Draw a line chart of the number of H3 cells at each resolution.
- Show a pie chart of the traffic signals and the crossings.
