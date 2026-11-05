# Where the DuckDB files live

Date: 2026-10-29. Status: a proposal. Nothing in this note is built. No data was changed, no file was uploaded, and no account was made.

The question: where do the DuckDB files of the Stockholm stack live, how do they update, and how does each project read them? MotherDuck is the first option. This note compares it with three others.

## Summary

Recommendation: **keep DuckDB for building, and share Parquet.** A `.duckdb` file is a build product. It stays on the machine that builds it. What other projects and browsers read is a set of Parquet and CSV files in a Cloudflare R2 bucket, and the hand-made decisions stay in git. Do not use MotherDuck now. It adds a store, and it removes none of the stores that you need anyway.

Three facts decide this:

1. **A `.duckdb` file is tied to one DuckDB version.** The files here use DuckDB 1.5.2. DuckDB 2.0 (2026-10-21) writes a new storage format by default, and older versions cannot read it. Each reader (MotherDuck, DuckDB-Wasm, the Oracle VM, this PC) has its own version. Parquet has no such tie.
2. **Remote reading works, and it is cheap.** A test on 2026-10-29 attached the 511 MB Stockholm county file over HTTP (read-only, byte ranges). The attach read 2.1 MB. `count(*)` over 456,813 edges read 4.2 MB more. A lookup of one edge read 0.8 MB.
3. **The daily page needs static files anyway.** The site is on Cloudflare Pages, and a deploy replaces the whole site. The day files can go to R2 once, and the page can read them from there.

## What the stack needs

- The level editor and the data must not depend on this PC or on the Oracle VM.
- A browser must read the files (the daily page, the online level editor).
- The nightly job on the Oracle VM (1 CPU, less than 1 GB of RAM) must write one day of new files, and nothing larger.
- A person makes hand edits (`edits.csv`) in the level editor. Those edits must not be lost.
- The `spatial` extension must work where a query needs it.

## Data inventory

"Rebuild" says whether another machine can make the file again from other files. Sizes were measured on this PC on 2026-10-29.

| File | Written by | Read by | Size | Rebuild |
|---|---|---|---|---|
| `duckOSM/data/db/<area>.duckdb` (roads, `edge_graph`, `visualization.edge_levels`, `edge_levels_meta`) | `duckosm build`, `duckosm levels` | roadstyle, mapstyle, sensor-matching `adapters.network`, SonoFlow `adapters/build_inputs.py`, the level editor | Södermalm 37 MB, Norrmalm 37 MB, Nacka 10 MB, Sundbyberg 60 MB, Stockholm county 511 MB, Tartu 155 MB, Vancouver city 681 MB, Estonia 2.4 GB | Yes, from the `.osm.pbf` file. `edge_id` is a content hash, so the ids do not change. The edit files below are not part of the rebuild |
| `duckOSM/data/db/<area>.levels/`: `edits.csv`, `heads.csv`, `caps.csv` | the level editor, a person | `roadstyle-levels`, `duckosm levels` | 1 KB each | **No.** These are decisions |
| the same folder: `levels.csv`, `pairs.csv`, `roads.parquet`, `levels_info.json` | `roadstyle-levels` | the level editor | 0.5 to 1.2 MB each | Yes, from the area file and the edits |
| `SonoFlow/areas/<area>/levels.duckdb` (`visualization.edge_levels`, `edge_levels_meta`) | roadstyle 0.17.1 (the meta row says so), copied from the area file | SonoFlow viz pages | 780 KB for Södermalm and Norrmalm. Nacka has none | Yes |
| `sensor-matching/areas/<area>/`: `network.parquet`, `movements.parquet`, `boundary.parquet`, `calendar.parquet`, `priors.parquet`, `levels.parquet` | sensor-matching adapters | SonoFlow, the daily page | 0.6 MB in total for Södermalm | Yes, from the area file |
| `sensor-matching/areas/<area>/observations/`: `history.parquet`, one file for each day | `sensormatch.combine` | SonoFlow (`contract.read_observations`) | 2.8 MB, 138 files of 3 to 8 KB | Yes, if the raw readings exist |
| `sensor-matching/areas/<area>/sources/*/match/`, `overrides.csv` | matchers, a person | `combine` | `sources/` is 147 MB | `overrides.csv`: **No.** `match/`: yes |
| `fetching-sweden-data/data/processed/<area>_traffic.duckdb` (the raw readings) | the traffic poller, `fetching-sweden-data` | sensor-matching adapters, the Oracle VM nightly | Södermalm 122 MB, Nacka 38 MB, Norrmalm 15 MB, Sundbyberg 9 MB | **No.** The poller exists because nobody serves this history. The R2 backup is coded in `deploy/backup_raw.sh`, but `R2_BUCKET` is empty, so no backup runs |
| `SonoFlow/areas/<area>/state/engine/`, `state/model/` | `sonoflow.daily` (nightly) | the next night | 22 MB and 15 MB for Södermalm. The two halves must travel as a pair | Only by a replay of the archive (about 2.75 hours) |
| `state/snapshots/` | `daily.advance` (monthly) | rare | 11 GB for Södermalm | By replay |
| `state/output/daily_<date>.parquet` and `daily_<date>.json.gz` | the nightly | the daily page, the Pages upload | 5.0 MB and 0.53 MB for each day of Södermalm, so about 2 GB for each year | Yes, by replay |
| the published site (`sonoflow`, `sonoflow-nacka`, status projects on Cloudflare Pages) | `docs/deploy_site.py` | people, behind Cloudflare Access | not measured | Yes |

