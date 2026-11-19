# Questions to ask the AI agent

Questions for **Ask the data**, on the page of [`duckosm ask`](query.md#ask-in-words). Each one works on the Monaco
sample. A short question with one task gives the best answer.

| Topic | Question |
|---|---|
| Roads | What are the 10 longest primary roads? |
| Roads | How many edges and how many km of road does each network have? |
| Junctions and turns | Show the junctions that have a turn restriction. |
| Traffic signals | Show the crossings that have traffic signals. |
| Buildings and walls | Show the walls of all buildings, with a color for each wall type. |
| Your changes | Which objects did I change, and when? |
| Color and focus | Show only the primary roads and fade the others. |
| H3 cells | Show the H3 cells of the boundary at resolution 9. |
| The map | Switch the base map to satellite. |
| Charts | Draw a bar chart of the km of road in each class. |

All questions, by topic:
[the full list](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/ask_questions.md).

| To get a good answer | Why |
|---|---|
| Say what you want: "show", "color", "count" or "draw" | Ask then selects the right tool |
| Use the values of the data, for example `primary` or the wall types `front`, `back`, `side`, `party`, `court` | Ask writes SQL with these values |
| Click **New** for a new conversation | each step sends the full conversation again, and uses your free limit faster |
| If an answer is wrong, open "1 query" to see the SQL | change the SQL in the SQL line and run it again, or tell Ask what to do: "add a column `color`" |

Ask reads the database and controls the map. It cannot change the data: to change the OSM data, use
[Edit OSM data](edit-osm-data.md).
