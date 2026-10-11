"""
Interactive viewer for a GMNS DuckDB — a single self-contained HTML page (no server, no tiles) that
reads an exported GMNS `.duckdb` and lets you explore it: a **layer toggle** (individual *lanes*
offset by use, vs the *mesoscopic* section + turn-connector network), **hover tooltips** naming every
line (its type, id and attributes), and pan/zoom. Built to make the GMNS output legible — the two
geometric levels (offset lanes vs centerline meso) are separate layers, never confusingly stacked.

    from duckosm.gmns_viewer import write_viewer
    write_viewer("sodermalm_gmns.duckdb", "reports/viewer.html", mode="driving")

Needs only ``duckdb`` (+ the spatial extension) — geometry is drawn client-side on a canvas. The meso
layer appears only if the db has a ``meso_<mode>`` schema (built with ``duckosm gmns --meso``).
"""
import json
import logging
import math
from pathlib import Path

logger = logging.getLogger("duckosm")

_Q = 8192
_USE = {"auto": 0, "bus": 1, "bus,bike": 1, "bike": 2}


def _table(con, schema, table):
    return con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name = ? AND table_name = ?",
                       [schema, table]).fetchone()[0] > 0


def _coords(wkt):
    body = wkt[wkt.index("(") + 1:wkt.rindex(")")]
    return [tuple(float(v) for v in p.split()[:2]) for p in body.split(",")]


