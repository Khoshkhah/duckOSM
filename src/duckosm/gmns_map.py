"""
Pretty, self-contained HTML maps of a GMNS DuckDB — a presentation counterpart to the utilitarian
[gmns_viewer](gmns_viewer.py). Two styles:

  * ``road`` — one **carriageway ribbon per direction** (a two-way road splits into two), width =
    lane count × lane-width, offset to the travel side, coloured by road class and cased.
  * ``lane`` — **every lane** as a ribbon of its real width (metres) along its own geometry, coloured
    by use (auto / bus / bike), with thin gaps as lane markings.

Both overlay the smooth Bézier turn connectors, render on a dark canvas with a neon-ish glow/casing,
and support pan/zoom. Needs only ``duckdb`` (+ spatial); geometry is drawn client-side.

    from duckosm.gmns_map import write_map
    write_map("sodermalm_gmns.duckdb", "road.html", style="road")
    write_map("sodermalm_gmns.duckdb", "lanes.html", style="lane")
"""
import json
import logging
import math
from pathlib import Path

logger = logging.getLogger("duckosm")

_Q = 8192
_LW = 3.25                                              # default lane width (m) when width:lanes absent
_CLASS = {"motorway": 4, "trunk": 4, "primary": 3, "secondary": 2, "tertiary": 1,
          "residential": 0, "unclassified": 0, "living_street": 0, "service": 0, "road": 0}
_USE = {"auto": 0, "bus": 1, "bike": 2}


def _coords(w):
    b = w[w.index("(")+1:w.rindex(")")]
    return [tuple(float(v) for v in p.split()[:2]) for p in b.split(",")]


def _cls(ft):
    if not ft:
        return 0
    h = ft.split(";")[0]
    if h.endswith("_link"):
        h = h[:-5]
    return _CLASS.get(h, 0)


def _quantizer(wkts):
    """Build a lon/lat → compact-grid quantizer covering all the given WKT linestrings."""
    allc = [p for w in wkts for p in _coords(w)]
    xs = [p[0] for p in allc]; ys = [p[1] for p in allc]
    minx, maxx, miny, maxy = min(xs), max(xs), min(ys), max(ys)
    kx = math.cos(math.radians((miny+maxy)/2)) or 1.0
    scale = _Q / max((maxx-minx)*kx, (maxy-miny), 1e-9)
    qx = lambda x: round((x-minx)*kx*scale)
    qy = lambda y: round((maxy-y)*scale)

    def flat(w):
        o = []
        for x, y in _coords(w):
            o += [qx(x), qy(y)]
        return o
    grid = {"w": round((maxx-minx)*kx*scale), "h": round((maxy-miny)*scale)}
    return flat, qx, qy, grid, scale, (minx, maxx, miny, maxy)


def _conns(con, mode):
    """The turn curves: every GMNS movement's geometry (WKT), so no meso network is needed."""
    return [r[0] for r in con.execute(
        f"SELECT geometry FROM gmns_{mode}.movement WHERE geometry IS NOT NULL").fetchall()]


def build_road_payload(con, mode="driving"):
    from duckosm.gmns import _offset_wkt
    g = f"gmns_{mode}"
    rows = con.execute(f"""
      WITH tw AS (SELECT a.link_id FROM {g}.link a JOIN {g}.link b
                    ON a.from_node_id=b.to_node_id AND a.to_node_id=b.from_node_id)
      SELECT facility_type, lanes, ST_AsText(geom), (link_id IN (SELECT link_id FROM tw)) AS twoway
      FROM {g}.link WHERE geom IS NOT NULL""").fetchall()
    conns = _conns(con, mode)
    foc = con.execute(f"""SELECT n.x_coord, n.y_coord FROM {g}.node n
      JOIN {g}.link l ON (l.from_node_id=n.node_id OR l.to_node_id=n.node_id)
      WHERE l.facility_type IN ('primary','primary_link','secondary','secondary_link')
      GROUP BY n.node_id, n.x_coord, n.y_coord ORDER BY count(*) DESC LIMIT 1""").fetchone()

    roads = []
    for ft, lanes, wkt, twoway in rows:
        cw = int(lanes or 1) * _LW
        geom = _offset_wkt(wkt, -(cw/2 + 0.4)) if twoway else wkt          # right-hand: offset right
        roads.append((_cls(ft), cw, geom))
    flat, qx, qy, grid, scale, bb = _quantizer([wkt for *_, wkt, _ in rows])
    if not foc:
        foc = ((bb[0]+bb[1])/2, (bb[2]+bb[3])/2)
    fx, fy = qx(foc[0]), qy(foc[1]); half = round(150*scale/111320)
    return {"grid": grid, "focus": [fx-half, fy-half, fx+half, fy+half], "gpm": scale/111320.0,
            "roads": [[ci, round(cw, 2)] + flat(w) for ci, cw, w in roads],
            "conn": [flat(w) for w in conns],
            "n": {"roads": len(roads), "conn": len(conns), "file": ""}}


