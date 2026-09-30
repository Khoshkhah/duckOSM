def utm_epsg(lon, lat):
    """EPSG code of the WGS84 UTM zone containing (lon, lat), e.g. 'EPSG:32632' for Monaco."""
    return f"EPSG:{(32600 if lat >= 0 else 32700) + int((lon + 180) // 6) % 60 + 1}"


def data_utm_crs(con, table, geom="geom"):
    """A metric CRS that fits the data wherever it is: the UTM zone of ``table``'s centre."""
    lon, lat = con.execute(f"SELECT avg(ST_X(ST_Centroid({geom}))), avg(ST_Y(ST_Centroid({geom}))) "
                           f"FROM {table} WHERE {geom} IS NOT NULL").fetchone()
    return utm_epsg(lon, lat)