What the table shows:

- Five groups of files exist. Three hold decisions or history that nothing else can rebuild: the hand edits (`edits.csv`, `heads.csv`, `caps.csv`, `overrides.csv`), the raw readings, and the model state.
- Everything that a browser needs is small. The per-area road file is about 0.3 MB, and the editor files are about 3 MB. Only the day files grow.
- Two files that look like copies are not the same format. `SonoFlow/areas/sodermalm/levels.duckdb` has the same five columns and the same hash as the table in `duckOSM/data/db/sodermalm.duckdb`. It was made by roadstyle 0.17.1. The note from the other session says that the current roadstyle cannot read this form. That claim was not tested here.

## Facts that were checked

Pages and limits were read on 2026-10-29. A number that I did not find on an official page is marked "not found".

| Topic | Fact | Source |
|---|---|---|
| MotherDuck free plan (Lite) | $0. 10 GB of storage. 10 hours of Pulse compute each month, then $0.60 for each CU-hour. 3 users. 2 service accounts. Snapshots kept for 1 day | [pricing](https://motherduck.com/product/pricing/) |
| MotherDuck paid plan (Business) | $250 each month for an organization, plus usage. Storage $0.04 for each GB-month. Snapshots kept for up to 90 days | [pricing](https://motherduck.com/product/pricing/) |
| MotherDuck read-only tokens | Read scaling tokens exist (up to 16 replicas). The page does not say which plan includes them | [pricing](https://motherduck.com/product/pricing/), [read scaling](https://motherduck.com/docs/key-tasks/authenticating-and-connecting-to-motherduck/read-scaling/) |
| MotherDuck DuckDB versions | In `us-east-1`, 1.4.0 to 1.5.6. In `eu-central-1` and `eu-west-1`, 1.4.1 to 1.5.6. DuckDB 2.0 is not in this range | [architecture](https://motherduck.com/docs/concepts/architecture-and-capabilities/) |
| MotherDuck `spatial` | Supported in MotherDuck Cloud, the MotherDuck UI (Wasm) and the DuckDB CLI. A footnote says that not all features work in the Wasm UI. Community extensions are not supported | [extensions](https://motherduck.com/docs/concepts/duckdb-extensions.md) |
| MotherDuck in a browser | A Wasm client exists. It needs an `mdToken`. The page gives no advice on a token in public page code, and it says nothing on extensions | [Wasm client](https://motherduck.com/docs/key-tasks/data-apps/wasm-client/) |
| R2 free tier | 10 GB-month of Standard storage, 1 million Class A operations, 10 million Class B operations each month. Egress is free. Page date: 2026-10-26 | [R2 pricing](https://developers.cloudflare.com/r2/pricing/) |
| R2 paid | $0.015 for each GB-month. $4.50 for each million Class A operations. $0.36 for each million Class B operations | [R2 pricing](https://developers.cloudflare.com/r2/pricing/) |
| Cloudflare Pages free plan | 20,000 files for each site. 25 MiB for each file. 500 builds each month. The page states no bandwidth limit | [Pages limits](https://developers.cloudflare.com/pages/platform/limits/) |
| GitHub Pages | A site is at most 1 GB. Soft limits: 100 GB of bandwidth and 10 builds each hour. No file-size limit on the page. The page does not say if a private repository can publish | [limits](https://docs.github.com/en/pages/getting-started-with-github-pages/github-pages-limits) |
| GitHub Pages headers | Measured on 2026-10-29 with `curl`: `access-control-allow-origin: *` and `accept-ranges: bytes` | `khoshkhah.github.io/duckOSM/llms.txt` |
| Remote `.duckdb` | `ATTACH 'https://…' (READ_ONLY)` after `INSTALL httpfs`. Only read-only connections. No write over HTTPS or S3 | [DuckDB guide](https://duckdb.org/docs/2.0/guides/network_cloud_storage/duckdb_over_https_or_s3.html) |
| Storage format | DuckDB 2.0 writes storage version 69 by default. Version 68 is 1.5.x, and 67 is 1.4.x. Older DuckDB cannot read version 69. `STORAGE_VERSION 'v1.2.0'` on `ATTACH` writes an older form | [storage](https://duckdb.org/docs/2.0/internals/storage) |
| DuckDB-Wasm | A remote read needs CORS and HTTPS. It relies on range requests, and these can fail in Firefox. The threaded build needs cross-origin isolation headers | [troubleshoot](https://duckdb.org/docs/current/clients/wasm/troubleshoot) |
| DuckDB-Wasm `spatial` | `INSTALL spatial; LOAD spatial;` works in Wasm. [Issue 2005](https://github.com/duckdb/duckdb-wasm/issues/2005) (an `ST_Read` failure) was fixed in PR 2020. A Firefox error followed. The reporter confirmed on 2025-09-30 that it works. A Wasm build for DuckDB 2.0: not found | the links in this row |

My own measurement, from a script in the scratchpad folder: attach to the Södermalm file read 2.4 MB. Attach to the county file read 2.1 MB. The test used a local server with range support and DuckDB 1.5.2. A real server (R2) adds a round trip for each range. The test did not measure that delay, and it did not use a browser.

## Candidates

### A. Files stay where they are (today)

Files live on this PC and on the Oracle VM. `rsync` moves them. `DEPLOY.md` has the move checklist.

- **Cost:** none.
- **Read access from a browser:** none, except through the published Pages site.
- **Write access:** each machine writes its own copy. A person must copy by hand.
- **Risk:** a lost or off PC stops the level editor. Nacka has no area database or levels. Two copies of one table can differ, as the two `levels.duckdb` files show. The raw readings have three copies, and all three are on running machines.

### B. MotherDuck as the one store

Every area file becomes a MotherDuck database. Projects use `ATTACH 'md:…'`.

- **Cost:** $0 inside the free plan. The area files that are in use (without Estonia and old copies) are about 1.5 GB. The raw readings add 0.2 GB. The 10 GB limit is not a problem. The 10 CU-hours of compute each month is the number to watch. It is not measured here.
- **`spatial`:** supported in the cloud and in the CLI. The Wasm client is not documented for `spatial`.
- **Read access from a browser:** the Wasm client needs a token in the page. A public page then holds a token that anyone can copy. A read scaling token limits what the copy can do, but it still spends your compute hours. The site is behind Cloudflare Access today, so a token could stay behind the login. That ties the data to the login.
- **Write access:** the Oracle VM can write with a service account (2 are free). The level editor in a browser would need a write token. That is the wrong place for one. The editor must commit `edits.csv` to git instead.
- **Risk:**
  - The DuckDB version window (1.4.0 to 1.5.6) does not include 2.0. A move to 2.0 depends on the vendor.
  - The Wasm client is early. Its npm page says that the API can change much.
  - It does not remove any store. The Pages site, the day files and the hand edits still need a static host and git.
  - Snapshots last 1 day on the free plan. It is not a backup.
- **Good for:** SQL from several machines, or from several people, over many areas. That need does not exist yet.

### C. Parquet and CSV in R2, hand decisions in git (recommended)

The unit of exchange is a file that every DuckDB version reads. A bucket holds the files. A manifest names the current set.

```text
r2://<bucket>/
    areas/<area>/<build-id>/        roads.parquet, network.parquet, movements.parquet,
                                    levels.parquet, boundary.parquet, calendar.parquet, priors.parquet
    areas/<area>/latest.json        the build-id that readers use
    areas/<area>/days/<date>.parquet
    areas/<area>/days/<date>.json.gz
    db/<area>-<build-id>.duckdb     optional: the area file, for ATTACH in Python and Rust
    raw/<area>/<YYYY-MM-DD>.duckdb.gz   the backup of the readings (already coded)
git (private repository)
    edits.csv, heads.csv, caps.csv, overrides.csv, roadstyle.json (no key), the manifest of each area
```

- **Cost:** $0 inside 10 GB. The day files of Södermalm are about 2 GB for each year. Beyond 10 GB, storage costs $0.015 for each GB-month, so 20 GB costs $0.15 each month. Reads from a browser are Class B operations (10 million free), and egress is free. A bucket with one custom domain needs no new account, because the Cloudflare account exists (the Pages projects and the Worker are there).
- **`spatial`:** not needed to read GeoParquet as WKB. A query that uses `ST_*` runs in Python or Rust, where the extension is stable. A browser that needs `ST_*` can load it in Wasm (issue 2005 is still open on GitHub, but a reporter confirmed on 2025-09-30 that it works; check the Wasm version).
- **Read access from a browser:** a public bucket with a CORS rule and a custom domain. DuckDB-Wasm reads Parquet by range requests, and `roads.parquet` for one area is about 1 MB. Python reads with `read_parquet('https://…')` or `ATTACH`. A private bucket needs a Worker with the Access service token. The Worker `sonoflow-front-door` already does this for the Pages site.
- **Write access:**
  - The Oracle VM nightly uploads one day (5.5 MB). It uses a token that is scoped to R2 only. `DEPLOY.md` already names this token (`R2_API_TOKEN`, separate from the Pages token).
  - This PC uploads a new area build.
  - A browser never writes to R2.
  - A person edits in the level editor, which commits `edits.csv` to git (see the next section).
- **Risk:**
  - Two writers (this PC and the VM) can overwrite one file. The build-id folder and `latest.json` remove that. A writer makes a new folder and then changes `latest.json`.
  - A public bucket shows the data to everyone. The Trafikverket and Stockholm city data licenses were not read for this note.
  - Parquet has no schema `visualization.edge_levels` as a table. The name moves into a file name and a column list. The contract must say this in one place (see `roadstyle/docs/design/contract_schema.md`).

### D. Git and GitHub Pages for the small files

All small files go to a repository, and GitHub Pages serves them. `sensor-matching` already commits `observations/` (a day is 10 KB).

- **Cost:** none.
- **Limits:** a site is at most 1 GB. The bandwidth soft limit is 100 GB each month. The 5 MB day files of Södermalm reach 1 GB in about 6 months. The day files do not fit. `edits.csv`, `roads.parquet` and the per-area road files do fit.
- **Read access from a browser:** `access-control-allow-origin: *` and range support were measured. This works with no setup.
- **Write access:** a commit. An online editor on GitHub Pages saves `edits.csv` by a commit through the GitHub API. It needs a fine-grained token for one repository. The person keeps the token in the browser.
- **Risk:** the repository must be public to use Pages on a free plan. This was not checked, and `SonoFlow/CLAUDE.md` says that the earlier GitHub Pages site stopped on 2026-09-06 when the repository became private. A public repository shows the edits and the road files.
- **Good for:** the editor and its hand edits. Not for the day files, and not for `.duckdb` files.

## Recommendation

Use C for the data and D for the hand edits. Do not use B now.

| Kind of file | Home | Writer | Reader |
|---|---|---|---|
| Area `.duckdb` (build product) | the machine that builds it. One copy goes to R2 as a backup and for `ATTACH` | this PC | Python and Rust projects |
| Road and contract Parquet (per area) | R2, a new build-id folder for each build | this PC | the Oracle VM, the editor, the daily page |
| Day files | R2 | the Oracle VM nightly | the daily page |
| Hand decisions (`edits.csv`, `heads.csv`, `caps.csv`, `overrides.csv`) | git | a person, through the editor or by hand | the level builds |
| Raw readings | the poller, and the R2 backup that is coded already | the poller | sensor-matching |
| Model state, snapshots | the Oracle VM. A monthly copy of `state/engine/` and `state/model/` (37 MB, a pair) goes to R2 | the nightly | the next night |
| Secrets | `~/.sonoflow.env` on the VM. A Cloudflare token for each job. No secret in a page, in git or in a crontab | | |

Rules:

1. **A `.duckdb` file is never the exchange format between projects.** A project reads the Parquet. Only Python and Rust projects that must run SQL on a whole area `ATTACH` a `.duckdb` file, and only with the DuckDB version that wrote it. The file name has the version: `<area>-<build-id>.duckdb`.
2. **Never overwrite.** A build makes a new `<build-id>` folder. Then it changes `latest.json`. A reader that started before the change finishes with the old folder.
3. **Join on `edge_id` as BIGINT.** Parquet keeps the type, but a JSON sidecar does not. The sidecar already writes `edge_id` as a string (`SonoFlow/CLAUDE.md`). A reader must cast it.
4. **The day file is the only nightly write.** The nightly writes one `days/<date>` pair and updates a small `days/index.json`. It does not redeploy the Pages site. The daily page reads the dates from the index.
5. **Nacka gets an area file first.** `duckOSM/data/db/nacka.duckdb` has `driving.edges` but no `visualization` schema (checked on 2026-10-29), and Nacka has no `.levels` folder. Run `duckosm levels` once, then publish.

Online level editor, on this plan: the page reads `roads.parquet` and `levels.csv` for the area from R2, and `edits.csv` from git. A save is a commit of `edits.csv`. A CI job or the PC then runs the level build, writes `levels.csv` into a new R2 folder, and changes `latest.json`. The browser never holds a write token for R2.

## When to look at B again

MotherDuck becomes worth its cost when two things are true at the same time: more than one person (or machine) asks SQL questions over many areas, and the work can wait for DuckDB 2.0 support. The step is small, because every file in C is Parquet: `CREATE TABLE … AS SELECT * FROM read_parquet('https://…')` loads it.

## Open questions

Ordered by what blocks the next step.

1. **Public or private data?** The bucket can be public (simple, browser reads at once) or private (a Worker with a login). Which one for the road files? Which one for the day files?
2. **Is a public repository for `edits.csv` acceptable?** If not, the online editor needs a private repository and a different host, because GitHub Pages for a private repository was not verified.
3. **Which R2 bucket name and custom domain?** For example `data.khoshkhah.com`. The account has R2 access already (`R2_BUCKET` is empty in `deploy/hosts/sodermalm.env`).
4. **Is the backup of the raw readings first?** It is the only file here that nothing can rebuild, and the code exists. It needs a bucket and one token.
5. **Which areas go to R2 first?** Södermalm, Norrmalm, Nacka and Sundbyberg are 144 MB in total. The county file is 511 MB.
6. **When does the project move to DuckDB 2.0?** The files then use storage version 69. MotherDuck and DuckDB-Wasm may not read it. A published `.duckdb` file would need `STORAGE_VERSION` set, or only Parquet is published.

## What this note did not check

- DuckDB-Wasm for DuckDB 2.0, with `spatial` (not found).
- The speed of a real R2 range read from a browser. Only a local range server was tested.
- The data licenses for a public bucket.
- MotherDuck compute use for the real queries. The free 10 CU-hours may be enough or not.
- The claim that roadstyle cannot read the Södermalm `levels.duckdb` (given by the other session).
- Vocabulary was checked against known rulings and high-risk patterns only, not against the official ASD-STE100 Part 2 dictionary. Full compliance needs verification against the official standard.
