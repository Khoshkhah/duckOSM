import argparse
import sys
import logging
from pathlib import Path

Path('logs').mkdir(exist_ok=True)
logging.basicConfig(
    level=logging.INFO,
    format='[%(asctime)s] [%(levelname)s] [%(name)s] - %(message)s',
    datefmt='%H:%M:%S',
    handlers=[
        logging.FileHandler("logs/download_boundary.log"),
        logging.StreamHandler()
    ]
)
logger = logging.getLogger("download-boundary")


def download_boundary(place: str, output_file: str):
    try:
        import osmnx as ox
    except ImportError:
        raise ImportError("osmnx is required. Install it with: conda install -c conda-forge osmnx")

    output_path = Path(output_file)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    logger.info(f"Fetching boundary for: {place}")
    gdf = ox.geocode_to_gdf(place)
    gdf.to_file(output_file, driver="GeoJSON")
    logger.info(f"Saved to: {output_file}")


def main():
    parser = argparse.ArgumentParser(description="Download OSM boundary as GeoJSON using osmnx")
    parser.add_argument("--place", required=True, help="Place name (e.g. 'Somerset, Kentucky, USA')")
    parser.add_argument("--output", required=True, help="Output GeoJSON file path")

    args = parser.parse_args()

    try:
        download_boundary(args.place, args.output)
    except Exception as e:
        logger.error(str(e))
        sys.exit(1)


if __name__ == "__main__":
    main()
