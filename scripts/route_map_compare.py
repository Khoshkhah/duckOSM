"""Browser check for a `duckosm route-map` page made with `-m walk+drive`: for random trips a few metres
off a road, the page and `route_multimodal_points` give the same time. Needs playwright.

    python scripts/route_map_compare.py monaco.duckdb reports/monaco_route_map.html [trips]
"""
import asyncio
import os
import random
import sys

import duckdb
from playwright.async_api import async_playwright

from duckosm import route_multimodal_points

PICK = """() => { const ids = rsQuery(() => true), d = map.getSource('roads')._data, fs = (d.features || d.geojson.features), out = [];
  for (let n = 0; n < ids.length; n += 7) { const g = fs[ids[n]] && fs[ids[n]].geometry;
    if (g && g.type === 'LineString') out.push(g.coordinates[Math.floor(g.coordinates.length / 2)]); }
  return out; }"""
JS = """async ([a, b]) => {
  map.jumpTo({center: [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2], zoom: 13.5}); await new Promise((r) => map.once('idle', r));
  rmRoute(a, b); const r = window.rmLast && window.rmLast.res;
  return r ? {time: r.time, s: [r.start.f, r.start.d], e: [r.end.f, r.end.d]} : null; }"""


async def main(db, page, n):
    con = duckdb.connect(db, read_only=True)
    async with async_playwright() as p:
        b = await p.chromium.launch()
        pg = await b.new_page(viewport={"width": 1100, "height": 750})
        errs = []
        pg.on("pageerror", lambda e: errs.append(str(e)))
        await pg.goto("file://" + os.path.abspath(page))
        await pg.wait_for_function("window.rmLast && window.rmLast.res", timeout=30000)
        await pg.wait_for_timeout(800)
        pts = await pg.evaluate(PICK)                       # points on the page's own roads
        rng = random.Random(1)
        same = diff = skipped = 0
        for _ in range(n):
            a, c = (list(rng.choice(pts)) for _ in range(2))
            for q in (a, c):                                 # a few metres off the road, as a click is
                q[0] += rng.uniform(-4e-5, 4e-5)
                q[1] += rng.uniform(-4e-5, 4e-5)
            web = await pg.evaluate(JS, [a, c])
            if web is None:
                skipped += 1
                continue
            try:
                py = route_multimodal_points(con, a, c)
            except ValueError:                              # no walking road within the radius
                py = None
            if py is None:
                skipped += 1
                continue
            if py is not None and abs(py["time_s"] - web["time"]) < 0.1:
                same += 1
            else:
                diff += 1
                print("differs", a, c, None if py is None else round(py["time_s"], 2), round(web["time"], 2))
        print(f"{same} same, {diff} different, {skipped} not compared (no walking road in reach, or no route); page errors: {errs[:2]}")
        await b.close()


asyncio.run(main(sys.argv[1], sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 30))
