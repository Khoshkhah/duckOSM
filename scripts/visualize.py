import streamlit as st
import duckdb
import pydeck as pdk
import pandas as pd
from shapely import wkt
from shapely.geometry import LineString
from pathlib import Path
import time
import os

# --- PAGE CONFIG ---
st.set_page_config(
    layout="wide", 
    page_title="duckOSM Viewer",
    page_icon="",
    initial_sidebar_state="expanded"
)

# --- CSS: TRUE MINIMALIST FULLSCREEN ---
st.markdown("""
    <style>
    /* Force 100% viewport height and hide everything extraneous */
    html, body, [data-testid="stApp"] {
        height: 100vh;
        overflow: hidden;
        background-color: #0e1117 !important;
    }
    
    [data-testid="stAppViewBlockContainer"] {
        padding: 0 !important;
        margin: 0 !important;
        max-width: 100% !important;
        height: 100vh !important;
    }

    [data-testid="stVerticalBlock"] {
        gap: 0 !important;
    }

    /* Fixed Sidebar styling - Dark Theme */
    [data-testid="stSidebar"] {
        background-color: #161b22 !important;
        border-right: 1px solid #30363d;
    }
    
    /* Hide menus, footer and padding */
    #MainMenu {visibility: hidden;}
    footer {visibility: hidden;}
    header {visibility: hidden;}
    [data-testid="stHeader"] {display: none;}

    /* Sidebar text visibility */
    [data-testid="stSidebar"] [data-testid="stWidgetLabel"] p,
    [data-testid="stSidebar"] .stMarkdown p,
    [data-testid="stSidebar"] .stMarkdown h3 {
        color: #c9d1d9 !important;
        font-weight: 600;
        font-size: 0.9rem;
    }
    
    /* Dark mode selectbox */
    .stSelectbox div[data-baseweb="select"] > div {
        background-color: #0d1117 !important;
        color: #c9d1d9 !important;
        border-color: #30363d !important;
    }
    </style>
""", unsafe_allow_html=True)

# --- SIDEBAR: MINIMAL & CLEAN ---
with st.sidebar:
    st.markdown("### duckOSM Explorer")
    
    # Search in multiple locations
    search_paths = [Path("data/output"), Path("../../data")]
    db_files = []
    for p in search_paths:
        if p.exists():
            db_files.extend(list(p.glob("*.duckdb")))
    
    if not db_files:
        st.error("No DuckDB files found.")
        st.stop()
        
    selected_db = st.selectbox("Database", options=db_files, format_func=lambda x: x.name)
    
    @st.cache_resource
    def get_con(path):
        c = duckdb.connect(str(path), read_only=True)
        c.execute("INSTALL spatial; LOAD spatial;")
        return c
    
    con = get_con(selected_db)
    st.caption(f"Connected: {selected_db.name}")
    
    # Discovery
    schemas = [r[0] for r in con.execute("SELECT schema_name FROM information_schema.schemata WHERE schema_name NOT IN ('information_schema', 'pg_catalog', 'main', 'raw')").fetchall()]
    
    st.markdown("---")
    st.markdown("**Networks**")
    selected_modes = []
    
    # Define standard modes
    available_modes = ["driving", "walking", "cycling"]
    
    # Create checkboxes for each standard mode if they exist in schemas
    for mode in available_modes:
        if mode in schemas:
            if st.checkbox(mode.capitalize(), value=(mode == "driving")):
                selected_modes.append(mode)
    
    # Handle non-standard schemas
    for schema in schemas:
        if schema not in available_modes and schema != 'shortcuts':
            if st.checkbox(schema.capitalize(), value=False):
                selected_modes.append(schema)
    
    # Shortcut visualization toggle
    show_shortcuts = False
    if 'shortcuts' in schemas:
        st.markdown("---")
        st.markdown("**Hierarchy**")
        show_shortcuts = st.checkbox("Show Shortcuts", value=False)
    
    
    # Simple selection for Map Styles
    basemap_name = st.selectbox(
        "Style",
        options=["Dark Matter", "Light (Positron)", "Voyager"],
        index=0
    )

    st.markdown("---")
    st.markdown("**Performance**")
    edge_limit = st.slider("Max Edges", 5000, 500000, 50000, step=5000)
    st.caption("Lower this if you hit 200MB limit")

    if selected_modes:
        st.markdown("---")
        # High contrast legend
        mode_colors_hex = {'driving': '#ff9f43', 'walking': '#54a0ff', 'cycling': '#1dd1a1'}
        for mode in selected_modes:
            color = mode_colors_hex.get(mode.lower(), '#c9d1d9')
            st.markdown(f'<div style="display: flex; align-items: center; margin-bottom:4px; font-size:0.85rem; color:#c9d1d9;"><div style="width: 12px; height: 12px; background-color: {color}; margin-right: 8px; border-radius: 50%;"></div>{mode.capitalize()}</div>', unsafe_allow_html=True)
    
    st.markdown("---")
    st.markdown("**Debug**")
    debug_ph = st.empty()


