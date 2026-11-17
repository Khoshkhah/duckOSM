## Validation

| mode · check | result | detail |
|---|:---:|---|
| driving · unique_node_id | ✅ | 0 duplicate node_id(s) in nodes |
| driving · way_length_conserved | ✅ | 0 deleted parallel arc(s) (way stretch missing while a kept edge bridges its endpoints) |
| driving · layer_without_structure | ⚠️ | 11 edge(s) with layer≠0 but no bridge/tunnel tag (OSM tagging; drawn grade-separated, no 3D deck) |
| walking · unique_node_id | ✅ | 0 duplicate node_id(s) in nodes |
| walking · way_length_conserved | ✅ | 0 deleted parallel arc(s) (way stretch missing while a kept edge bridges its endpoints) |
| walking · layer_without_structure | ⚠️ | 282 edge(s) with layer≠0 but no bridge/tunnel tag (OSM tagging; drawn grade-separated, no 3D deck) |
| cycling · unique_node_id | ✅ | 0 duplicate node_id(s) in nodes |
| cycling · way_length_conserved | ✅ | 0 deleted parallel arc(s) (way stretch missing while a kept edge bridges its endpoints) |
| cycling · layer_without_structure | ⚠️ | 253 edge(s) with layer≠0 but no bridge/tunnel tag (OSM tagging; drawn grade-separated, no 3D deck) |
