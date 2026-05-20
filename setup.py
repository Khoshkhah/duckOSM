from setuptools import setup, find_packages

setup(
    name="duckosm",
    version="0.1.0",
    description="High-performance OSM-to-routing-network converter built on DuckDB",
    author="Kaveh",
    packages=find_packages(where="src"),
    package_dir={"": "src"},
    python_requires=">=3.10",
    install_requires=[
        "duckdb>=1.0.0",
        "shapely>=2.0.0",
        "pyyaml>=6.0",
        "click>=8.0",
        "h3>=4.0.0",
    ],
    entry_points={
        "console_scripts": [
            "duckosm=duckosm.cli:main",
        ],
    },
)