def build_lane_payload(con, mode="driving"):
    g = f"gmns_{mode}"
    lanes = con.execute(f"SELECT allowed_uses, COALESCE(width,{_LW}), ST_AsText(geom) "
                        f"FROM {g}.lane WHERE geom IS NOT NULL").fetchall()
    conns = _conns(con, mode)
    foc = con.execute(f"""SELECT n.x_coord,n.y_coord FROM {g}.node n
      JOIN {g}.link l ON l.to_node_id=n.node_id
      WHERE n.node_id IN (SELECT node_id FROM {g}.movement)
      ORDER BY (l.link_id IN (SELECT link_id FROM {g}.lane WHERE allowed_uses='bus')) DESC,
               l.lanes DESC LIMIT 1""").fetchone()
    flat, qx, qy, grid, scale, bb = _quantizer([w for *_, w in lanes])
    if not foc:
        foc = ((bb[0]+bb[1])/2, (bb[2]+bb[3])/2)
    fx, fy = qx(foc[0]), qy(foc[1]); half = round(48*scale/111320)
    return {"grid": grid, "focus": [fx-half, fy-half, fx+half, fy+half], "gpm": scale/111320.0,
            "lanes": [[_USE.get(u, 0), round(wm, 2)] + flat(w) for u, wm, w in lanes],
            "conn": [flat(w) for w in conns],
            "n": {"lanes": len(lanes), "bus": sum(1 for u, *_ in lanes if u == "bus"),
                  "conn": len(conns), "file": ""}}


def write_map(gmns_db, out, style="road", mode="driving", standalone=True):
    """Write a pretty GMNS HTML map. ``style`` is ``"road"`` or ``"lane"``. Returns the output path."""
    import duckdb

    if style not in ("road", "lane"):
        raise ValueError(f"style must be 'road' or 'lane', got {style!r}")
    con = duckdb.connect(str(gmns_db), read_only=True)
    con.execute("INSTALL spatial; LOAD spatial;")
    payload = (build_road_payload if style == "road" else build_lane_payload)(con, mode=mode)
    con.close()
    payload["n"]["file"] = Path(gmns_db).name
    tmpl = _ROAD_T if style == "road" else _LANE_T
    title = f"duckOSM · GMNS {'road' if style == 'road' else 'lane'} map · {Path(gmns_db).name}"
    body = f"<title>{title}</title>\n" + tmpl.replace("/*__P__*/null", json.dumps(payload, separators=(",", ":")))
    html = body if not standalone else (
        '<!doctype html>\n<html lang="en"><head><meta charset="utf-8">'
        '<meta name="viewport" content="width=device-width,initial-scale=1">'
        f'<title>{title}</title></head><body>\n{body}\n</body></html>\n')
    Path(out).parent.mkdir(parents=True, exist_ok=True)
    Path(out).write_text(html, encoding="utf-8")
    logger.info(f"GMNS {style} map [{mode}] -> {out}")
    return out


_CHROME = r"""<style>
:root{--sans:system-ui,-apple-system,"Segoe UI",Roboto,sans-serif;--mono:ui-monospace,"SF Mono",Menlo,Consolas,monospace}
*{box-sizing:border-box}html,body{margin:0}
.hero{position:relative;height:100vh;min-height:560px;background:#0b0f15;overflow:hidden;font-family:var(--sans);color:#e6edf3}
.hero canvas{position:absolute;inset:0;width:100%;height:100%;display:block;cursor:grab;touch-action:none;opacity:0;transition:opacity 1s ease}
.hero canvas.in{opacity:1}.hero canvas:active{cursor:grabbing}
.vig{position:absolute;inset:0;pointer-events:none;background:radial-gradient(125% 95% at 50% 42%,transparent 55%,rgba(4,6,10,.75) 100%)}
.cap{position:absolute;left:16px;top:16px;padding:10px 14px 12px;border-radius:8px;background:rgba(11,15,21,.97);pointer-events:none}
.cap .k{font-family:var(--mono);font-size:11px;letter-spacing:.24em;text-transform:uppercase;color:#6a8a92}
.cap h1{margin:6px 0 0;font-size:25px;font-weight:600;letter-spacing:-.015em;line-height:1.12;text-shadow:0 2px 22px rgba(0,0,0,.6)}
.cap h1 b{color:#f0b25a;font-weight:600}.cap p{margin:9px 0 0;font-family:var(--mono);font-size:12px;color:#8397a6;max-width:36ch}
.leg{position:absolute;left:16px;bottom:16px;padding:8px 12px;border-radius:8px;background:rgba(11,15,21,.97);display:flex;flex-wrap:wrap;gap:14px;font-family:var(--mono);font-size:12px;color:#9fb1c0;pointer-events:none}
.leg span{display:inline-flex;align-items:center;gap:7px}.leg i{width:15px;height:9px;border-radius:2px;display:inline-block}
.hint{position:absolute;right:22px;bottom:22px;font-family:var(--mono);font-size:11px;color:#5f7183;pointer-events:none}
@media(prefers-reduced-motion:reduce){.hero canvas{transition:none}}
@media(max-width:900px){.hint{display:none}}
</style>"""

