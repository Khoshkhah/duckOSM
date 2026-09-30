"""Browser check for a `duckosm route-map` page: random trips between road points, at zoom 13.5
and 16 with both markers on screen, and at 17 with the end marker off screen; prints how many
routed and why the others didn't. Needs playwright (`pip install playwright && playwright install
chromium`).

    python scripts/route_map_stress.py reports/monaco_route_map.html
"""
import asyncio
import os
import sys
from playwright.async_api import async_playwright
JS = """async ([zoom, n, offscreen]) => {
  const ids = rsQuery(() => true), fs = map.getSource('roads')._data.features;
  const pt = () => { const c = fs[ids[Math.floor(Math.random() * ids.length)]].geometry.coordinates; return c[Math.floor(c.length / 2)]; };
  const out = {};
  for (let i = 0; i < n; i++) {
    const a = pt(), b = pt();
    map.jumpTo({center: offscreen ? a : [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2], zoom});
    await new Promise((r) => map.once('idle', r));
    rmRoute(a, b);
    const t = document.getElementById('rm-result').innerText.split('\\n')[0];
    const key = (window.rmLast && window.rmLast.res && t.includes('·')) ? 'ok' : t.slice(0, 50);
    out[key] = (out[key] || 0) + 1;
  }
  return out;
}"""
async def main(path):
    async with async_playwright() as p:
        b = await p.chromium.launch(); pg = await b.new_page(viewport={"width": 1100, "height": 750})
        errs = []; pg.on("pageerror", lambda e: errs.append(str(e)))
        await pg.goto("file://" + os.path.abspath(path))
        await pg.wait_for_function("window.rmLast && window.rmLast.res", timeout=30000); await pg.wait_for_timeout(800)
        for zoom, off in [(13.5, False), (16, False), (17, True)]:
            print(path.split('/')[-1], "zoom", zoom, "B off screen" if off else "both on screen", await pg.evaluate(JS, [zoom, 40, off]))
        print("errors", errs[:2]); await b.close()
asyncio.run(main(sys.argv[1]))