def build_viewer_payload(con, mode="driving"):
    """Read gmns_<mode> (lane) + meso_<mode> (meso_link/meso_node) and return the JSON payload the
    page renders. Raises ValueError if there's no gmns_<mode> network."""
    g, m = f"gmns_{mode}", f"meso_{mode}"
    if not _table(con, g, "lane"):
        raise ValueError(f"no {g}.lane table — is this a GMNS db with mode '{mode}'?")
    try:
        con.execute("INSTALL spatial; LOAD spatial;")
    except Exception:
        pass

    lanes = con.execute(f"SELECT link_id, lane_num, allowed_uses, turn, ST_AsText(geom) "
                        f"FROM {g}.lane WHERE geom IS NOT NULL").fetchall()
    have_meso = _table(con, m, "meso_link")
    secs = cons = []
    mnodes = []
    if have_meso:
        secs = con.execute(f"SELECT link_id, macro_link_id, lanes, facility_type, ST_AsText(geom) "
                           f"FROM {m}.meso_link WHERE meso_type='normal' AND geom IS NOT NULL").fetchall()
        cons = con.execute(f"SELECT link_id, mvmt_txt_id, lanes, start_ib_lane, end_ib_lane, "
                           f"ST_AsText(geom) FROM {m}.meso_link WHERE meso_type='movement' "
                           f"AND geom IS NOT NULL").fetchall()
        mnodes = con.execute(f"SELECT ST_AsText(geom) FROM {m}.meso_node").fetchall()
    mi = f"micro_{mode}"
    have_micro = _table(con, mi, "micro_link")
    micro = []
    if have_micro:
        micro = con.execute(f"SELECT cell_type, mvmt_txt_id, ST_AsText(geom) FROM {mi}.micro_link "
                            f"WHERE geom IS NOT NULL").fetchall()
    cnt = dict(con.execute(f"SELECT allowed_uses, count(*) FROM {g}.lane GROUP BY 1").fetchall())

    # a lane-rich junction to open on (prefer a bus-lane / multi-lane approach)
    focus = con.execute(f"""SELECT n.x_coord, n.y_coord FROM {g}.node n
        JOIN {g}.link l ON l.to_node_id = n.node_id
        WHERE n.node_id IN (SELECT node_id FROM {g}.movement)
        ORDER BY (l.link_id IN (SELECT link_id FROM {g}.lane WHERE allowed_uses='bus')) DESC,
                 l.lanes DESC, l."length" DESC LIMIT 1""").fetchone() if _table(con, g, "movement") else None

    allc = [p for *_, w in lanes for p in _coords(w)]
    xs = [p[0] for p in allc]; ys = [p[1] for p in allc]
    minx, maxx, miny, maxy = min(xs), max(xs), min(ys), max(ys)
    kx = math.cos(math.radians((miny + maxy) / 2)) or 1.0
    scale = _Q / max((maxx - minx) * kx, (maxy - miny), 1e-9)
    qx = lambda x: round((x - minx) * kx * scale)
    qy = lambda y: round((maxy - y) * scale)
    gw, gh = round((maxx - minx) * kx * scale), round((maxy - miny) * scale)

    def flat(w):
        o = []
        for x, y in _coords(w):
            o += [qx(x), qy(y)]
        return o

    lane_feats = [{"t": "lane", "u": _USE.get(u, 0), "id": str(lid), "n": ln, "tr": tr or "",
                   "g": flat(w)} for lid, ln, u, tr, w in lanes]
    sec_feats = [{"t": "section", "id": sid, "m": str(mid), "la": la, "f": fac or "", "g": flat(w)}
                 for sid, mid, la, fac, w in secs]
    con_feats = [{"t": "conn", "id": cid, "mv": mv or "", "la": la, "s": sib, "e": eib, "g": flat(w)}
                 for cid, mv, la, sib, eib, w in cons]
    mnode_arr = [[qx(x), qy(y)] for x, y in (_coords(w)[0] for (w,) in mnodes)]
    # micro: cells + lane-change drawn from flat coords (no hover); turn connectors hoverable
    mi_cells = [flat(w) for ct, mv, w in micro if ct == "normal"]
    mi_lchg = [flat(w) for ct, mv, w in micro if ct == "lane_change"]
    mi_conns = [{"t": "mconn", "mv": mv or "", "g": flat(w)} for ct, mv, w in micro if ct == "movement"]
    if not focus:
        focus = ((minx + maxx) / 2, (miny + maxy) / 2)
    fx, fy = qx(focus[0]), qy(focus[1])
    half = round(55 * scale / 111320)

    return {
        "grid": {"w": gw, "h": gh}, "focus": [fx - half, fy - half, fx + half, fy + half],
        "lanes": lane_feats, "sections": sec_feats, "conns": con_feats, "mnodes": mnode_arr,
        "micro_cells": mi_cells, "micro_lchg": mi_lchg, "micro_conns": mi_conns,
        "has_meso": bool(sec_feats or con_feats), "has_micro": bool(micro), "mode": mode,
        "stats": {"lanes": sum(cnt.values()), "bus": cnt.get("bus", 0), "bike": cnt.get("bike", 0),
                  "sec": len(sec_feats), "con": len(con_feats),
                  "mcell": len(mi_cells), "mconn": len(mi_conns), "file": ""},
    }


def render_viewer_html(payload, standalone=True, file_label=""):
    """Render the viewer page. ``standalone`` → a full HTML document; else the body-only content an
    Artifact wrapper expects."""
    payload = dict(payload, stats=dict(payload["stats"], file=file_label))
    title = f"GMNS viewer · {file_label}" if file_label else "GMNS viewer"
    content = "<title>" + title + "</title>\n" + _TEMPLATE.replace(
        "/*__P__*/null", json.dumps(payload, separators=(",", ":")))
    if not standalone:
        return content
    return ('<!doctype html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n'
            '<meta name="viewport" content="width=device-width, initial-scale=1">\n'
            f'<title>{title}</title>\n</head>\n<body>\n{content}\n</body>\n</html>\n')


def write_viewer(gmns_db, out, mode="driving", standalone=True):
    """Build + write the interactive viewer for a GMNS db. Returns the output path."""
    import duckdb

    con = duckdb.connect(str(gmns_db), read_only=True)
    con.execute("INSTALL spatial; LOAD spatial;")
    payload = build_viewer_payload(con, mode=mode)
    con.close()
    Path(out).parent.mkdir(parents=True, exist_ok=True)
    Path(out).write_text(render_viewer_html(payload, standalone=standalone,
                                            file_label=Path(gmns_db).name), encoding="utf-8")
    logger.info(f"GMNS viewer [{mode}]: {payload['stats']['lanes']:,} lanes, "
                f"{payload['stats']['con']:,} connectors -> {out}")
    return out