_PANZOOM = r"""
let drag=null;
cv.addEventListener('pointerdown',e=>{drag={x:e.clientX,y:e.clientY,tx:v.tx,ty:v.ty};cv.setPointerCapture(e.pointerId);});
cv.addEventListener('pointermove',e=>{if(drag){v.tx=drag.tx+(e.clientX-drag.x);v.ty=drag.ty+(e.clientY-drag.y);draw();}});
cv.addEventListener('pointerup',()=>drag=null);cv.addEventListener('pointercancel',()=>drag=null);
cv.addEventListener('wheel',e=>{e.preventDefault();const r=cv.getBoundingClientRect(),mx=e.clientX-r.left,my=e.clientY-r.top,
  k=Math.exp(-e.deltaY*.0016),ns=Math.min(1400,Math.max(.03,v.s*k));v.tx=mx-(mx-v.tx)*(ns/v.s);v.ty=my-(my-v.ty)*(ns/v.s);v.s=ns;draw();},{passive:false});
function resize(){const r=cv.getBoundingClientRect();cv.width=Math.max(1,r.width*dpr);cv.height=Math.max(1,r.height*dpr);fit();}
function fit(){const r=cv.getBoundingClientRect(),pad=44,b=D.focus,bw=b[2]-b[0],bh=b[3]-b[1];
  const s=Math.min((r.width-2*pad)/bw,(r.height-2*pad)/bh);v.s=s;v.tx=r.width/2-((b[0]+b[2])/2)*s;v.ty=r.height/2-((b[1]+b[3])/2)*s;draw();}
new ResizeObserver(resize).observe(cv);resize();requestAnimationFrame(()=>cv.classList.add('in'));"""

_ROAD_T = _CHROME + r"""
<div class="hero"><canvas id="c"></canvas><div class="vig"></div>
  <div class="cap"><div class="k" id="file"></div>
    <h1>Roads by <b>direction</b> — one carriageway each way</h1><p id="sub"></p></div>
  <div class="leg"><span><i style="background:#e8663c"></i>trunk</span><span><i style="background:#f0a44a"></i>primary</span>
    <span><i style="background:#e6c95c"></i>secondary</span><span><i style="background:#aebac6"></i>tertiary</span>
    <span><i style="background:#7f8c99"></i>local</span><span><i style="background:#39c6d6"></i>turn</span></div>
  <div class="hint">scroll to zoom · drag to pan</div></div>
<script>const D=/*__P__*/null;(function(){const $=i=>document.getElementById(i);
$('file').textContent='duckOSM · GMNS · '+D.n.file;
$('sub').textContent=D.n.roads.toLocaleString()+' directed carriageways · two-way roads split into two ribbons';
const cv=$('c'),ctx=cv.getContext('2d'),GPM=D.gpm;let dpr=Math.min(window.devicePixelRatio||1,2),v={s:1,tx:0,ty:0};
const X=x=>(x*v.s+v.tx)*dpr,Y=y=>(y*v.s+v.ty)*dpr,FILL=['#7f8c99','#aebac6','#e6c95c','#f0a44a','#e8663c'];
function poly(g,o){ctx.moveTo(X(g[o]),Y(g[o+1]));for(let i=o+2;i<g.length;i+=2)ctx.lineTo(X(g[i]),Y(g[i+1]));}
function draw(){ctx.setTransform(1,0,0,1,0,0);ctx.fillStyle='#0b0f15';ctx.fillRect(0,0,cv.width,cv.height);
  ctx.lineCap='round';ctx.lineJoin='round';const byC=[[],[],[],[],[]];for(const R of D.roads)byC[R[0]].push(R);
  for(let ci=0;ci<5;ci++){const arr=byC[ci];if(!arr.length)continue;
    ctx.strokeStyle='#0c1017';for(const R of arr){ctx.lineWidth=Math.max(2,R[1]*GPM*v.s*dpr+2.5*dpr);ctx.beginPath();poly(R,2);ctx.stroke();}
    ctx.strokeStyle=FILL[ci];for(const R of arr){ctx.lineWidth=Math.max(1,R[1]*GPM*v.s*dpr);ctx.beginPath();poly(R,2);ctx.stroke();}}
  ctx.strokeStyle='#39c6d6';const cw=Math.max(.8,2.6*GPM*v.s*dpr);
  for(const g of D.conn){ctx.lineWidth=cw;ctx.beginPath();poly(g,0);ctx.stroke();}}
""" + _PANZOOM + "})();</script>"

