# A place for the users: questions and feature requests

Status: proposal, for Kaveh's decision. Nothing is enabled or changed on GitHub or on any other service.

## 1. The problem

duckOSM, duckOSM Studio and roadstyle have one maintainer. The users are OSMnx users, GIS users, traffic
simulation and planning users, and ML users. Today, a user can only open an issue. An issue is for a bug.
A user has no place to ask a question, show a result or ask for a feature.

The place must:

- Be easy to find from the manual, the README, the notebooks, the Python package and the Studio page.
- Need little work from one maintainer.
- Be visible to search engines, so that one answer helps many users.
- Connect a feature request to an issue, a pull request and a release.
- Work well with few users. An empty forum tells a new user that the project is dead.

## 2. What similar projects use

Data from the GitHub API on 2026-11-20. "Threads" is the number of GitHub Discussions threads. "Answered" is the
number of Q&A threads with a marked answer.

| Project | Stars | Main place for users | Threads | Answered | Last thread |
|---|---:|---|---:|---:|---|
| OSMnx (one maintainer) | 5 906 | Questions on Stack Overflow. Issues only for bugs and feature proposals (issue form). Questions in issues are closed. | off | – | – |
| QuackOSM | 382 | GitHub Discussions | 5 | 4 | 2025-07 |
| pyrosm | 444 | GitHub Discussions | 0 | 0 | – |
| pandana | 434 | Issues only | off | – | – |
| vedo (one main maintainer) | 2 274 | GitHub Discussions | 169 | 121 | – |
| GeoPandas | 5 305 | GitHub Discussions. The issue chooser links to it. | 140 | 34 | 2026-02 |
| kepler.gl | 12 102 | GitHub Discussions | 40 | 10 | – |
| OSRM | 8 232 | GitHub Discussions and Discord | 91 | 12 | – |
| MapLibre GL JS | 12 142 | GitHub Discussions and the OSM US Slack (#maplibre) | 542 | 115 | 2026-11 |
| DuckDB | 43 210 | GitHub Discussions and Discord. Feature requests go to the "Ideas" category, not to issues. | 3 641 | 398 | 2026-11 |
| SUMO | 4 289 | Eclipse mailing lists (sumo-user, sumo-announce) and issues with a question form | off | – | – |

What this shows:

1. **GitHub Discussions is the standard.** 8 of 11 projects use it. All use the default categories: Announcements,
   General, Ideas, Polls, Q&A, Show and tell.
2. **Small projects get few threads.** QuackOSM has 5 threads and pyrosm has 0. With few users, most users open an
   issue. The channel does not make the users. The links to the channel do.
3. **Chat is for big teams.** Only projects with many maintainers (DuckDB, OSRM, MapLibre) have Discord or Slack.
   Chat needs fast answers, and the answers are not visible to search engines.
4. **A strict issue tracker saves time.** OSMnx (one maintainer) and DuckDB turn off blank issues and send questions
   and ideas to another place. The issue list then holds only work.
5. **One maintainer can answer most questions.** vedo has one main maintainer and 121 of 169 questions have a marked
   answer. A marked answer stays useful for the next user.

## 3. The options

| Option | Cost | Setup | Upkeep (one maintainer) | Search engines | Sign-in | Request → issue → PR | Fit for few users |
|---|---|---|---|---|---|---|---|
| **GitHub Discussions on Khoshkhah/duckOSM** | Free | 15 min: one setting, categories, one pinned post | Low. Same notifications and same place as issues | Yes | GitHub account | Built in: "Create issue from discussion", links by `#number` | Good. Lives next to the code, so it does not look empty |
| GitHub Discussions, one for the organization | Free | Needs a GitHub organization. The repositories are under a user account now | Same as above | Yes | GitHub account | Same | Bad now: a move to an organization changes the Pages URLs (manual, Colab badges) |
| Discourse, free hosting for open source | Free if the project qualifies. The offer is for popular projects ([discourse.org/free](https://discourse.org/free)). Not checked if duckOSM qualifies | 1–2 h, an application, a wait | Medium. A second site, second moderation, second notifications | Yes | Own account, or GitHub sign-in | By hand: copy a link into an issue | Bad. An empty forum looks dead |
| Discourse, self-hosted | About 5–20 USD a month for a server, plus mail | Half a day | High. Updates, backups, mail delivery, spam | Yes | Own account, or GitHub sign-in | By hand | Bad |
| Discord | Free | 30 min | High. Users expect a fast answer. Spam bots | No | Discord account | By hand | Bad. Answers are lost in the chat |
| Matrix (matrix.org, Element) | Free | 30 min | Medium to high, same as Discord | No (mostly) | Matrix account | By hand | Bad |
| Mailing list (Google Groups) | Free | 30 min | Medium | Partly | Google account or e-mail | By hand | Medium. Old style for Python and GIS users |
| OpenStreetMap community forum (community.openstreetmap.org) | Free | None for the forum. One post per release | Low, if used only for announcements | Yes | OSM account | By hand | Good as an **extra**: the OSM users are there. Bad as the main place: the forum is for OSM, not for the support of one tool |
| Feature board: Fider (self-hosted) or Canny (hosted) | Fider: a server. Canny: a free plan with limits (not checked in detail) | 1–2 h | Medium. A third place to watch | Partly | Own account | By hand | Bad. Votes need many users |
| Feature board: GitHub issue forms and reactions (👍) | Free | 30 min: two YAML files | Low | Yes | GitHub account | Built in | Good. Sort issues by 👍 when you plan a release |

## 4. Recommendation

**Main channel: GitHub Discussions on Khoshkhah/duckOSM, for all three projects.**

- It is free. It needs no new site, no new account for Kaveh and no new notifications.
- It is next to the issues, so a request becomes an issue with one click.
- Search engines find the answers.
- Most target users (Python, GIS, ML) have a GitHub account.
- One place for duckOSM, duckOSM Studio and roadstyle is easier to find and does not look empty. The issue
  chooser of duckosm-studio and roadstyle links to it. An issue made from a discussion can move to
  duckosm-studio or roadstyle ("Transfer issue"), because all three repositories have the same owner.

**Extra (one): the OpenStreetMap community forum, for announcements only.** One post for each release, with a link
to the Discussions. The OSM users are there, and this costs a few minutes for each release. Each post is an
outward action and needs Kaveh's yes.

**Not recommended now:** Discord, Matrix, Slack, Discourse and feature boards. They are a second place to watch,
and they need more users than the project has now. Do this check again when the Discussions get more than about
30 threads in one month.

### 4.1 Categories

Keep few categories. Delete "General" and "Polls": a user with few choices chooses correctly.

| Category | Format | For |
|---|---|---|
| 📣 Announcements | Announcement (only Kaveh can post) | Releases, breaking changes |
| 🙏 Q&A | Question and answer | "How do I …?", "Why is this edge missing?" |
| 💡 Ideas | Open discussion | Feature requests. Users vote with 👍 |
| 🙌 Show and tell | Open discussion | Maps, studies, papers made with the tools |

Labels on discussions: `duckosm`, `studio`, `roadstyle`, `python-sdk`.

### 4.2 Issue forms

Add `.github/ISSUE_TEMPLATE/` to duckOSM (and the same `config.yml` to duckosm-studio and roadstyle):

- `config.yml`: `blank_issues_enabled: false`. Contact links: "Ask a question" → Discussions Q&A,
  "Request a feature" → Discussions Ideas, "Read the manual" → the manual.
- `bug_report.yml`: the duckOSM version (`duckosm --version`), the area or a small OSM file, the command or code,
  the output, what you expected.
- `feature.yml`: only for an idea that Kaveh accepted. It links to the discussion. Most users never use this form.

### 4.3 From a request to a release

1. A user opens a thread in **Ideas**.
2. Kaveh replies within one week (the weekly routine). He asks for the use case, if it is not clear.
3. Kaveh decides:
   - **Yes:** "Create issue from discussion". The issue gets the label `feature` and, if known, a milestone.
     The discussion gets a link to the issue.
   - **Not now:** a reply with the reason. The thread stays open for votes.
   - **No:** a reply with the reason, and close the thread.
4. The pull request says `Closes #<issue>`.
5. The CHANGELOG line links to the issue. The release notes thank the user.
6. Kaveh posts the release link in the discussion, and the thread closes.

When Kaveh plans a release, he sorts the Ideas threads and the `feature` issues by 👍.

### 4.4 Links to the channel

| Where | What to add |
|---|---|
| Manual (https://khoshkhah.github.io/duckOSM/) | A short page "Get help" (Q&A, Ideas, bugs). A "Discussions" icon in `extra.social` (the footer of every page). A line on the home page. |
| README | A section "Get help and ask for features" with the three links. Also in the README of duckosm-studio and roadstyle. |
| Notebooks | A last Markdown cell: "Questions or ideas? Ask in the Discussions." with the link. |
| `pyproject.toml` | `Discussions = "https://github.com/Khoshkhah/duckOSM/discussions"` in `[project.urls]`. PyPI then shows the link. |
| Python package | A footer on unexpected errors (not on `MissingDependency`, which already says what to do): "If this is a bug, report it at …/issues". Optional: `duckosm.feedback()`, which opens the Discussions page in the browser. |
| Command line | The same footer in the `duckosm --help` text. |
| duckOSM Studio | A "Feedback" link in the page, next to the manual link. |

### 4.5 Weekly routine for Kaveh (about 30 minutes)

Set GitHub notifications for Khoshkhah/duckOSM to "Custom: Issues, Pull requests, Discussions".
Then once a week:

1. Open the **unanswered Q&A** threads. Answer them, or ask for the missing data. Mark the answer.
2. Open the **new Ideas** threads. Reply with yes, not now or no (section 4.3).
3. Open the **new issues**. Move a question to the Discussions ("Convert to discussion").
4. If a release was made, post it in **Announcements** (and, with your yes, on the OSM forum).

Claude can prepare step 1 to 3 as a list with draft answers. Kaveh reads and posts.

## 5. Setup steps (only after Kaveh says yes)

| # | Step | Who |
|---|---|---|
| 1 | Turn on Discussions on Khoshkhah/duckOSM (Settings → Features → Discussions). | Kaveh |
| 2 | Set the four categories, delete General and Polls, add the labels. | Kaveh, or Claude with `gh` after a yes |
| 3 | Pin a welcome post in Announcements: what goes where, the answer time. | Kaveh posts a draft from Claude |
| 4 | Add the issue forms and `config.yml` to duckOSM, duckosm-studio and roadstyle. | Claude, one PR for each repository |
| 5 | Add the links: manual page, footer, README, notebooks, `pyproject.toml`, Studio. | Claude, one PR for each repository |
| 6 | Add the error footer and, if wanted, `duckosm.feedback()`. | Claude, one PR |
| 7 | Add `CODE_OF_CONDUCT.md` (Contributor Covenant). | Claude, one PR |
| 8 | Post the first release announcement on the OSM forum. | Kaveh |

## 6. Questions for Kaveh

1. Do you agree with **one place on Khoshkhah/duckOSM** for all three projects? Or do you want Discussions on
   duckosm-studio too, for Studio users?
2. Some GIS and planning users have no GitHub account. Do you want a second way for them (for example, an e-mail
   address or the OSM forum)? An e-mail address makes your address public.
3. Who decides the features: the 👍 votes, or your own plan? Do you want a public road map (a GitHub Project board)?
4. What answer time do you promise in the welcome post: one week, or "best effort"?
5. Do you want a release post on the OSM community forum? (Each post needs your yes.)
6. Do you want the error footer and `duckosm.feedback()` in the Python package?
7. Do you accept the Contributor Covenant as the code of conduct?
8. Do you plan to move the repositories to a GitHub organization later? This changes the Pages URLs, so it is
   easier before the links go into many places.

## Sources

- GitHub API (`gh api graphql`, 2026-11-20): Discussions state, threads, categories and issue forms of the projects in section 2.
- OSMnx `CONTRIBUTING.md` and issue forms; SUMO README (mailing lists); DuckDB, OSRM and MapLibre READMEs (Discord, Slack).
- Discourse free plans: https://discourse.org/free and https://blog.discourse.org/2018/11/free-hosting-for-open-source-v2/
