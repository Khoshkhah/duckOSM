# Questions to ask the AI agent

These are questions that you can ask in **Ask the data** on the page of [`duckosm ask`](query.md#ask-questions-on-a-page-duckosm-ask). They are in groups. For each question, the tables, the columns and the map actions that it needs exist in the Monaco sample, and the SQL of the main ones was run there.

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

## Buildings and walls

Questions about the buildings (`features.buildings`) and their walls (`main.building_faces`, with `face_type`, `computed_type` and `edited`). Walls need a database that was built with `options.building_faces: true` ([The walls of the buildings](../concepts/building-walls.md)).

- How many walls of each type do the buildings have?
- Which buildings have no front wall?
- Which buildings have more than 10 walls?
- What are the 100 longest walls?

## My changes

Questions about the fixes file, which the page reads as the table `extra.edits` (`kind`, `type`, `osm_id`, `what`, `changed`, `note`). See [Edit OSM data](edit-osm-data.md).

- Which objects did I change, and when?
- Which walls did I change by hand?
- What did I change in the last day?

## Shapes on the map

The page draws a result that has a geometry column: nodes as points, buildings as shapes, walls as lines. The table shows the first 1000 rows, and the map shows every row. A wall has a color for its `face_type`, and any shape can have a column `color` with a CSS color.

- Show all buildings on the map.
- Show the walls of all buildings, with a color for each wall type.
- Show the dead-end nodes of the driving network.
- Show all nodes of the driving network on the map.
- Show only the front walls of the buildings.
- Color the walls that I changed by hand in red and the others in grey.

## Roads: color and focus

Ask colors roads, or keeps some roads and fades the others. It uses the road classes that the page tells it (for example `primary`, `secondary`, `residential`).

- Color the roads by their class.
- Show only the primary roads and fade the others.
- Color the one-way roads red and the two-way roads grey.
- Color the roads that have a speed limit above 50 km/h.

## H3 cells and the boundary

The page computes the H3 cells of the boundary (the table `extra.boundary_h3`, resolutions 0 to 11), because DuckDB has no H3 functions. A result with a column `h3_cell` is drawn as hexagons. The nodes have `h3_cell` at resolution 8.

- Show the smallest H3 cell that contains the whole boundary.
- Show the H3 cells of the boundary at resolution 9.
- How many nodes are in each H3 cell?
- Make the H3 cells more transparent.

## Water, 3D and base maps

Ask can turn the layers on and off, change the base map, turn the 3D view on, and move the camera. It reads the layers and the base maps of the page first.

- Turn off the water and the waterways layers.
- Switch the base map to satellite.
- Turn on the 3D view and tilt the map.
- Switch to the dark base map.

## Charts of the roads

Ask draws a chart of the result in the data dock: bar, line, histogram, scatter or pie.

- Draw a bar chart of the km of road in each class.
- Draw a histogram of the road lengths.
- Show a pie chart of the one-way and the two-way roads.

## Change the look of the shapes

The shapes that the page draws for a result (H3 cells, buildings, walls, nodes) have a tool of their own, `set_shapes`: ask "Make the H3 cells more transparent", or "Make the shapes solid". A new result keeps the look. "Back to the default look" brings the standard look back. The map layers (water, parking, base map) are a different tool: see *Water, 3D and base maps*.

## When an answer is wrong

- Send the same question again with more detail, for example the column or the class that you mean.
- Open the details of the answer ("1 query"). You see the SQL that Ask ran. You can change it in the SQL line and run it again.
- If Ask says that it cannot do something, tell it the way: for example "select the column `geom`" or "add a column `color`".