_LANE_T = _CHROME + r"""
<div class="hero"><canvas id="c"></canvas><div class="vig"></div>
  <div class="cap"><div class="k" id="file"></div>
    <h1>Every lane, its own <b>width &amp; geometry</b></h1><p id="sub"></p></div>
  <div class="leg"><span><i style="background:#3d4654;outline:1px solid #e6ebf0"></i>traffic lane</span><span><i style="background:#8a5a33;outline:1px solid #e6ebf0"></i>bus lane</span>
    <span><i style="background:#2f6488;outline:1px solid #e6ebf0"></i>bike lane</span><span><i style="background:#39c6d6"></i>turn connector</span></div>
  <div class="hint">scroll to zoom · drag to pan · widths in metres</div></div>
<script>const D=/*__P__*/null;(function(){const $=i=>document.getElementById(i);
$('file').textContent='duckOSM · GMNS · '+D.n.file;
$('sub').textContent=D.n.lanes.toLocaleString()+' lanes as width ribbons · '+D.n.bus+' bus · '+D.n.conn.toLocaleString()+' smooth turn connectors';
const cv=$('c'),ctx=cv.getContext('2d'),GPM=D.gpm;let dpr=Math.min(window.devicePixelRatio||1,2),v={s:1,tx:0,ty:0};
const X=x=>(x*v.s+v.tx)*dpr,Y=y=>(y*v.s+v.ty)*dpr,LC=['#3d4654','#8a5a33','#2f6488'];
function poly(g,o){ctx.moveTo(X(g[o]),Y(g[o+1]));for(let i=o+2;i<g.length;i+=2)ctx.lineTo(X(g[i]),Y(g[i+1]));}
function draw(){ctx.setTransform(1,0,0,1,0,0);ctx.fillStyle='#0a0d13';ctx.fillRect(0,0,cv.width,cv.height);
  ctx.lineCap='round';ctx.lineJoin='round';const m=GPM*v.s*dpr;
  // white under, asphalt on top: what is left of the white between two lanes is the lane line
  ctx.strokeStyle='#e6ebf0';for(const L of D.lanes){ctx.lineWidth=Math.max(1,L[1]*m);ctx.beginPath();poly(L,2);ctx.stroke();}
  for(const L of D.lanes){const w=L[1]*m;ctx.strokeStyle=LC[L[0]];
    ctx.lineWidth=Math.max(.8,w-Math.min(w*.3,Math.max(1.2*dpr,.3*m)));ctx.beginPath();poly(L,2);ctx.stroke();}
  if(3.25*m>14){ctx.fillStyle='rgba(230,235,240,.85)';const a=.9*m;       // one arrow per lane: its direction
    for(const L of D.lanes){const n=(L.length-2)/2,j=Math.floor((n-1)/2),i=2+2*j;if(n<2)continue;
      const x0=X(L[i]),y0=Y(L[i+1]),x1=X(L[i+2]),y1=Y(L[i+3]),dx=x1-x0,dy=y1-y0,d=Math.hypot(dx,dy);if(d<3*a)continue;
      const ux=dx/d,uy=dy/d,cx=(x0+x1)/2,cy=(y0+y1)/2;ctx.beginPath();
      ctx.moveTo(cx+ux*a,cy+uy*a);ctx.lineTo(cx-ux*a-uy*a*.6,cy-uy*a+ux*a*.6);ctx.lineTo(cx-ux*a+uy*a*.6,cy-uy*a-ux*a*.6);ctx.fill();}}
  ctx.strokeStyle='#39c6d6';ctx.lineCap='butt';const cw=Math.max(.8,.6*m);for(const g of D.conn){ctx.lineWidth=cw;ctx.beginPath();poly(g,0);ctx.stroke();}}
""" + _PANZOOM + "})();</script>"