# ---------------------------------------------------------------------------------------------------
# The page — CSS + markup + a canvas renderer with a layer toggle and hover tooltips, all inlined.
# `/*__P__*/null` is replaced with the JSON payload. Theme-aware chrome; committed-dark map surface.
# ---------------------------------------------------------------------------------------------------
_TEMPLATE = r"""<style>
:root{--bg:#eef1f4;--panel:#fff;--ink:#0f1720;--muted:#5b6b7a;--line:#dce2e8;--accent:#0d7d8c;
--map:#0b1017;--auto:#8aa0b6;--bus:#e8934a;--bike:#5aa0d8;--sec:#9aa7b2;--con:#35c4d4;--nd:#f2c14e;--hi:#ffffff;
--mono:ui-monospace,"SF Mono",Menlo,Consolas,monospace;--sans:system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
@media(prefers-color-scheme:dark){:root{--bg:#0c1116;--panel:#121a22;--ink:#e6edf3;--muted:#8b9aa8;--line:#222c37;--accent:#35c4d4}}
:root[data-theme="light"]{--bg:#eef1f4;--panel:#fff;--ink:#0f1720;--muted:#5b6b7a;--line:#dce2e8;--accent:#0d7d8c}
:root[data-theme="dark"]{--bg:#0c1116;--panel:#121a22;--ink:#e6edf3;--muted:#8b9aa8;--line:#222c37;--accent:#35c4d4}
*{box-sizing:border-box}.lv{font-family:var(--sans);color:var(--ink);background:var(--bg);min-height:100vh;padding:18px;display:flex;flex-direction:column;gap:14px}
.lv h1{margin:0;font-size:19px;font-weight:650;letter-spacing:-.01em}
.eyebrow{font-size:11px;letter-spacing:.14em;text-transform:uppercase;color:var(--muted);font-weight:600}
.sub{font-family:var(--mono);font-size:12.5px;color:var(--muted);margin-top:3px}.sub b{color:var(--ink)}
.tiles{display:flex;flex-wrap:wrap;gap:9px}
.tile{background:var(--panel);border:1px solid var(--line);border-radius:11px;padding:9px 13px;display:flex;flex-direction:column;gap:3px;min-width:92px}
.tile .k{font-size:10px;letter-spacing:.08em;text-transform:uppercase;color:var(--muted);font-weight:600}
.tile .v{font-family:var(--mono);font-size:18px;font-weight:600;font-variant-numeric:tabular-nums}
.tile .v.bus{color:var(--bus)}.tile .v.bike{color:var(--bike)}.tile .v.con{color:var(--con)}
.mapwrap{position:relative;background:var(--map);border:1px solid #1c2530;border-radius:14px;overflow:hidden;flex:1;min-height:560px;display:flex}
canvas{width:100%;height:100%;display:block;cursor:crosshair;touch-action:none}canvas.drag{cursor:grabbing}
.seg{position:absolute;left:12px;top:12px;display:inline-flex;background:rgba(10,15,22,.72);border:1px solid rgba(255,255,255,.12);border-radius:9px;padding:3px;backdrop-filter:blur(6px)}
.seg button{font-family:var(--sans);font-size:12px;font-weight:600;color:#c4d0da;background:transparent;border:0;padding:5px 12px;border-radius:6px;cursor:pointer}
.seg button:hover{color:#fff}.seg button[aria-pressed="true"]{background:var(--accent);color:#04222a}
.legend{position:absolute;left:12px;top:56px;background:rgba(10,15,22,.72);border:1px solid rgba(255,255,255,.12);border-radius:10px;padding:9px 12px;backdrop-filter:blur(6px);display:flex;flex-direction:column;gap:5px}
.legend .h{font-size:10px;letter-spacing:.12em;text-transform:uppercase;color:#8ea0ad;font-weight:700}
.legend .r{display:flex;align-items:center;gap:8px;font-size:12px;color:#c9d5de}
.legend .sw{width:16px;height:3px;border-radius:2px}.legend .sw.dot{width:9px;height:9px;border-radius:50%;background:var(--nd)}
.tip{position:absolute;pointer-events:none;background:rgba(12,18,25,.95);border:1px solid rgba(255,255,255,.16);border-radius:8px;
padding:7px 10px;font-family:var(--mono);font-size:11.5px;color:#e6edf3;max-width:300px;display:none;z-index:5;box-shadow:0 6px 20px rgba(0,0,0,.4)}
.tip .h{font-weight:700;margin-bottom:3px;font-family:var(--sans);font-size:12px}.tip .m{color:#8ea0ad;word-break:break-all}
.foot{position:absolute;right:12px;bottom:11px;font-family:var(--mono);font-size:11px;color:#8ea0ad;background:rgba(10,15,22,.66);border:1px solid rgba(255,255,255,.1);border-radius:8px;padding:5px 9px;backdrop-filter:blur(6px)}
.hint{position:absolute;right:12px;top:12px;font-size:11px;color:#8ea0ad;background:rgba(10,15,22,.66);border:1px solid rgba(255,255,255,.1);border-radius:8px;padding:5px 9px}
@media(prefers-reduced-motion:reduce){*{transition:none!important}}
</style>
<div class="lv">
  <div><div class="eyebrow">GMNS DuckDB · interactive viewer</div>
    <h1>Lanes &amp; mesoscopic network — hover any line</h1>
    <div class="sub">reading <b id="f"></b> · mode <b id="md"></b> · toggle layers; hover a line for its id &amp; attributes</div></div>
  <div class="tiles" id="tiles"></div>
  <div class="mapwrap">
    <canvas id="c"></canvas>
    <div class="seg" id="seg" role="group" aria-label="Layer">
      <button data-l="lanes" aria-pressed="false">Lanes</button>
      <button data-l="meso" aria-pressed="true">Meso</button>
      <button data-l="micro" data-micro="1" aria-pressed="false">Micro</button></div>
    <div class="legend" id="legend"></div>
    <div class="hint">scroll zoom · drag pan · hover = info</div>
    <div class="foot"><b id="z">1.0×</b></div>
    <div class="tip" id="tip"></div>
  </div>
</div>
<script>
const D=/*__P__*/null;(function(){
const $=i=>document.getElementById(i),fN=n=>n.toLocaleString('en-US');
$('f').textContent=D.stats.file;$('md').textContent=D.mode;
const T=[['Lanes',fN(D.stats.lanes),''],['Bus lanes',fN(D.stats.bus),'bus'],['Bike lanes',fN(D.stats.bike),'bike']];
if(D.has_meso)T.push(['Meso sections',fN(D.stats.sec),''],['Connectors',fN(D.stats.con),'con']);
if(D.has_micro)T.push(['Micro cells',fN(D.stats.mcell),''],['Turn cells',fN(D.stats.mconn),'con']);
$('tiles').innerHTML=T.map(([k,v,c])=>`<div class="tile"><span class="k">${k}</span><span class="v ${c}">${v}</span></div>`).join('');
const cv=$('c'),ctx=cv.getContext('2d'),tip=$('tip');
const LCOL=['#8aa0b6','#e8934a','#5aa0d8'];let dpr=Math.min(window.devicePixelRatio||1,2),v={s:1,tx:0,ty:0};
let layer=D.has_meso?'meso':'lanes',hover=null;
if(!D.has_meso&&!D.has_micro)$('seg').style.display='none';
[...$('seg').children].forEach(b=>{if(b.dataset.l==='meso'&&!D.has_meso)b.style.display='none';
 if(b.dataset.micro&&!D.has_micro)b.style.display='none';});
const mesoFeats=D.sections.concat(D.conns);
const cssVar=n=>getComputedStyle(document.documentElement).getPropertyValue(n).trim();
const X=x=>(x*v.s+v.tx)*dpr,Y=y=>(y*v.s+v.ty)*dpr;

function legend(){let L,title;
 if(layer==='lanes'){L=[['--auto','auto'],['--bus','bus (psv)'],['--bike','bike']];title='Lane · allowed use';}
 else if(layer==='micro'){L=[['--sec','lane cell'],['#5f6f7d','lane-change'],['--con','turn connector']];title='Micro (cell-based)';}
 else{L=[['--sec','section (per macro link)'],['--con','movement connector (turn)']];title='Meso links';}
 const sw=c=>c[0]==='#'?c:'var('+c+')';
 $('legend').innerHTML='<div class="h">'+title+'</div>'
 +L.map(([c,t])=>`<div class="r"><span class="sw" style="background:${sw(c)}"></span>${t}</div>`).join('')
 +(layer==='meso'?'<div class="r"><span class="sw dot"></span>meso node</div>':'');}

function poly(g){ctx.moveTo(X(g[0]),Y(g[1]));for(let i=2;i<g.length;i+=2)ctx.lineTo(X(g[i]),Y(g[i+1]));}
function draw(){ctx.setTransform(1,0,0,1,0,0);ctx.fillStyle=cssVar('--map')||'#0b1017';ctx.fillRect(0,0,cv.width,cv.height);
ctx.lineCap='round';ctx.lineJoin='round';const lw=Math.max(.8,Math.min(4,v.s*.9))*dpr;
if(layer==='lanes'){const bk=[[],[],[]];for(const f of D.lanes)bk[f.u].push(f);
 bk.forEach((arr,u)=>{if(!arr.length)return;ctx.strokeStyle=LCOL[u];ctx.lineWidth=lw;ctx.beginPath();for(const f of arr)poly(f.g);ctx.stroke();});
}else if(layer==='micro'){
 ctx.strokeStyle='#5f6f7d';ctx.lineWidth=lw*.7;ctx.beginPath();for(const g of D.micro_lchg)poly(g);ctx.stroke();
 ctx.strokeStyle=cssVar('--sec');ctx.lineWidth=lw;ctx.beginPath();for(const g of D.micro_cells)poly(g);ctx.stroke();
 ctx.strokeStyle=cssVar('--con');ctx.lineWidth=lw*1.2;ctx.beginPath();for(const f of D.micro_conns)poly(f.g);ctx.stroke();
}else{ctx.strokeStyle=cssVar('--sec');ctx.lineWidth=lw;ctx.beginPath();for(const f of D.sections)poly(f.g);ctx.stroke();
 ctx.strokeStyle=cssVar('--con');ctx.lineWidth=lw*1.15;ctx.beginPath();for(const f of D.conns)poly(f.g);ctx.stroke();
 ctx.fillStyle=cssVar('--nd');const rad=Math.max(.8,Math.min(2.6,v.s*.5))*dpr;
 for(const n of D.mnodes){ctx.beginPath();ctx.arc(X(n[0]),Y(n[1]),rad,0,6.29);ctx.fill();}}
if(hover){ctx.strokeStyle=cssVar('--hi')||'#fff';ctx.lineWidth=lw*1.8+dpr;ctx.beginPath();poly(hover.g);ctx.stroke();}
$('z').textContent=v.s.toFixed(2).replace(/0$/,'')+'×';}

function ds2(px,py,x1,y1,x2,y2){const dx=x2-x1,dy=y2-y1,l2=dx*dx+dy*dy;let t=0;
 if(l2>0){t=((px-x1)*dx+(py-y1)*dy)/l2;t=t<0?0:t>1?1:t;}const cx=x1+t*dx,cy=y1+t*dy,ex=px-cx,ey=py-cy;return ex*ex+ey*ey;}
function hit(mx,my){const gx=(mx-v.tx)/v.s,gy=(my-v.ty)/v.s,th=(9/v.s)*(9/v.s);let best=null,bd=th;
 const feats=layer==='lanes'?D.lanes:layer==='micro'?D.micro_conns:mesoFeats;
 for(const f of feats){const g=f.g;for(let i=0;i<g.length-2;i+=2){const d=ds2(gx,gy,g[i],g[i+1],g[i+2],g[i+3]);if(d<bd){bd=d;best=f;}}}
 return best;}
function showTip(f,mx,my){if(!f){tip.style.display='none';return;}let h='',m='';
 if(f.t==='lane'){h='Lane · '+['auto','bus','bike'][f.u];m='link '+f.id+'<br>lane '+f.n+(f.tr?' · turn '+f.tr:'');}
 else if(f.t==='section'){h='Meso section';m=f.id+'<br>macro '+f.m+'<br>'+f.la+' lane(s) · '+(f.f||'—');}
 else if(f.t==='mconn'){h='Micro turn cell';m='movement '+(f.mv||'?');}
 else{h='Turn connector';m=(f.mv||'?')+' · '+f.la+' lane(s)'+(f.s!=null?'<br>ib lanes '+f.s+'–'+f.e:'')+'<br>'+f.id;}
 tip.innerHTML='<div class="h">'+h+'</div><div class="m">'+m+'</div>';tip.style.display='block';
 const r=cv.getBoundingClientRect();let tx=mx+14,ty=my+14;if(tx>r.width-200)tx=mx-tip.offsetWidth-14;if(ty>r.height-70)ty=my-tip.offsetHeight-14;
 tip.style.left=tx+'px';tip.style.top=ty+'px';}

function resize(){const r=cv.getBoundingClientRect();cv.width=Math.max(1,r.width*dpr);cv.height=Math.max(1,r.height*dpr);fitBox(D.focus);}
function fitBox(b){const r=cv.getBoundingClientRect(),pad=30,bw=b[2]-b[0],bh=b[3]-b[1];
 const s=Math.min((r.width-2*pad)/bw,(r.height-2*pad)/bh);v.s=s;v.tx=r.width/2-((b[0]+b[2])/2)*s;v.ty=r.height/2-((b[1]+b[3])/2)*s;draw();}

$('seg').addEventListener('click',e=>{const b=e.target.closest('button');if(!b)return;layer=b.dataset.l;
 [...$('seg').children].forEach(x=>x.setAttribute('aria-pressed',x===b));hover=null;showTip(null);legend();draw();});

let drag=null;
cv.addEventListener('pointerdown',e=>{drag={x:e.clientX,y:e.clientY,tx:v.tx,ty:v.ty};cv.classList.add('drag');cv.setPointerCapture(e.pointerId);showTip(null);});
cv.addEventListener('pointermove',e=>{const r=cv.getBoundingClientRect(),mx=e.clientX-r.left,my=e.clientY-r.top;
 if(drag){v.tx=drag.tx+(e.clientX-drag.x);v.ty=drag.ty+(e.clientY-drag.y);draw();return;}
 const h=hit(mx,my);if(h!==hover){hover=h;draw();}showTip(h,mx,my);});
const end=()=>{drag=null;cv.classList.remove('drag');};
cv.addEventListener('pointerup',end);cv.addEventListener('pointercancel',end);
cv.addEventListener('pointerleave',()=>{showTip(null);if(hover){hover=null;draw();}});
cv.addEventListener('wheel',e=>{e.preventDefault();const r=cv.getBoundingClientRect(),mx=e.clientX-r.left,my=e.clientY-r.top,
 k=Math.exp(-e.deltaY*.0016),ns=Math.min(800,Math.max(.05,v.s*k));v.tx=mx-(mx-v.tx)*(ns/v.s);v.ty=my-(my-v.ty)*(ns/v.s);v.s=ns;draw();},{passive:false});
legend();new ResizeObserver(resize).observe(cv);resize();})();
</script>"""