# --- MAP STYLE & CACHE BUSTING ---
map_key = f"deck-{basemap_name}-{'-'.join(selected_modes)}"

style_urls = {
    "Dark Matter": "https://basemaps.cartocdn.com/gl/dark_matter-gl-style/style.json",
    "Light (Positron)": "https://basemaps.cartocdn.com/gl/positron-gl-style/style.json",
    "Voyager": "https://basemaps.cartocdn.com/gl/voyager-gl-style/style.json"
}
chosen_style = style_urls[basemap_name]

# Hardcoded constants
LINE_WIDTH = 4

# --- DATA LOADING ---
mtime = Path(__file__).stat().st_mtime

@st.cache_data(hash_funcs={duckdb.DuckDBPyConnection: id})
def fetch_network_data(db_path, mode, limit, cache_key=mtime):
    # Discover available columns in the table
    try:
        cols_df = con.execute(f"DESCRIBE {mode}.edges").df()
        available_cols = set(cols_df['column_name'].tolist())
        # st.sidebar.caption(f"{mode} cols: {len(available_cols)}")
    except Exception as e:
        st.error(f"Error discovering columns for {mode}: {e}")
        return None

    # Define desired columns and their fallbacks
    select_fields = []
    
    # Priority columns for visualization
    if 'geometry' in available_cols: select_fields.append("ST_AsText(geometry) as wkt_geom")
    else: 
        st.error(f"No geometry column in {mode}")
        return None
    
    if 'osm_id' in available_cols: select_fields.append("osm_id")
    elif 'id' in available_cols: select_fields.append("id as osm_id")
    else: select_fields.append("0 as osm_id")
    
    if 'name' in available_cols: select_fields.append("name")
    else: select_fields.append("'Unknown' as name")
    
    if 'highway' in available_cols: select_fields.append("highway")
    else: select_fields.append("'road' as highway")
    
    if 'is_reverse' in available_cols: select_fields.append("is_reverse")
    else: select_fields.append("false as is_reverse")
    
    if 'length_m' in available_cols: select_fields.append("round(length_m, 1) as length_m")
    elif 'length' in available_cols: select_fields.append("round(length, 1) as length_m")
    else: select_fields.append("0.0 as length_m")
    
    if 'maxspeed_kmh' in available_cols: select_fields.append("round(maxspeed_kmh, 1) as speed_kmh")
    elif 'cost' in available_cols: select_fields.append("round(cost, 1) as speed_kmh")
    else: select_fields.append("0.0 as speed_kmh")

    query = f"""
        SELECT 
            {', '.join(select_fields)}
        FROM {mode}.edges
        ORDER BY length_m DESC
        LIMIT {limit}
    """
    df = con.execute(query).df()
    if df.empty: return None
    
    # Helper for Python-side parallel offset
    def parse_path_with_offset(row):
        try:
            line = wkt.loads(row['wkt_geom'])
            if row['is_reverse']:
                # Apply slight offset in Python for visibility
                line = line.parallel_offset(0.00003, 'right', join_style=2)
            return list(line.coords)
        except:
            # Fallback to simple parse if offset fails
            try:
                line = wkt.loads(row['wkt_geom'])
                return list(line.coords)
            except:
                return []
            
    df['path'] = df.apply(parse_path_with_offset, axis=1)
    
    # Ensure osm_id is safe for JSON (handle HUGEINT)
    if 'osm_id' in df.columns:
        df['osm_id'] = df['osm_id'].astype(str)
    
    mode_colors = {

        'driving': [255, 159, 67, 180],
        'walking': [84, 160, 255, 180],
        'cycling': [29, 209, 161, 180]
    }
    df['color'] = [mode_colors.get(mode.lower(), [201, 209, 217, 180])] * len(df)
    return df

