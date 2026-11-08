# Questions to ask the AI agent

These are questions that you can ask in **Ask the data** on the page of [`duckosm ask`](query.md#ask-questions-on-a-page-duckosm-ask). They are in groups. For each question, the tables, the columns and the map actions that it needs exist in the Monaco sample, and the SQL that answers each question was run there and gave rows. Ask itself needs an AI key, so the questions were not sent to it. The list is one file, `web/ask/questions.js`: the page imports it, and another page can copy it.

In the list of examples, **six general groups** come first: *Know the data*, *Find and count*, *Show on the map*, *Colour and focus*, *The map and the layers* and *Charts*. They come from roadstyle's Ask, and they work with any data: they name no table and no column, so Ask reads the schema and picks the names. After them come the groups of this page, which are about the data of duckOSM and are listed below.

Write a question in your own words. You do not need these exact sentences. A short, plain question with one thing to do gives the best answer.

## How to ask

- **Say what you want to see.** Use "show", "color", "count" or "draw". Ask then uses the right tool: a table, a color, a count or a chart.
- **Use the names that the data has.** For example the road classes `primary` and `residential`, the wall types `front`, `back`, `side`, `party` and `court`.
- **Say "all" if you want all.** The table shows only the first 1000 rows, but the map shows every row that the query selects.
- **Click New to start again.** Each step sends the whole conversation again, so a long conversation uses your free limit faster.
- **Ask cannot change the data.** It reads the database and controls the map. To change the OSM data, use [Edit OSM data](edit-osm-data.md).

## Roads of the area

Questions about the roads of each network: the tables `driving.edges`, `walking.edges` and `cycling.edges`. The answer is a table. The roads of the table light on the map.

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

Questions about the tags of the roads: the columns `surface`, `bridge`, `tunnel`, `junction`, `access`, `service` and `layer` of `driving.edges`. An empty value means that OSM has no tag.

- How many km of road does each surface have?
- Which roads have no surface information?
- How many roads are bridges, and how many are tunnels?
- Which roads are in a roundabout?
- Which roads have a restricted access, and what is the restriction?
- What kinds of service roads are there, and how many km has each kind?
- Which roads are below the ground level (a layer below 0)?

## Junctions and turns

Questions about the nodes and the turns of the driving network: `driving.nodes`, `driving.turn_restrictions` and `driving.edge_graph` (one row for each legal move from an edge to the next edge). A result with the column `geom` of the nodes draws the nodes on the map.

- How many turn restrictions are there of each type?
- Show the junctions that have a turn restriction.
- Which roads start a turn restriction?
- Which 10 nodes have the most roads that start there?
- Show the nodes where four or more roads start.
- Which 10 roads have the most legal turns after them?

## Traffic signals and crossings

Questions about `features.traffic`. The column `kind` is `traffic_signals` or `crossing`, and `tags` has the OSM tags (for example `tags['crossing']` and `tags['tactile_paving']`). A result with the column `geom` draws the points on the map.

- How many traffic signals and how many crossings are there?
- Show the traffic signals on the map.
- Show the crossings that have traffic signals.
- How many crossings of each type are there?
- Show the crossings that have tactile paving.
- Show the crossings that have a pedestrian island.

## Buildings and walls

Questions about the buildings (`features.buildings`) and their walls (`main.building_faces`, with `face_type`, `computed_type` and `edited`). Walls need a database that was built with `options.building_faces: true` ([The walls of the buildings](../concepts/building-walls.md)).

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

Questions about the OSM tags of the buildings: `tags['building:levels']`, `tags['height']` and `tags['roof:shape']` of `features.buildings`. Many buildings have no such tag. A question counts only the buildings that have it.

- Which 10 buildings have the most levels?
- How many buildings have a height, and how many have a number of levels?
- Show the buildings that are taller than 30 m.
- What roof shapes do the buildings have, and how many has each shape?
- Color the buildings with more than 10 levels in red and the others in grey.
- What is the average number of levels of the apartment buildings?

## Walls and streets

Questions about the columns of `main.building_faces` that show how a wall relates to its street: `length_m`, `distance_m` (the distance to the street that the wall faces), `street_name` and `type_reason` (why the wall has its type). The Monaco sample has no door tag, so there is no question about doors.

- What is the average length of the walls of each type?
- Show the walls that are longer than 20 m.
- Which streets do the most front walls face?
- Show the walls that are more than 30 m from the nearest street.
- Why do the walls have their type? Count the walls for each reason.

## My changes

Questions about the fixes file, which the page reads as the table `extra.edits` (`kind`, `type`, `osm_id`, `what`, `changed`, `note`). If there is no fixes file, the table is empty. See [Edit OSM data](edit-osm-data.md).

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

The page draws a result that has a geometry column: nodes as points, buildings as shapes, walls as lines. The table shows the first 1000 rows, and the map shows every row. A wall has a color for its `face_type`, and any shape can have a column `color` with a CSS color.

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

Ask colors roads, or keeps some roads and fades the others. It uses the road classes that the page tells it (for example `primary`, `secondary`, `residential`).

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

The page computes the H3 cells of the boundary (the table `extra.boundary_h3`, resolutions 0 to 11), because DuckDB has no H3 functions. A result with a column `h3_cell` is drawn as hexagons. The nodes have `h3_cell` at resolution 8.

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

Ask can turn the layers on and off, change the base map, turn the 3D view on, and move the camera. It reads the layers and the base maps of the page first. The layers of the page are, for example, water, waterways, buildings, parking, railways, traffic signals, crossings and bus stops.

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

The shapes that the page draws for a result (H3 cells, buildings, walls, nodes) have a tool of their own, `set_shapes`. It changes their opacity. A new result keeps the look. "Go back to the default look" brings the standard look back. The map layers (water, parking, base map) are a different tool: see *Water, 3D and base maps*.

- Show all the buildings and make them half transparent.
- Make the shapes solid.
- Make the shapes almost invisible.
- Go back to the default look of the shapes.

## Charts of the data

Ask draws a chart of the result in the data dock: bar, line, histogram, scatter or pie. A pie has at most 6 slices.

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

## When an answer is wrong

- Send the same question again with more detail, for example the column or the class that you mean.
- Open the details of the answer ("1 query"). You see the SQL that Ask ran. You can change it in the SQL line and run it again.
- If Ask says that it cannot do something, tell it the way: for example "select the column `geom`" or "add a column `color`".
