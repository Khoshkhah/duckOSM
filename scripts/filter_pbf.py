import argparse
import subprocess
import sys
import logging
from pathlib import Path

# Configure logging
Path('logs').mkdir(exist_ok=True)
logging.basicConfig(
    level=logging.INFO,
    format='[%(asctime)s] [%(levelname)s] [%(name)s] - %(message)s',
    datefmt='%H:%M:%S',
    handlers=[
        logging.FileHandler("logs/filter.log"),
        logging.StreamHandler()
    ]
)
logger = logging.getLogger("filter-pbf")

def filter_pbf(input_file: str, boundary_file: str, output_file: str):
    """
    Filter PBF file using osmium-tool with the given boundary.
    """
    if not Path(input_file).exists():
        raise FileNotFoundError(f"Input file {input_file} not found")
    if not Path(boundary_file).exists():
        raise FileNotFoundError(f"Boundary file {boundary_file} not found")
        
    logger.info(f"Filtering PBF using osmium-tool...")
    logger.info(f"Input: {input_file}")
    logger.info(f"Boundary: {boundary_file}")
    logger.info(f"Output: {output_file}")
    
    try:
        # Generate the osmium extract command
        # -p: boundary polygon
        # -s: strategy (simple or complete_ways)
        # We use complete_ways to ensure we don't break way geometries
        cmd = [
            "osmium", "extract",
            "-p", boundary_file,
            input_file,
            "-o", output_file,
            "--overwrite"
        ]
        
        result = subprocess.run(cmd, check=True, capture_output=True, text=True)
        # Note: osmium often writes stats to stderr, which is normal
        if result.stderr:
            logger.debug(f"Osmium stderr: {result.stderr}")
            
        print("Filtering completed successfully!")
        
    except subprocess.CalledProcessError as e:
        logger.error(f"osmium error: {e.stderr}")
        raise RuntimeError(f"Osmium failed: {e.stderr}")
    except FileNotFoundError:
        logger.error("Error: 'osmium' command not found. Please install osmium-tool.")
        raise FileNotFoundError("'osmium' command not found")

def main():
    parser = argparse.ArgumentParser(description="Filter PBF file by GeoJSON boundary using osmium-tool")
    parser.add_argument("--input", required=True, help="Input PBF file")
    parser.add_argument("--boundary", required=True, help="GeoJSON boundary file")
    parser.add_argument("--output", required=True, help="Output PBF file")
    
    args = parser.parse_args()
    
    try:
        filter_pbf(args.input, args.boundary, args.output)
    except Exception as e:
        sys.exit(1)

if __name__ == "__main__":
    main()