@st.cache_data(hash_funcs={duckdb.DuckDBPyConnection: id})
def fetch_shortcut_data(db_path, cache_key=mtime):
    # Discover columns in shortcuts.edges
    try:
        cols_df = con.execute("DESCRIBE shortcuts.edges").df()
        available_cols = set(cols_df['column_name'].tolist())
    except Exception as e:
        return None

    # Fetch shortcuts and their corresponding edge geometries
    select_fields = ["s.from_edge", "s.to_edge", "s.cost"]
    
    if 'geometry' in available_cols: select_fields.append("ST_AsText(e.geometry) as wkt_geom")
    else: return None
    
    if 'length' in available_cols: select_fields.append("round(e.length, 1) as length_m")
    elif 'length_m' in available_cols: select_fields.append("round(e.length_m, 1) as length_m")
    else: select_fields.append("0.0 as length_m")

    query = f"""
        SELECT 
            {', '.join(select_fields)}
        FROM shortcuts.shortcuts s
        JOIN shortcuts.edges e ON s.from_edge = e.id
        LIMIT 50000 
    """
    df = con.execute(query).df()
    if df.empty: return None

    def parse_wkt(wkt_str):
        try:
            line = wkt.loads(wkt_str)
            return list(line.coords)
        except:
            return []
            
    df['path'] = df['wkt_geom'].apply(parse_wkt)
    df['color'] = [[255, 0, 255, 120]] * len(df) # Magenta for shortcuts
    return df


all_layers = []
with st.sidebar:
    status = st.empty()

for mode in selected_modes:
    status.info(f"Loading {mode}...")
    df = fetch_network_data(selected_db, mode, edge_limit, cache_key=mtime)

    if df is not None:
        status.success(f"{mode}: {len(df)} edges")
        all_layers.append(pdk.Layer(
            "PathLayer",
            df,
            pickable=True,
            get_path="path",
            get_color="color",
            get_width=LINE_WIDTH,
            width_min_pixels=1,
            auto_highlight=True
        ))
    else:
        status.warning(f"{mode}: No data")

if show_shortcuts:
    status.info("Loading shortcuts...")
    sdf = fetch_shortcut_data(selected_db, cache_key=mtime)

    if sdf is not None:
        status.success(f"Shortcuts: {len(sdf)}")
        all_layers.append(pdk.Layer(
            "PathLayer",
            sdf,
            pickable=True,
            get_path="path",
            get_color="color",
            get_width=2,
            width_min_pixels=1,
            auto_highlight=True
        ))
    else:
        status.warning("Shortcuts: no data")

status.empty()

# --- RENDERING ---
if all_layers:
    # Centering logic
    lat, lon, zoom = None, None, None
    
    # 1. Try metadata
    try:
        meta = con.execute("SELECT center_lat, center_lon, initial_zoom FROM main.visualization").fetchone()
        if meta and not any(x is None for x in meta):
            lat, lon, zoom = meta
    except:
        pass # Metadata missing or invalid

    # 2. Fallback to data calculation if metadata failed
    if lat is None:
        try:
            first_mode = selected_modes[0]
            # Use a small limit for centering purposes to be fast
            first_df = fetch_network_data(selected_db, first_mode, min(edge_limit, 5000), cache_key=mtime)
            if first_df is not None and not first_df.empty and len(first_df['path']) > 0:
                first_path = list(first_df['path'])[0]
                # first_path[0] is (lon, lat)
                lon, lat = first_path[0] 
                zoom = 13
        except Exception as e:
            st.warning(f"Could not auto-center from data: {e}")

    # 3. Ultimate fallback (Vancouver)
    if lat is None:
        lat, lon, zoom = 49.2827, -123.1207, 12
    
    # Force types
    try:
        lat = float(lat)
        lon = float(lon)
        zoom = int(zoom)
    except:
        lat, lon, zoom = 49.2827, -123.1207, 12
        
    debug_ph.caption(f"View: {lat:.4f}, {lon:.4f}, Z{zoom}")
    
    view_state = pdk.ViewState(
        longitude=lon,
        latitude=lat,
        zoom=zoom,
        pitch=0
    )

    placeholder = st.empty()
    with placeholder:
        st.pydeck_chart(pdk.Deck(
            map_style=chosen_style,
            initial_view_state=view_state,
            layers=all_layers,
            height=1000, 
            tooltip={
                "html": "<b>{highway}</b>: {name}<br/>OSM ID: {osm_id}<br/>{speed_kmh} km/h | {length_m}m",
                "style": {"backgroundColor": "#161b22", "color": "#c9d1d9"}
            }
        ), key=map_key)
else:
    st.info(" Select networks in the sidebar to visualize.")
