"""Drives reef-app in headless Chromium, over plans put through the `reef` CLI.

Each step checks one of M4's to M10's criteria, or how the app shows a node's work, in the browser, printing `ok` or `FAILED` and what
it saw; the script exits 1 if any failed. C48's sign-in runs last, on a server of its own that asks
every caller to sign in (127.0.0.1:12191, its data under the same directory's `sign-in`), then
C51's, on another that serves over TLS beside it (127.0.0.1:12192 and 12193, under `origin`), with
a page of another origin served from 127.0.0.1:12194, then C77's account page, on a third
(127.0.0.1:12196, under `account`).

It runs everything itself: it builds the CLI, writes a config of its own (the app on
127.0.0.1:12190, shoal's data under central/target/tmp/reef-app-check), registers a scratch repo,
starts the server dx built, and stops the server's whole process group at the end, since an
orphaned server keeps shoal's data directory locked.

Build the app first, from central/reef-app, with no `RUSTFLAGS` exported:

    env -u RUSTFLAGS dx build @server --platform server --features embedded

then, with Playwright installed:

    python check.py [screenshot dir] [--release] [--only=c33,c35,c36,c56,c73]

`--release` runs the server `dx build ... --release` built instead, which the timings in the
plan's figures are taken on. `--only` runs the checks whose names start with one of those given.
"""

import functools
import http.server
import json
import os
import shutil
import signal
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

# reef's checkout, the app's crate, and where dx put the server it built
APP = Path(__file__).resolve().parent
REEF = APP.parent.parent
FLAGS = [arg for arg in sys.argv[1:] if arg.startswith("--")]
ARGS = [arg for arg in sys.argv[1:] if not arg.startswith("--")]
RELEASE = "--release" in FLAGS
ONLY = [name for flag in FLAGS if flag.startswith("--only=") for name in flag[len("--only="):].split(",")]
SERVER = REEF / f"central/target/dx/reef-app/{'release' if RELEASE else 'debug'}/web/server"
# where the check keeps its config, repo and shoal's data: under target, never a tmpfs /tmp
WORK = REEF / "central/target/tmp/reef-app-check"
ADDRESS = "127.0.0.1:12190"
URL = f"http://{ADDRESS}"
SHOTS = Path(ARGS[0]) if ARGS else WORK
# the checks that failed, and everything the page logged
failed = []
logs = []
# the mdbook build of the plan C54 draws, which reef-view draws, for C55 to compare against
BOOK = {}
# C48's server, which asks every caller to sign in, its own config, repo and data, and the
# password its user signs in with
SIGN_IN_WORK = WORK / "sign-in"
SIGN_IN_URL = "http://127.0.0.1:12191"
PASSWORD = "correct horse battery"
# C51's server, over plain HTTP and TLS, and the page of another origin that posts to it
ORIGIN_WORK = WORK / "origin"
ORIGIN_URL = "http://127.0.0.1:12192"
ORIGIN_TLS = "https://127.0.0.1:12193"
ELSEWHERE = "http://127.0.0.1:12194"
# C77's server, whose users and tokens the account page manages
ACCOUNT_WORK = WORK / "account"
ACCOUNT_URL = "http://127.0.0.1:12196"


def check(label, ok, seen):
    """Prints whether a check held and what it saw, and remembers it if it didn't."""
    print(f"{'ok' if ok else 'FAILED'} {label}: {seen}")
    if not ok:
        failed.append(label)


def run(*args, cwd=None):
    """Runs a command, failing loudly if it fails, and returns what it printed."""
    done = subprocess.run(args, cwd=cwd, capture_output=True, text=True)
    if done.returncode != 0:
        raise RuntimeError(f"{' '.join(map(str, args))}: {done.stderr or done.stdout}")
    return done.stdout


def reef(*args, cwd=None):
    """Runs the reef CLI against the check's own config."""
    return run(REEF / "target/debug/reef", "--conf", WORK / "reef.toml", *args, cwd=cwd or WORK / "repo")


def put(plan, yaml):
    """Puts a node's YAML in a plan of the check's project."""
    path = WORK / f"put-{plan}-{time.monotonic_ns()}.yaml"
    path.write_text(yaml)
    return reef("node", "put", str(path), "--plan", plan)


def git(*args):
    """Runs git in the check's repo as its own author, with no one's config."""
    run("git", "-C", WORK / "repo", "-c", "user.name=reef", "-c", "user.email=reef@example.invalid",
        "-c", "init.defaultBranch=main", *args)


def setup():
    """A fresh repo and config, and the CLI built."""
    if WORK.exists():
        shutil.rmtree(WORK)
    (WORK / "repo").mkdir(parents=True)
    git("init", "-q")
    git("commit", "-q", "--allow-empty", "-m", "start")
    (WORK / "reef.toml").write_text(
        f'[api]\naddress = "{ADDRESS}"\n\n[shoal]\ndata = "{WORK / "shoal"}"\ncores = 1\n'
    )
    run("cargo", "build", "-q", "-p", "reef", cwd=REEF)
    run("cargo", "build", "-q", "--release", "-p", "mdbook-reef", cwd=REEF)


def start(conf=None, url=URL, log_path=None):
    """Starts the built server in a process group of its own, and waits for its JSON API.

    It waits on `/api/v1/me`, which answers whether or not the server asks callers to sign in.
    """
    if not SERVER.exists():
        sys.exit(f"no server at {SERVER}: build it with dx first (see this script's docs)")
    conf = conf or WORK / "reef.toml"
    log_path = log_path or WORK / "server.log"
    env = dict(os.environ, REEF_CONFIG=str(conf))
    env.pop("RUSTFLAGS", None)
    log = open(log_path, "w")
    server = subprocess.Popen([SERVER], cwd=SERVER.parent, env=env, stdout=log, stderr=subprocess.STDOUT,
                              start_new_session=True)
    # shoal starts in the process, so the first answer takes a moment
    for _ in range(120):
        try:
            with urllib.request.urlopen(f"{url}/api/v1/me", timeout=1) as answer:
                if answer.status == 200:
                    return server
        except OSError:
            time.sleep(0.5)
    stop(server)
    sys.exit(f"the server never answered; see {log_path}")


def stop(server):
    """Stops the server's whole process group, as a terminal's ctrl-c would, once."""
    # a server stopped already has no group left to stop
    if server.poll() is not None:
        return
    try:
        os.killpg(server.pid, signal.SIGINT)
        server.wait(timeout=15)
    except (ProcessLookupError, subprocess.TimeoutExpired):
        os.killpg(server.pid, signal.SIGKILL)


def node_yaml(node_id, title, extra=""):
    """A node's YAML that says why and what, with anything else it needs after."""
    return f'id: {node_id}\ntitle: "{title}"\ndescription:\n  why: "{title} is missing."\n  what: "{title} exists."\n{extra}'


# a plan that shows most of what a box can: a group, needs, a status, an optional node,
# acceptance criteria and a performance figure, with why and what inline so it reads the same
# from a book's files and from the store
DRAWS = """id: G1
title: "G1 The work"
description:
  why: "The work is not grouped."
  what: "The work is grouped."
nodes:
  - id: W1
    title: "W1 Lay the ground"
    status: done
    description:
      why: "There is no ground."
      what: "There is ground."
    acceptance:
      - unit: The ground holds
        status: met
  - id: W2
    title: "W2 Build on it"
    needs: [W1]
    description:
      why: "Nothing stands."
      what: "Something stands."
    acceptance:
      - feature: It stands
      - kind: performance
        text: It stands in at most 10 ms
        metric: stand-ms
        at-most: 10
    performance:
      metrics:
        - name: Standing, ms
          key: stand-ms
          before: 12
          after: 8
  - id: W3
    title: "W3 Look after it"
    optional: true
    needs: [W2]
    description:
      why: "It decays."
      what: "It lasts."
"""
LATER = node_yaml("L1", "L1 After the work", "needs: [G1]\n")


def boxes(page):
    """Every box and group the diagram draws, as its classes (in any order) and its text, sorted."""
    return page.evaluate(
        "() => [...document.querySelectorAll('.reef-viewport .reef-node, .reef-viewport .reef-group')]"
        ".map(e => [e.getAttribute('class').split(' ').sort().join(' '), e.textContent]).sort()"
    )


def wait_drawn(page):
    """Waits for the viewer to be mounted and to hold boxes."""
    page.wait_for_selector(".reef.reef-mounted .reef-node", timeout=20000)


def book_of(nodes):
    """Builds a scratch mdbook whose plan is the given node files, returning its plan's page."""
    book = WORK / "book"
    (book / "src").mkdir(parents=True)
    mdbook_reef = REEF / "target/release/mdbook-reef"
    (book / "book.toml").write_text(
        f'[book]\ntitle = "draws"\nsrc = "src"\n\n[preprocessor.reef]\ncommand = "{mdbook_reef}"\n\n'
        '[output.html]\nadditional-js = ["reef.js"]\nadditional-css = ["reef.css"]\n'
    )
    for asset in ["reef.js", "reef.css"]:
        shutil.copy(REEF / "crates/mdbook-reef/assets" / asset, book / asset)
    summary = ["# Summary", ""]
    for name, (title, yaml, body) in nodes.items():
        (book / "src" / f"{name}.md").write_text(f"---\n{yaml}---\n# {title}\n{body}")
        summary.append(f"- [{title}]({name}.md)")
    (book / "src/SUMMARY.md").write_text("\n".join(summary) + "\n")
    run("mdbook", "build", cwd=book)
    return book / "book" / f"{next(iter(nodes))}.html"


def c54_draws_as_in_mdbook(page):
    """C54: a plan put through the CLI draws as the same nodes do in mdbook."""
    reef("plan", "new", "draws")
    put("draws", DRAWS)
    put("draws", LATER)
    html = book_of({
        "work": ("The work", DRAWS, '\n```reef\nlayout = "page"\n```\n'),
        "later": ("Later", LATER, ""),
    })
    # the book's page, with every group open
    page.goto(f"file://{html}")
    wait_drawn(page)
    page.click("button[data-act=expand]")
    page.wait_for_timeout(300)
    in_book = (page.get_attribute(".reef", "data-boxes"), boxes(page))
    BOOK["work"] = html
    # the app's, the same way
    page.goto(URL + "/plans/check/draws")
    wait_drawn(page)
    page.click("button[data-act=expand]")
    page.wait_for_timeout(300)
    in_app = (page.get_attribute(".reef", "data-boxes"), boxes(page))
    page.screenshot(path=str(SHOTS / "reef-app-draws.png"))
    check("C54 a plan put through the CLI draws as it does in mdbook", in_book == in_app and len(in_app[1]) == 5,
          {"boxes in the book": in_book[0], "in the app": in_app[0],
           "differ": [b for b in in_book[1] if b not in in_app[1]] + [a for a in in_app[1] if a not in in_book[1]]})


def c54_selecting_opens_the_panel(page):
    """C54: clicking a node in the diagram opens it in the sidebar, and letting it go closes it."""
    page.goto(URL + "/plans/check/draws")
    wait_drawn(page)
    page.click("button[data-act=expand]")
    page.wait_for_timeout(300)
    page.locator(".reef-viewport .reef-node", has_text="W2 Build on it").click()
    page.wait_for_selector(".reef-side", timeout=5000)
    shown = (page.inner_text(".reef-side .reef-id"), page.inner_text(".reef-side h2"))
    page.screenshot(path=str(SHOTS / "reef-app-panel.png"))
    page.click(".reef-side [data-act=close]")
    page.wait_for_timeout(300)
    closed = page.locator(".reef-side").count() == 0
    check("C54 selecting a node opens it in the node panel", shown == ("W2", "W2 Build on it") and closed,
          {"sidebar": shown, "closed": closed})


def c54_one_viewer_listens(page):
    """C58 in the app: leaving a plan for the picker and coming back leaves one canvas drawing."""
    page.goto(URL + "/plans/check/draws")
    wait_drawn(page)
    page.evaluate("() => { window.__old = document.querySelector('.reef'); window.__kept = 'same page';"
                  " window.__oldDraws = window.__old.getAttribute('data-draws'); }")
    page.click("text=← Plans")
    page.wait_for_selector(".reef-app-plans a")
    page.click(".reef-app-plans a:text-is('draws')")
    wait_drawn(page)
    page.wait_for_timeout(300)
    # a filter change lays the live canvas out again, which counts it in `data-draws`
    before = page.get_attribute(".reef", "data-draws")
    chip = page.locator(".reef-acc-filter button").first
    chip.click()
    page.wait_for_timeout(500)
    drew = page.evaluate("() => [window.__old.getAttribute('data-draws') !== window.__oldDraws,"
                         " document.querySelector('.reef').getAttribute('data-draws'),"
                         " window.__old.isConnected, window.__kept]")
    chip.click()
    check("C54 a plan left and opened again leaves one viewer listening",
          drew[0] is False and drew[1] != before and drew[2] is False and drew[3] == "same page",
          {"left view drew": drew[0], "new view's layouts": [before, drew[1]], "left element in the page": drew[2],
           "no reload": drew[3]})


def seen(page):
    """What C55 compares in a view: search, sidebar, chart hover and a group collapsed from its head."""
    wait_drawn(page)
    page.click("button[data-act=expand]")
    page.wait_for_timeout(300)
    out = {}
    # a search for W2, taken with Enter, selects it and opens its sidebar
    page.fill(".reef-search", "W2")
    page.wait_for_timeout(200)
    out["results"] = page.eval_on_selector_all(".reef-results a", "as => as.map(a => a.textContent)")
    page.press(".reef-search", "Enter")
    page.wait_for_selector(".reef-side .reef-id", timeout=5000)
    page.wait_for_timeout(300)
    out["searched"] = (page.inner_text(".reef-side .reef-id"), page.get_attribute(".reef", "data-boxes"), boxes(page))
    # its sidebar: the sections, the criteria, and the charts as drawn, whatever node index they name
    out["sections"] = page.eval_on_selector_all(".reef-side .reef-sec > summary h3", "hs => hs.map(h => h.textContent)")
    out["criteria"] = page.eval_on_selector_all(".reef-side .reef-acc-item", "is => is.map(i => i.textContent)")
    out["charts"] = page.evaluate(
        "() => [...document.querySelectorAll('.reef-side svg.reef-perf-chart')]"
        ".map(s => s.outerHTML.replace(/ data-node=\"[0-9]+\"/, ''))"
    )
    # a hover over the first chart's middle shows the same tooltip
    chart = page.locator(".reef-side svg.reef-perf-chart").first
    chart.scroll_into_view_if_needed()
    chart.hover()
    page.wait_for_timeout(200)
    out["tip"] = page.evaluate("() => { const t = document.querySelector('.reef-tip:not([hidden])'); return t && t.textContent; }")
    page.mouse.move(5, 5)
    # G1's head, clicked, closes it, and the boxes are the same
    page.click(".reef-side [data-act=close]")
    page.wait_for_timeout(200)
    page.locator(".reef-viewport .reef-group .reef-head", has_text="G1 The work").first.click()
    page.wait_for_timeout(300)
    out["collapsed"] = (page.get_attribute(".reef", "data-boxes"), boxes(page))
    return out


def c55_matches_the_viewer(page):
    """C55: the canvas searches, collapses groups and draws charts as reef-view does in mdbook."""
    page.goto(f"file://{BOOK['work']}")
    in_book = seen(page)
    page.goto(URL + "/plans/check/draws")
    in_app = seen(page)
    page.screenshot(path=str(SHOTS / "reef-app-collapsed.png"))
    for what in ["results", "searched", "sections", "criteria", "charts", "tip", "collapsed"]:
        same = in_book[what] == in_app[what] and in_app[what] not in (None, [], "")
        check(f"C55 {what} match the mounted viewer", same,
              {"book": str(in_book[what])[:300], "app": str(in_app[what])[:300]} if not same else str(in_app[what])[:120])
    # the pills' and arrows' tooltips are SVG titles, which a browser shows
    page.click("button[data-act=expand]")
    page.wait_for_timeout(300)
    spaces = page.eval_on_selector_all(".reef-viewport title", "ts => [...new Set(ts.map(t => t.namespaceURI))]")
    check("C55 the boxes' tooltips are SVG titles", spaces == ["http://www.w3.org/2000/svg"], spaces)


def c55_answers_the_pointer(page):
    """C55: the canvas pans, zooms, opens a group on a double-click and keeps its sidebar's width."""
    page.goto(URL + "/plans/check/draws")
    wait_drawn(page)
    page.click("button[data-act=collapse]")
    page.wait_for_timeout(300)
    seen = {}
    transform = lambda: page.get_attribute(".reef-viewport", "transform")
    box = page.locator(".reef-svg").bounding_box()
    x, y = box["x"] + 60, box["y"] + 60
    # a wheel zooms, since the view fills the page, and a drag on nothing pans and selects nothing
    first = transform()
    page.mouse.move(x, y)
    page.mouse.wheel(0, -200)
    page.wait_for_timeout(150)
    seen["wheel zooms"] = transform() != first
    zoomed = transform()
    page.mouse.down()
    page.mouse.move(x + 60, y + 40, steps=6)
    page.mouse.up()
    page.wait_for_timeout(150)
    seen["drag pans"] = transform() != zoomed and page.locator(".reef-side").count() == 0
    # a double-click on the collapsed group's head opens it: past its toggle, and left of where the
    # sidebar its first click opens comes in, which reaches over the right of the centred plan
    page.keyboard.press("0")
    group = page.locator(".reef-viewport .reef-node.collapsed", has_text="G1 The work").bounding_box()
    page.mouse.dblclick(group["x"] + 60, group["y"] + 12)
    page.wait_for_timeout(300)
    seen["double-click opens"] = page.get_attribute(".reef", "data-boxes") == "5"
    # every arrow's head is a marker the page has
    seen["arrowheads"] = page.evaluate(
        "() => [...document.querySelectorAll('.reef-edge')].every(e => document.querySelector(e.getAttribute('marker-end').slice(4, -1)))")
    # the sidebar's edge, dragged, keeps its width over a reload
    page.locator(".reef-viewport .reef-node", has_text="W2 Build on it").click()
    page.wait_for_selector(".reef-side")
    grip = page.locator(".reef-side-grip").bounding_box()
    page.mouse.move(grip["x"] + 4, grip["y"] + 200)
    page.mouse.down()
    page.mouse.move(grip["x"] - 100, grip["y"] + 200, steps=5)
    page.mouse.up()
    page.wait_for_timeout(150)
    dragged = round(page.locator(".reef-side").bounding_box()["width"])
    page.reload()
    wait_drawn(page)
    page.locator(".reef-viewport .reef-node", has_text="W2 Build on it").click()
    page.wait_for_selector(".reef-side")
    seen["width kept"] = [dragged, round(page.locator(".reef-side").bounding_box()["width"])]
    page.evaluate("() => localStorage.removeItem('reef:side-w:page')")
    check("C55 the canvas answers the pointer", all(v is True for k, v in seen.items() if k != "width kept")
          and seen["width kept"][0] == seen["width kept"][1], seen)


def c15_a_put_shows_live(page):
    """C15: a status set from the CLI shows up without a reload, keeping the groups and selection."""
    page.goto(URL + "/plans/check/draws")
    wait_drawn(page)
    page.click("button[data-act=expand]")
    page.wait_for_timeout(300)
    page.locator(".reef-viewport .reef-node", has_text="W2 Build on it").click()
    page.wait_for_selector(".reef-side", timeout=5000)
    page.evaluate("() => { window.__kept = 'same page'; }")
    badges = "() => [...document.querySelectorAll('.reef-side .reef-badge')].map(b => b.textContent)"
    before = (page.get_attribute(".reef", "data-boxes"), page.evaluate(badges))
    # Claude, or anyone, sets the status from the terminal
    reef("status", "W2", "done", "--plan", "draws")
    page.wait_for_function("() => [...document.querySelectorAll('.reef-side .reef-badge')]"
                           ".some(b => b.textContent === 'done')", timeout=10000)
    after = (page.get_attribute(".reef", "data-boxes"), page.evaluate(badges))
    kept = page.evaluate("() => window.__kept")
    panel = page.inner_text(".reef-side .reef-id") if page.locator(".reef-side").count() else None
    check("C15 a CLI put shows up without a reload", kept == "same page" and before[0] == after[0]
          and "done" in after[1] and panel == "W2",
          {"before": before, "after": after, "no reload": kept, "panel": panel})


def c20_a_stale_save_shows_the_newer(page):
    """C20: the panel saves a node's YAML, and a save after another writer's shows the newer version."""
    page.goto(URL + "/plans/check/draws")
    wait_drawn(page)
    page.click("button[data-act=expand]")
    page.wait_for_timeout(300)
    page.locator(".reef-viewport .reef-node", has_text="W3 Look after it").click()
    page.click(".reef-side .reef-app-tab:text-is('Edit')")
    page.wait_for_function("() => (document.querySelector('.reef-app-yaml') || {}).value?.includes('id: W3')",
                           timeout=10000)
    # a plain edit saves, and the diagram redraws with it
    yaml = page.input_value(".reef-app-yaml")
    page.fill(".reef-app-yaml", yaml.replace("W3 Look after it", "W3 Look after it well"))
    page.click(".reef-app-save")
    page.wait_for_selector(".reef-app-saved", timeout=10000)
    page.wait_for_function("() => document.querySelector('.reef-viewport').textContent.includes('Look after it well')",
                           timeout=10000)
    page.wait_for_timeout(1000)
    redrawn = "Look after it well" in page.evaluate("() => document.querySelector('.reef-viewport').textContent")
    # an edit made while another writer moves the node on
    page.fill(".reef-app-yaml", page.input_value(".reef-app-yaml").replace("It lasts.", "It lasts for years."))
    reef("status", "W3", "done", "--plan", "draws")
    page.wait_for_timeout(1500)
    kept = "It lasts for years." in page.input_value(".reef-app-yaml")
    page.click(".reef-app-save")
    page.wait_for_selector(".reef-app-stale", timeout=10000)
    newer = page.inner_text(".reef-app-newer")
    page.screenshot(path=str(SHOTS / "reef-app-stale.png"))
    # starting again from the newer version
    page.click(".reef-app-take")
    page.wait_for_timeout(300)
    taken = page.input_value(".reef-app-yaml")
    check("C20 a stale save shows the newer version",
          redrawn and kept and "status: done" in newer and "status: done" in taken
          and page.locator(".reef-app-stale").count() == 0,
          {"saved and redrawn": redrawn, "draft kept through the change": kept, "newer shown": newer[:120],
           "newer taken": "status: done" in taken})


# a page kept with a plan, under a heading a node links
PAGE = "# The design\n\n## Rows\n\nRows live with the plan.\n\n## Columns\n\nColumns are elsewhere.\n"


def page_text(page):
    """The text of the sidebar's open doc sections."""
    return page.evaluate("() => [...document.querySelectorAll('.reef-side .reef-doc-text')]"
                         ".map(d => d.textContent).join(' | ')")


def c28_a_linked_page_shows_in_the_sidebar(page):
    """C28: a node's doc link to a page kept with the plan, with no file in the repo behind it,
    shows the page's section in the sidebar."""
    # a plan with a page put from the terminal, and a node linking its heading
    reef("plan", "new", "pages")
    source = WORK / "rows.md"
    source.write_text(PAGE)
    reef("page", "put", "design/rows.md", str(source), "--plan", "pages")
    put("pages", node_yaml("P1", "P1 Linked to a page", "docs: [design/rows.md#rows]\n"))
    in_repo = (WORK / "repo/design/rows.md").exists()
    page.goto(URL + "/plans/check/pages")
    wait_drawn(page)
    page.locator(".reef-viewport .reef-node", has_text="P1 Linked to a page").click()
    page.wait_for_selector(".reef-side", timeout=5000)
    try:
        page.wait_for_function("() => [...document.querySelectorAll('.reef-side .reef-doc-text')]"
                               ".some(d => d.textContent.includes('Rows live with the plan.'))", timeout=10000)
    except Exception:
        pass
    text = page_text(page)
    page.screenshot(path=str(SHOTS / "reef-app-page.png"))
    check("C28 a linked plan page shows in the sidebar",
          "Rows live with the plan." in text and "Columns are elsewhere." not in text and not in_repo,
          {"sidebar": text, "file in the repo": in_repo})


def c28_an_edited_page_shows_live(page):
    """C28: a page edited from the terminal shows in the sidebar that has it open, without a
    reload."""
    page.evaluate("() => { window.__kept = 'same page'; }")
    # the page got, changed and put back from the terminal, at the version it was got at
    got = reef("page", "get", "design/rows.md", "--plan", "pages")
    edited = WORK / "rows-edited.md"
    edited.write_text(got.replace("Rows live with the plan.", "Rows moved with the plan."))
    reef("page", "put", "design/rows.md", str(edited), "--plan", "pages")
    try:
        page.wait_for_function("() => [...document.querySelectorAll('.reef-side .reef-doc-text')]"
                               ".some(d => d.textContent.includes('Rows moved with the plan.'))", timeout=10000)
    except Exception:
        pass
    text = page_text(page)
    kept = page.evaluate("() => window.__kept")
    check("C28 a page edited from the terminal shows in the open sidebar without a reload",
          "Rows moved with the plan." in text and kept == "same page",
          {"sidebar": text, "no reload": kept})


def sidebar_title(page):
    """The title of the node the sidebar shows, or None with no sidebar open."""
    return page.evaluate("() => document.querySelector('.reef-side h2')?.textContent ?? null")


def c68_a_node_link_opens_its_node(page):
    """C68: a page's link naming a node, as import makes a link into a node's file, opens that
    node in the sidebar when it is clicked in another node's doc section."""
    # a node, a page naming it, and a node whose doc link reads that page's section
    put("pages", node_yaml("P2", "P2 Named by a page"))
    source = WORK / "named.md"
    source.write_text("# Named\n\n## Rows\n\nSee [the work it names](node:P2).\n")
    reef("page", "put", "design/named.md", str(source), "--plan", "pages")
    put("pages", node_yaml("P3", "P3 Reads the naming page", "docs: [design/named.md#rows]\n"))
    page.goto(URL + "/plans/check/pages")
    wait_drawn(page)
    page.locator(".reef-viewport .reef-node", has_text="P3 Reads the naming page").click()
    link = ".reef-side .reef-doc-text a[data-goto=P2]"
    try:
        page.wait_for_selector(link, timeout=10000)
    except Exception:
        pass
    shown = page.locator(link).count() == 1
    before = sidebar_title(page)
    # clicked, the sidebar shows the node it names, with nothing loaded in the page's place
    if shown:
        page.click(link)
        try:
            page.wait_for_function("() => document.querySelector('.reef-side h2')?.textContent === 'P2 Named by a page'",
                                   timeout=5000)
        except Exception:
            pass
    after = sidebar_title(page)
    page.screenshot(path=str(SHOTS / "reef-app-node-link.png"))
    check("C68 a page's link naming a node opens that node in the sidebar",
          shown and before == "P3 Reads the naming page" and after == "P2 Named by a page"
          and page.url.endswith("/plans/check/pages"),
          {"link shown": shown, "before": before, "after": after, "url": page.url})


def c69_a_body_put_shows_in_the_sidebar(page):
    """C69: a node's body put from the terminal as a node file shows in the sidebar that has the
    node open, without a reload."""
    page.evaluate("() => { window.__kept = 'same page'; }")
    # the node got as markdown, given a body, and put back at the version it was got at
    got = reef("node", "get", "P2", "--md", "--plan", "pages")
    edited = WORK / "p2.md"
    edited.write_text(got + "# P2\n\nThe body, put from the terminal.\n")
    reef("node", "put", str(edited), "--plan", "pages")
    try:
        page.wait_for_function("() => document.querySelector('.reef-side .reef-body')?.textContent"
                               ".includes('The body, put from the terminal.')", timeout=10000)
    except Exception:
        pass
    body = page.evaluate("() => document.querySelector('.reef-side .reef-body')?.textContent ?? null")
    kept = page.evaluate("() => window.__kept")
    check("C69 a body put from the terminal shows in the open sidebar",
          body is not None and "The body, put from the terminal." in body and kept == "same page",
          {"body": body, "no reload": kept})


def rows_of(page, path):
    """Each line of an open file's diff as drawn: its class, its two numbers, and its sign and text."""
    return page.evaluate(
        """(path) => [...document.querySelectorAll(`.reef-app-diff[data-path="${path}"] tr.reef-app-line`)]
            .map(tr => [tr.className.replace('reef-app-line ', ''), ...[...tr.cells].map(td => td.textContent)])""",
        path)


# opens a file of the Changes tab and waits for every line of its diff to be drawn and painted,
# answering how long that took, and the longest task the main thread ran meanwhile
OPEN_TIMED = """async ([path, lines]) => {
    const longest = { ms: 0 };
    const observer = new PerformanceObserver(list => {
        for (const entry of list.getEntries()) longest.ms = Math.max(longest.ms, entry.duration);
    });
    try { observer.observe({ type: 'longtask' }); } catch (e) {}
    const head = document.querySelector(`.reef-app-file-head[data-path="${path}"]`);
    const started = performance.now();
    head.click();
    await new Promise(resolve => {
        const tick = () => document.querySelector(`.reef-app-diff[data-path="${path}"]`)
            ? resolve() : requestAnimationFrame(tick);
        requestAnimationFrame(tick);
    });
    // a frame after the rows are in, so they are painted
    await new Promise(resolve => requestAnimationFrame(() => setTimeout(resolve, 0)));
    const ms = performance.now() - started;
    observer.disconnect();
    const drawn = document.querySelectorAll(`.reef-app-diff[data-path="${path}"] tr.reef-app-line`).length;
    return { ms, drawn, longest: longest.ms, wanted: lines };
}"""


def c33_changes_in_the_sidebar(page):
    """C33: the Changes tab lists a node's revisions and files, draws an opened file's diff as it was
    captured, shows a capture without a reload, and opens a large file without freezing the app."""
    repo = WORK / "repo"
    # a file the base holds, then a plan of its own, so the other checks' plans are as they were
    (repo / "src").mkdir(exist_ok=True)
    (repo / "src" / "keep.rs").write_text("a\nb\nc\n")
    git("add", "-A")
    git("commit", "-q", "-m", "keep")
    reef("plan", "new", "changes")
    put("changes", node_yaml("K1", "K1 Change the code"))
    # K1 started, then an edit, a new file and X2's commit's Cargo.lock, all captured
    reef("start", "K1", "--plan", "changes")
    (repo / "src" / "keep.rs").write_text("a\nB\nc\n")
    (repo / "src" / "small.rs").write_text('fn main() {\n    println!("<hi> & bye");\n}')
    lock = run("git", "-C", REEF, "show", "31155d8:spikes/x2-rows/Cargo.lock")
    (repo / "Cargo.lock").write_text(lock)
    git("add", "-A")
    reef("node", "capture")
    page.goto(URL + "/plans/check/changes")
    wait_drawn(page)
    page.locator(".reef-viewport .reef-node", has_text="K1 Change the code").click()
    page.click(".reef-side .reef-app-tab:text-is('Changes')")
    page.wait_for_selector(".reef-app-file-head", timeout=10000)
    seen = {}
    seen["files"] = page.eval_on_selector_all(
        ".reef-app-file-head",
        "heads => heads.map(h => [h.dataset.path, h.querySelector('.reef-app-status').textContent,"
        " h.querySelector('.reef-app-file-counts').textContent])")
    seen["revisions"] = page.eval_on_selector_all(".reef-app-revision option", "os => os.map(o => o.value)")
    # the edited file and the new one draw as git numbers their lines
    for path in ["src/keep.rs", "src/small.rs"]:
        page.click(f'.reef-app-file-head[data-path="{path}"]')
        page.wait_for_selector(f'.reef-app-diff[data-path="{path}"] tr.reef-app-line', timeout=10000)
        seen[path] = rows_of(page, path)
    seen["no newline"] = page.locator('.reef-app-diff[data-path="src/small.rs"] .reef-app-eof').count()
    page.locator(".reef-side").screenshot(path=str(SHOTS / "reef-app-changes.png"))
    page.emulate_media(color_scheme="dark")
    page.locator(".reef-side").screenshot(path=str(SHOTS / "reef-app-changes-dark.png"))
    page.emulate_media(color_scheme="light")
    wanted_files = [["Cargo.lock", "A", f"+{lock.count(chr(10))} −0"], ["src/keep.rs", "M", "+1 −1"],
                    ["src/small.rs", "A", "+3 −0"]]
    wanted_keep = [["context", "1", "1", " a"], ["removed", "2", "", "-b"], ["added", "", "2", "+B"],
                   ["context", "3", "3", " c"]]
    wanted_small = [["added", "", "1", "+fn main() {"], ["added", "", "2", '+    println!("<hi> & bye");'],
                    ["added", "", "3", "+}"]]
    check("C33 the Changes tab lists revisions and files, and draws a file's diff as captured",
          seen["files"] == wanted_files and seen["revisions"] == ["1"] and seen["src/keep.rs"] == wanted_keep
          and seen["src/small.rs"] == wanted_small and seen["no newline"] == 1, seen)
    # a capture from the terminal shows up in the open tab, the file still open
    page.evaluate("() => { window.__kept = 'same page'; }")
    (repo / "src" / "small.rs").write_text('fn main() {\n    println!("<hi> & bye");\n}\n// more\n')
    git("add", "-A")
    reef("node", "capture")
    try:
        page.wait_for_function("() => document.querySelectorAll('.reef-app-revision option').length === 2",
                               timeout=10000)
        page.wait_for_function("() => document.querySelector('.reef-app-diff[data-path=\"src/small.rs\"]')"
                               "?.dataset.revision === '2'", timeout=10000)
    except Exception:
        pass
    live = {
        "revisions": page.eval_on_selector_all(".reef-app-revision option", "os => os.map(o => o.value)"),
        "picked": page.eval_on_selector(".reef-app-revision", "s => s.value"),
        "open file's revision": page.get_attribute('.reef-app-diff[data-path="src/small.rs"]', "data-revision"),
        "no reload": page.evaluate("() => window.__kept"),
    }
    check("C33 a capture shows in an open Changes tab without a reload",
          live == {"revisions": ["2", "1"], "picked": "2", "open file's revision": "2", "no reload": "same page"},
          live)
    # the large file, opened again and again, each time from its click to its rows painted
    lines = lock.count("\n")
    page.click('.reef-app-file-head[data-path="src/keep.rs"]')
    page.click('.reef-app-file-head[data-path="src/small.rs"]')
    rounds = []
    for k in range(13):
        timed = page.evaluate(OPEN_TIMED, ["Cargo.lock", lines])
        if k >= 3:
            rounds.append(timed)
        page.click('.reef-app-file-head[data-path="Cargo.lock"]')
        page.wait_for_function("() => !document.querySelector('.reef-app-diff[data-path=\"Cargo.lock\"]')",
                               timeout=10000)
    times = sorted(r["ms"] for r in rounds)
    figures = {"panel-diff-ms": [round(times[len(times) // 2], 1), round(times[0], 1), round(times[-1], 1)],
               "longest task ms": max(r["longest"] for r in rounds),
               "lines drawn": sorted({r["drawn"] for r in rounds}), "lines": lines,
               "build": "release" if RELEASE else "debug"}
    # the figures are the plan's only from a release build
    in_time = figures["panel-diff-ms"][0] <= 200 if RELEASE else True
    check("C33 a large diff opens without freezing the app", figures["lines drawn"] == [lines] and in_time,
          figures)


def c56_drafts_in_the_sidebar(page):
    """C56: a node's draft says so in the Changes tab and takes no comment, an open file of it is
    drawn again each time a hook rewrites it, and once Stop seals it, it is a revision like any."""
    repo = WORK / "repo"
    reef("plan", "new", "drafts")
    put("drafts", node_yaml("D1", "D1 Draft the code"))
    reef("start", "D1", "--plan", "drafts")
    # a commit, which git's post-commit drafts
    (repo / "src").mkdir(exist_ok=True)
    (repo / "src" / "draft.rs").write_text("one\n")
    git("add", "-A")
    git("commit", "-q", "-m", "D1 one")
    reef("hook", "post-commit")
    page.goto(URL + "/plans/check/drafts")
    wait_drawn(page)
    page.locator(".reef-viewport .reef-node", has_text="D1 Draft the code").click()
    page.click(".reef-side .reef-app-tab:text-is('Changes')")
    page.wait_for_selector('.reef-app-file-head[data-path="src/draft.rs"]', timeout=10000)
    page.click('.reef-app-file-head[data-path="src/draft.rs"]')
    page.wait_for_selector('.reef-app-diff[data-path="src/draft.rs"] tr.reef-app-line', timeout=10000)
    shown = page.evaluate("""() => ({
        option: document.querySelector('.reef-app-revision option')?.textContent ?? null,
        tag: document.querySelector('.reef-app-revision-sum .reef-app-tag.draft')?.textContent ?? null,
        commenting: !!document.querySelector('.reef-app-diff.commenting'),
        note: [...document.querySelectorAll('.reef-app-diff-note')].map(p => p.textContent),
    })""")
    page.locator(".reef-side").screenshot(path=str(SHOTS / "reef-app-draft.png"))
    check("C56 a draft says so in the Changes tab, and its lines take no comment",
          shown["option"] is not None and "draft" in shown["option"] and shown["tag"] == "draft"
          and not shown["commenting"] and "Comments open once this draft is sealed." in shown["note"],
          shown)
    # a second commit rewrites the draft under its number, and the open file is drawn again
    page.evaluate("() => { window.__kept = 'same page'; }")
    (repo / "src" / "draft.rs").write_text("one\ntwo\n")
    git("commit", "-q", "-a", "-m", "D1 two")
    reef("hook", "post-commit")
    try:
        page.wait_for_function("() => document.querySelectorAll("
                               "'.reef-app-diff[data-path=\"src/draft.rs\"] tr.reef-app-line').length === 2",
                               timeout=10000)
    except Exception:
        pass
    redrawn = {
        "revisions": page.eval_on_selector_all(".reef-app-revision option", "os => os.map(o => o.value)"),
        "rows": rows_of(page, "src/draft.rs"),
        "no reload": page.evaluate("() => window.__kept"),
    }
    check("C56 a draft rewritten by a hook is drawn again in the open tab, under its number",
          redrawn == {"revisions": ["1"], "rows": [["added", "", "1", "+one"], ["added", "", "2", "+two"]],
                      "no reload": "same page"},
          redrawn)
    # D1 done, and the turn's end seals it, which opens its lines to comments
    reef("status", "D1", "done", "--plan", "drafts")
    stop = json.dumps({"cwd": str(repo), "hook_event_name": "Stop", "stop_hook_active": False})
    sealed_out = subprocess.run([REEF / "target/debug/reef", "--conf", WORK / "reef.toml", "hook", "stop"],
                                cwd=repo, input=stop, capture_output=True, text=True)
    try:
        page.wait_for_function("() => !document.querySelector('.reef-app-revision-sum .reef-app-tag.draft')"
                               " && !!document.querySelector('.reef-app-diff.commenting')", timeout=10000)
    except Exception:
        pass
    sealed = page.evaluate("""() => ({
        option: document.querySelector('.reef-app-revision option')?.textContent ?? null,
        tag: !!document.querySelector('.reef-app-revision-sum .reef-app-tag.draft'),
        commenting: !!document.querySelector('.reef-app-diff.commenting'),
    })""")
    sealed["stop"] = sealed_out.stdout.strip()
    check("C56 once Stop seals the draft, it is a revision whose lines take comments",
          sealed["stop"] == "reef: sealed `D1` as revision 1, 1 file +2 -0" and "draft" not in (sealed["option"] or "draft")
          and not sealed["tag"] and sealed["commenting"],
          sealed)


def open_counts(page):
    """How many open threads the Comments tab and the toolbar each say there are."""
    return page.evaluate("""() => [
        document.querySelector('.reef-side .reef-app-tab:nth-child(3) .reef-app-count')?.textContent ?? null,
        document.querySelector('.reef-app-open-comments .reef-app-count')?.textContent ?? null,
    ]""")


def wait_counts(page, counts):
    """Waits for the tab and the toolbar to say `counts` open threads, then answers what they say."""
    try:
        page.wait_for_function(f"""() => [
            document.querySelector('.reef-side .reef-app-tab:nth-child(3) .reef-app-count')?.textContent,
            document.querySelector('.reef-app-open-comments .reef-app-count')?.textContent,
        ].join() === '{counts},{counts}'""", timeout=10000)
    except Exception:
        pass
    return open_counts(page)


def c35_comment_threads_in_the_diff(page):
    """C35: a line clicked in the diff starts a thread shown under it, the node takes a general
    comment in its Comments tab, a reply shows in its thread, a resolve takes it off the open count,
    and the toolbar lists and counts what is open. Runs after C33, on its plan."""
    page.goto(URL + "/plans/check/changes")
    wait_drawn(page)
    page.locator(".reef-viewport .reef-node", has_text="K1 Change the code").click()
    page.click(".reef-side .reef-app-tab:text-is('Changes')")
    page.wait_for_selector('.reef-app-file-head[data-path="src/keep.rs"]', timeout=10000)
    seen = {"before": wait_counts(page, 0)}
    # a click on the added line's numbers opens a box under it, which keeps a thread on it
    page.click('.reef-app-file-head[data-path="src/keep.rs"]')
    line = '.reef-app-diff[data-path="src/keep.rs"] tr.reef-app-line[data-side="new"][data-line="2"]'
    page.wait_for_selector(line, timeout=10000)
    page.click(f"{line} td.reef-app-ln >> nth=1")
    page.fill(".reef-app-compose-row textarea", "**hi** <b>x</b>")
    page.click(".reef-app-compose-row .reef-app-send")
    page.wait_for_selector(".reef-app-thread-row .reef-app-thread", timeout=10000)
    seen["under its line"] = page.evaluate(f"""() => {{
        const row = document.querySelector('{line}').nextElementSibling;
        const body = row?.querySelector('.reef-app-comment-body');
        return [row?.className, body?.querySelector('strong')?.textContent, body?.textContent.trim(),
                body?.querySelectorAll('b').length];
    }}""")
    seen["after a line comment"] = wait_counts(page, 1)
    page.locator(".reef-side").screenshot(path=str(SHOTS / "reef-app-thread-inline.png"))
    check("C35 clicking a diff line starts a comment, and its thread shows under the line",
          seen["under its line"] == ["reef-app-thread-row", "hi", "hi <b>x</b>", 0], seen)
    # a general comment from the Comments tab
    page.click(".reef-side .reef-app-tab:nth-child(3)")
    page.fill(".reef-app-comments > .reef-app-composer textarea", "Looks good *overall*.")
    page.click(".reef-app-comments > .reef-app-composer .reef-app-send")
    page.wait_for_function("() => [...document.querySelectorAll('.reef-app-comments .reef-app-thread')]"
                           ".some(t => t.textContent.includes('Looks good overall.'))", timeout=10000)
    general = page.evaluate("""() => [...document.querySelectorAll('.reef-app-comments > .reef-app-thread')]
        .map(t => t.querySelector('.reef-app-comment-body').innerHTML.trim())""")
    seen["after a general comment"] = wait_counts(page, 2)
    check("C35 a general comment on the node shows in its Comments tab",
          general == ["<p>Looks good <em>overall</em>.</p>"], {"general threads": general})
    check("C35 a new comment bumps the open count, in the Comments tab and in the toolbar",
          seen["before"] == ["0", "0"] and seen["after a line comment"] == ["1", "1"]
          and seen["after a general comment"] == ["2", "2"], seen)
    # a reply in the line's thread, then the thread resolved
    thread = ".reef-app-line-thread .reef-app-thread"
    page.click(f"{thread} .reef-app-reply-open")
    page.fill(f"{thread} .reef-app-composer textarea", "Because.")
    page.click(f"{thread} .reef-app-composer .reef-app-send")
    page.wait_for_function(f"() => document.querySelectorAll('{thread} .reef-app-comment').length === 2",
                           timeout=10000)
    replies = page.eval_on_selector_all(f"{thread} .reef-app-comment-body", "bs => bs.map(b => b.textContent.trim())")
    page.click(f"{thread} [data-act=resolve]")
    page.wait_for_selector(".reef-app-line-thread .reef-app-thread.resolved", timeout=10000)
    resolved = wait_counts(page, 1)
    page.locator(".reef-side").screenshot(path=str(SHOTS / "reef-app-comments.png"))
    page.emulate_media(color_scheme="dark")
    page.locator(".reef-side").screenshot(path=str(SHOTS / "reef-app-comments-dark.png"))
    page.emulate_media(color_scheme="light")
    check("C35 a reply shows in its thread, and resolving it takes it off the open count",
          replies == ["hi <b>x</b>", "Because."] and resolved == ["1", "1"],
          {"thread": replies, "open after the resolve": resolved})
    # a comment another writer makes shows in the toolbar without a reload
    page.evaluate("() => { window.__kept = 'same page'; }")
    request = urllib.request.Request(
        f"{URL}/api/v1/projects/check/plans/changes/nodes/K1/comments",
        data=json.dumps({"target": {"on": "node"}, "body": "From the terminal"}).encode(),
        headers={"content-type": "application/json"}, method="POST")
    with urllib.request.urlopen(request, timeout=5) as answer:
        answer.read()
    live = {"counts": wait_counts(page, 2), "no reload": page.evaluate("() => window.__kept")}
    # the toolbar's list goes to a thread: the sidebar closed, then the entry clicked
    page.click(".reef-side .reef-close")
    page.click("[data-act=comments]")
    entries = page.eval_on_selector_all(".reef-app-open-list a", "as => as.map(a => a.textContent)")
    page.screenshot(path=str(SHOTS / "reef-app-open-list.png"))
    page.click(".reef-app-open-list a >> nth=0")
    page.wait_for_selector(".reef-side .reef-app-comments", timeout=10000)
    went = page.evaluate("""() => [document.querySelector('.reef-side .reef-id')?.textContent,
        document.querySelector('.reef-side .reef-app-tab.on')?.textContent]""")
    # and Show in diff opens the line's file at the line
    page.click(".reef-app-line-thread .reef-app-show-line")
    page.wait_for_selector(".reef-app-line.focus", timeout=10000)
    shown = page.evaluate("""() => { const l = document.querySelector('.reef-app-line.focus');
        return [l.closest('.reef-app-diff').dataset.path, l.dataset.side, l.dataset.line]; }""")
    check("C35 the toolbar lists the open threads, each going to its node, and a thread shows its line",
          live == {"counts": ["2", "2"], "no reload": "same page"} and len(entries) == 2
          and "K1" in entries[0] and went[0] == "K1" and went[1].startswith("Comments")
          and shown == ["src/keep.rs", "new", "2"],
          {"live": live, "entries": entries, "went to": went, "shown in diff": shown})


def c73_a_review_holds_comments_until_submitted(page):
    """C73: Start a review keeps a comment pending, tagged so and unresolvable, off the open
    counts and out of the JSON API's threads unless asked for, and the Comments tab's review bar
    submits it, publishing it at once. Runs after C33, on its plan."""
    page.goto(URL + "/plans/check/changes")
    wait_drawn(page)
    page.locator(".reef-viewport .reef-node", has_text="K1 Change the code").click()
    page.click(".reef-side .reef-app-tab:nth-child(3)")
    page.wait_for_selector(".reef-app-comments > .reef-app-composer", timeout=10000)
    before = open_counts(page)
    composer = ".reef-app-comments > .reef-app-composer"
    buttons = page.eval_on_selector_all(f"{composer} .reef-app-actions button", "bs => bs.map(b => b.textContent)")
    # a general comment written in a review, which starts one
    page.fill(f"{composer} textarea", "Held for *later*.")
    page.click(f"{composer} [data-act=review]")
    page.wait_for_selector(".reef-app-review-bar", timeout=10000)
    pending = page.evaluate("""() => {
        const t = [...document.querySelectorAll('.reef-app-comments .reef-app-thread')]
            .find(t => t.textContent.includes('Held for later.'));
        return [t?.classList.contains('pending'), t?.querySelector('.reef-app-pending')?.textContent,
                t?.querySelector('.reef-app-mark') === null,
                document.querySelector('.reef-app-review-sum')?.textContent];
    }""")
    after = open_counts(page)
    in_review = page.eval_on_selector_all(f"{composer} .reef-app-actions button", "bs => bs.map(b => b.textContent)")
    page.locator(".reef-side").screenshot(path=str(SHOTS / "reef-app-review.png"))

    # the JSON API leaves it out unless asked for it
    def bodies(query):
        path = f"{URL}/api/v1/projects/check/plans/changes/nodes/K1/comments{query}"
        with urllib.request.urlopen(path, timeout=5) as answer:
            threads = json.loads(answer.read())
        return [c["body"] for t in threads for c in t["comments"] if "later" in c["body"]]

    hidden = {"plain": bodies(""), "asked": bodies("?pending=true")}
    check("C73 Start a review keeps a comment pending, tagged so, unresolvable and off the open counts",
          buttons == ["Comment", "Start a review"] and pending == [True, "pending", True, "Review · 1 pending"]
          and after == before and in_review == ["Comment now", "Add to review"],
          {"buttons": buttons, "pending": pending, "counts": [before, after], "in review": in_review})
    check("C73 the JSON API leaves a pending comment out unless asked for it",
          hidden == {"plain": [], "asked": ["Held for *later*."]}, hidden)
    # submitted, it is published at once and counted
    page.click(".reef-app-review-bar [data-act=submit]")
    page.wait_for_selector(".reef-app-review-bar", state="detached", timeout=10000)
    count = str(int(before[0] or "0") + 1)
    submitted = {
        "counts": wait_counts(page, count),
        "pending": page.evaluate("() => document.querySelectorAll('.reef-app-comments .reef-app-thread.pending').length"),
        "api": bodies(""),
    }
    check("C73 submitting the review publishes its comment at once",
          submitted == {"counts": [count, count], "pending": 0, "api": ["Held for *later*."]}, submitted)


def c36_comments_follow_the_code(page):
    """C36: a capture that moves a commented line shows its thread at the line's new place, resolved
    or not, one that deletes a commented line shows its thread as outdated, and an older revision
    still shows each thread where its line was then. Runs after C35, on its plan."""
    repo = WORK / "repo"
    page.goto(URL + "/plans/check/changes")
    wait_drawn(page)
    page.locator(".reef-viewport .reef-node", has_text="K1 Change the code").click()
    page.click(".reef-side .reef-app-tab:text-is('Changes')")
    page.wait_for_selector('.reef-app-file-head[data-path="src/keep.rs"]', timeout=10000)
    # a thread on the line revision 2 added to small.rs, from another writer
    line = {"on": "line", "revision": 2, "path": "src/small.rs", "side": "new", "line": 4}
    request = urllib.request.Request(
        f"{URL}/api/v1/projects/check/plans/changes/nodes/K1/comments",
        data=json.dumps({"target": line, "body": "Why more?"}).encode(),
        headers={"content-type": "application/json"}, method="POST")
    with urllib.request.urlopen(request, timeout=5) as answer:
        answer.read()
    # revision 3 adds a line above keep.rs's `B`, C35's resolved thread's line, and drops small.rs's last
    (repo / "src" / "keep.rs").write_text("a\nz\nB\nc\n")
    (repo / "src" / "small.rs").write_text('fn main() {\n    println!("<hi> & bye");\n}\n')
    git("add", "-A")
    reef("node", "capture")
    seen = {}
    try:
        page.wait_for_function("() => document.querySelector('.reef-app-revision')?.value === '3'", timeout=10000)
    except Exception:
        pass
    seen["picked"] = page.eval_on_selector(".reef-app-revision", "s => s.value")
    # in revision 3, the thread is under `B` where it is now, and small.rs has none
    moved = '.reef-app-diff[data-path="src/keep.rs"] tr.reef-app-line[data-side="new"][data-line="3"]'
    page.click('.reef-app-file-head[data-path="src/keep.rs"]')
    page.click('.reef-app-file-head[data-path="src/small.rs"]')
    page.wait_for_selector(f"{moved} + tr.reef-app-thread-row", timeout=10000)
    page.wait_for_selector('.reef-app-diff[data-path="src/small.rs"][data-revision="3"] tr.reef-app-line',
                           timeout=10000)
    under = """line => { const l = document.querySelector(line), row = l?.nextElementSibling;
        return [l?.querySelector('.reef-app-code').textContent,
                row?.classList.contains('reef-app-thread-row') ? row.querySelector('.reef-app-comment-body').textContent.trim() : null,
                [...(row?.querySelector('.reef-app-thread')?.classList ?? [])].filter(c => c !== 'reef-app-thread')]; }"""
    seen["r3 keep.rs:3"] = page.evaluate(under, moved)
    seen["r3 threads on small.rs"] = page.locator('.reef-app-diff[data-path="src/small.rs"] .reef-app-thread-row').count()
    page.locator(".reef-side").screenshot(path=str(SHOTS / "reef-app-followed.png"))
    # the Comments tab names each where its line is now, and says which is outdated
    page.click(".reef-side .reef-app-tab:nth-child(3)")
    page.wait_for_selector(".reef-app-line-thread .reef-app-outdated", timeout=10000)
    seen["places"] = page.eval_on_selector_all(
        ".reef-app-line-thread",
        "ts => ts.map(t => [t.querySelector('code').textContent, !!t.querySelector('.reef-app-outdated')])")
    page.locator(".reef-side").screenshot(path=str(SHOTS / "reef-app-outdated.png"))
    page.emulate_media(color_scheme="dark")
    page.locator(".reef-side").screenshot(path=str(SHOTS / "reef-app-outdated-dark.png"))
    page.emulate_media(color_scheme="light")
    # revision 2 picked again still shows each thread where its line was then
    page.click(".reef-side .reef-app-tab:text-is('Changes')")
    page.wait_for_selector(".reef-app-revision", timeout=10000)
    page.select_option(".reef-app-revision", "2")
    try:
        page.wait_for_function("() => [...document.querySelectorAll('.reef-app-diff')].every(d => d.dataset.revision === '2')"
                               " && document.querySelectorAll('.reef-app-thread-row').length === 2", timeout=10000)
    except Exception:
        pass
    seen["r2 keep.rs:2"] = page.evaluate(
        under, '.reef-app-diff[data-path="src/keep.rs"] tr.reef-app-line[data-side="new"][data-line="2"]')
    seen["r2 small.rs:4"] = page.evaluate(
        under, '.reef-app-diff[data-path="src/small.rs"] tr.reef-app-line[data-side="new"][data-line="4"]')
    check("C36 a thread shows at its line in each revision it followed, and an outdated one says so",
          seen == {"picked": "3", "r3 keep.rs:3": ["+B", "hi <b>x</b>", ["resolved"]], "r3 threads on small.rs": 0,
                   "places": [["r2 · src/small.rs:4 · outdated in r3", True], ["r3 · src/keep.rs:3", False]],
                   "r2 keep.rs:2": ["+B", "hi <b>x</b>", ["resolved"]],
                   "r2 small.rs:4": ["+// more", "Why more?", ["open", "outdated"]]},
          seen)


def queue_order():
    """The check's queue as `reef queue ls` lists it: each entry's name, in order."""
    return [line.split()[1] for line in reef("queue", "ls").splitlines() if line[:1].isdigit()]


def queue_ranks():
    """Each entry of the check's queue with its rank, as the JSON API lists them."""
    with urllib.request.urlopen(f"{URL}/api/v1/projects/check/queue", timeout=5) as answer:
        listing = json.load(answer)
    return [(listed["entry"]["node"], listed["rank"]) for listed in listing["entries"]]


def panel_order(page):
    """The queue panel's entries, in order, as their names."""
    return page.eval_on_selector_all(".reef-app-queue-entry", "es => es.map(e => e.dataset.node)")


def first_entry_is(page, node):
    """Waits for the queue panel's first entry to be a node."""
    page.wait_for_function(
        "node => document.querySelector('.reef-app-queue-entry')?.dataset.node === node", arg=node, timeout=10000)


def c25_a_drag_writes_one_row(page):
    """C25: the last entry dragged above the first writes that entry's row alone, and the CLI agrees."""
    # three entries across the project's plans, queued from the terminal
    for node in ["check/first/A1", "check/second/A1", "check/draws/L1"]:
        reef("queue", "add", node)
    page.goto(URL + "/plans/check/draws")
    wait_drawn(page)
    page.wait_for_function("() => document.querySelectorAll('.reef-app-queue-entry').length === 3", timeout=10000)
    page.evaluate("() => { window.__kept = 'same page'; }")
    before = queue_ranks()
    shown = panel_order(page)
    # the last entry's grip, dragged above the first entry's middle
    grip = page.locator(".reef-app-queue-grip").nth(2).bounding_box()
    first = page.locator(".reef-app-queue-entry").first.bounding_box()
    x = grip["x"] + grip["width"] / 2
    page.mouse.move(x, grip["y"] + grip["height"] / 2)
    page.mouse.down()
    page.mouse.move(x, first["y"] + 2, steps=8)
    marked = page.locator(".reef-app-queue-entry.drop-before").count() == 1
    page.screenshot(path=str(SHOTS / "reef-app-queue-drag.png"))
    page.mouse.up()
    first_entry_is(page, "check/draws/L1")
    after = queue_ranks()
    moved = [node for node, rank in after if (node, rank) not in before]
    listed, panel = queue_order(), panel_order(page)
    page.screenshot(path=str(SHOTS / "reef-app-queue.png"))
    check("C25 a drag writes one row, and reef queue ls agrees",
          shown == [node for node, _ in before] and marked and moved == ["check/draws/L1"]
          and len(after) == len(before) and listed == panel and panel[0] == "check/draws/L1"
          and page.evaluate("() => window.__kept") == "same page",
          {"before": before, "after": after, "rows moved": moved, "drop marked": marked, "ls": listed,
           "panel": panel})


def c25_keys_and_the_sidebar_queue(page):
    """C25: an entry moves by its arrow key, and the sidebar takes the selected node off and queues it first."""
    page.goto(URL + "/plans/check/draws")
    wait_drawn(page)
    page.wait_for_selector(".reef-app-queue-entry")
    seen = {}
    # the first entry moved down a place from the keyboard
    start = panel_order(page)
    page.locator(".reef-app-queue-entry").first.focus()
    page.keyboard.press("ArrowDown")
    first_entry_is(page, start[1])
    seen["arrow down"] = (panel_order(page), queue_order())
    # L1 selected in the diagram, then taken off from its sidebar
    page.click("button[data-act=expand]")
    page.wait_for_timeout(300)
    page.locator(".reef-viewport .reef-node", has_text="L1 After the work").click()
    page.wait_for_selector(".reef-side [data-act=unqueue]", timeout=5000)
    page.click(".reef-side [data-act=unqueue]")
    page.wait_for_function(
        "() => ![...document.querySelectorAll('.reef-app-queue-entry')].some(e => e.dataset.node === 'check/draws/L1')",
        timeout=10000)
    page.wait_for_timeout(200)
    seen["taken off"] = (panel_order(page), queue_order(), page.locator(".reef-side [data-act=unqueue]").count())
    # and queued first again
    page.click(".reef-side [data-act=queue-top]")
    first_entry_is(page, "check/draws/L1")
    seen["queued first"] = (panel_order(page), queue_order())
    page.screenshot(path=str(SHOTS / "reef-app-queue-side.png"))
    rest = [node for node in start if node != "check/draws/L1"]
    check("C25 an entry moves by its keys, and the sidebar queues and takes off",
          seen["arrow down"] == ([start[1], start[0], start[2]],) * 2
          and seen["taken off"] == (rest, rest, 0)
          and seen["queued first"] == (["check/draws/L1", *rest],) * 2,
          seen)


# Claude Code as `reef loop run` runs it, played over stream-json for the session checks: on the
# loop's kickoff it takes the node with reef's own `loop take`, asks what the play asks and waits
# for the answer, marks the node done and runs reef's own Stop hook, then ends its turn
STUB = r'''#!/usr/bin/env python3
import json, os, subprocess, sys
REEF, PLAY = sys.argv[1], sys.argv[2]
SESSION = f"stub-{os.getpid()}"

def say(value):
    print(json.dumps(value), flush=True)

def text(words):
    say({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": words}]}})

def lines():
    for line in sys.stdin:
        try:
            yield json.loads(line)
        except ValueError:
            pass

INPUT = lines()

def ask(rid, tool, given):
    say({"type": "control_request", "request_id": rid,
         "request": {"subtype": "can_use_tool", "tool_name": tool, "input": given}})
    for line in INPUT:
        if line.get("type") == "control_response" and line["response"]["request_id"] == rid:
            return line["response"]["response"]
    sys.exit(0)

def reef(*args, stdin=None):
    return subprocess.run([REEF, *args], input=stdin, capture_output=True, text=True).stdout

def play(node):
    if PLAY == "permission":
        answer = ask(f"perm-{node}", "Bash", {"command": f"echo {node}"})
        text("Claude heard: " + ("allowed" if answer["behavior"] == "allow" else "denied: " + answer.get("message", "")))
    elif PLAY == "question":
        question = f"Which greeting for {node}?"
        answer = ask(f"ask-{node}", "AskUserQuestion", {"questions": [{
            "question": question, "header": "Greeting", "multiSelect": False,
            "options": [{"label": "Hello", "description": "The usual"}, {"label": "Hi", "description": "Shorter"}]}]})
        text("You picked " + answer.get("updatedInput", {}).get("answers", {}).get(question, "nothing") + ".")
    elif PLAY == "plan":
        plan, n = f"# Plan for {node}\n\nWrite the greeting.", 1
        while True:
            answer = ask(f"plan-{node}-{n}", "ExitPlanMode", {"plan": plan})
            if answer["behavior"] == "allow":
                text(f"Plan {n} was approved.")
                break
            n += 1
            plan = f"# Plan for {node}, revised\n\nWrite the greeting.\n\n{answer.get('message', '')}"

for message in INPUT:
    if message.get("type") != "user":
        continue
    said = message["message"]["content"][0]["text"]
    say({"type": "system", "subtype": "init", "session_id": SESSION, "permissionMode": "default"})
    if said.startswith("Take the next node"):
        taken = reef("loop", "take")
        say({"type": "assistant", "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": "t1", "name": "mcp__reef__next_task", "input": {}}]}})
        say({"type": "user", "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "t1", "content": taken}]}})
        name = next((line.split("`")[1] for line in taken.splitlines() if line.startswith("Took `")), None)
        if name:
            _, plan, node = name.split("/")
            play(node)
            reef("status", node, "done", "--plan", plan)
            stopped = reef("hook", "stop", stdin=json.dumps({"session_id": SESSION, "cwd": os.getcwd()}))
            try:
                message = json.loads(stopped)["systemMessage"]
                say({"type": "system", "subtype": "informational", "content": "Stop says: " + message})
            except ValueError:
                pass
        else:
            text("Nothing is ready to take.")
    else:
        text("You said: " + said)
    say({"type": "result", "subtype": "success", "num_turns": 3, "total_cost_usd": 0.01, "duration_ms": 5})
'''


def runner(plan, play):
    """Starts `reef loop run` in the check's repo, narrowed to a plan, with Claude played by the
    stub, in a process group of its own."""
    stub = WORK / "claude-stub.py"
    stub.write_text(STUB)
    stub.chmod(0o755)
    claude = WORK / f"claude-{play}.sh"
    claude.write_text(f"#!/bin/sh\nexec python3 -I '{stub}' '{REEF / 'target/debug/reef'}' '{play}'\n")
    claude.chmod(0o755)
    env = dict(os.environ)
    for name in list(env):
        if name.startswith(("CLAUDE", "ZELLIJ", "TMUX")) or name == "REEF_PANE":
            env.pop(name)
    log = open(WORK / f"runner-{plan}.log", "w")
    return subprocess.Popen([REEF / "target/debug/reef", "--conf", WORK / "reef.toml", "loop", "run",
                             "--plan", plan, "--claude", claude], cwd=WORK / "repo", env=env, stdout=log,
                            stderr=subprocess.STDOUT, start_new_session=True)


def stop_runner(process):
    """Stops a runner as Ctrl-C would, which lets go of its loop and detaches it."""
    try:
        os.killpg(process.pid, signal.SIGINT)
        process.wait(timeout=20)
    except (ProcessLookupError, subprocess.TimeoutExpired):
        os.killpg(process.pid, signal.SIGKILL)


def open_sessions(page, plan):
    """Opens a plan's page with its sessions panel, once a runner has attached."""
    page.goto(URL + f"/plans/check/{plan}")
    wait_drawn(page)
    page.click(".reef-app-sessions-toggle")
    page.wait_for_selector(".reef-app-runner[data-session] button[data-act=start]", timeout=20000)


def entries(page, kind=""):
    """How many entries of a kind the open transcript shows."""
    return page.locator(f".reef-app-transcript .reef-app-entry{'.' + kind if kind else ''}").count()


def c45_a_loop_started_from_the_app_streams_its_transcript(page):
    """C45: a runner attaches, the app starts its loop, and each session's transcript and prompt
    show live: a prompt allowed, the node done, and the next node taken in a fresh session."""
    reef("plan", "new", "runs")
    put("runs", node_yaml("R1", "R1 The first run"))
    put("runs", node_yaml("R2", "R2 The second run"))
    process = runner("runs", "permission")
    try:
        open_sessions(page, "runs")
        page.evaluate("() => { window.__kept = 'same page'; }")
        idle = page.inner_text(".reef-app-runner .reef-app-session-state")
        page.click(".reef-app-runner button[data-act=start]")
        # R1 taken, its tool waiting in a card, and counted in the bar
        page.wait_for_selector(".reef-app-prompt[data-prompt=perm-R1]", timeout=20000)
        waiting = page.inner_text(".reef-app-sessions-toggle")
        before = entries(page)
        page.screenshot(path=str(SHOTS / "reef-app-session-prompt.png"))
        page.click(".reef-app-prompt[data-prompt=perm-R1] button[data-act=allow]")
        # R2 taken in a fresh session, its own prompt waiting, with no reload
        page.wait_for_selector(".reef-app-prompt[data-prompt=perm-R2]", timeout=20000)
        page.wait_for_function("() => document.querySelectorAll('.reef-app-entry.session').length === 2",
                               timeout=10000)
        after = entries(page)
        said = page.inner_text(".reef-app-transcript")
        page.screenshot(path=str(SHOTS / "reef-app-session.png"))
        # stopped from the app, letting go of R2
        page.click(".reef-app-runner button[data-act=stop]")
        page.wait_for_function("() => !document.querySelector('.reef-app-prompt')", timeout=10000)
        stopped = page.inner_text(".reef-app-runner-loop")
        check("C45 a loop started from the app shows its transcript live",
              idle == "idle" and "1 waiting" in waiting and after > before
              and "Answered: allowed" in said and "Claude heard: allowed" in said and "`check/runs/R1` is done" in said
              and "status: done" in reef("node", "get", "R1", "--plan", "runs")
              and stopped == "No loop started" and page.evaluate("() => window.__kept") == "same page",
              {"idle": idle, "bar": waiting, "entries": [before, after], "stopped": stopped,
               "said": said[-600:]})
    finally:
        stop_runner(process)


def c46_a_question_is_answered_in_the_app(page):
    """C46: a session's question shows with its choices, a pick is sent once each question has one,
    and the session carries on with it; an answer in the user's own words goes the same way."""
    reef("plan", "new", "asks")
    put("asks", node_yaml("Q1", "Q1 The first question"))
    put("asks", node_yaml("Q2", "Q2 The second question"))
    process = runner("asks", "question")
    try:
        open_sessions(page, "asks")
        page.click(".reef-app-runner.picked button[data-act=start]")
        card = ".reef-app-question[data-prompt=ask-Q1]"
        page.wait_for_selector(card, timeout=20000)
        choices = page.locator(f"{card} [data-choice]").evaluate_all("els => els.map(e => e.dataset.choice)")
        legend = page.inner_text(f"{card} legend")
        waits = page.is_disabled(f"{card} button[data-act=answer]")
        page.screenshot(path=str(SHOTS / "reef-app-question.png"))
        # a pick, then the answer
        page.click(f"{card} [data-choice=Hi]")
        ready = page.is_enabled(f"{card} button[data-act=answer]")
        page.click(f"{card} button[data-act=answer]")
        page.wait_for_function("() => document.querySelector('.reef-app-transcript').innerText"
                               ".includes('You picked Hi.')", timeout=20000)
        # Q2's question, answered in the user's own words
        card = ".reef-app-question[data-prompt=ask-Q2]"
        page.wait_for_selector(card, timeout=20000)
        page.fill(f"{card} .reef-app-own", "Howdy")
        page.click(f"{card} button[data-act=answer]")
        page.wait_for_function("() => document.querySelector('.reef-app-transcript').innerText"
                               ".includes('You picked Howdy.')", timeout=20000)
        said = page.inner_text(".reef-app-transcript")
        check("C46 a question is answered in the app, and the session carries on",
              choices == ["Hello", "Hi"] and "Greeting" in legend and "Which greeting for Q1?" in legend
              and waits and ready and "status: done" in reef("node", "get", "Q1", "--plan", "asks")
              and "Answered: Which greeting for Q1? → Hi" in said,
              {"choices": choices, "legend": legend, "answer waits for a pick": waits, "then ready": ready,
               "said": said[-500:]})
    finally:
        stop_runner(process)


def c47_a_rejected_plan_comes_back_revised(page):
    """C47: a session's plan shows rendered, a rejection waits for feedback, the plan comes back
    revised from it, and an approval carries the session on to the node's end."""
    reef("plan", "new", "plans")
    put("plans", node_yaml("P1", "P1 The planned one"))
    process = runner("plans", "plan")
    try:
        open_sessions(page, "plans")
        page.click(".reef-app-runner.picked button[data-act=start]")
        card = ".reef-app-plan-card[data-prompt=plan-P1-1]"
        page.wait_for_selector(card, timeout=20000)
        heading = page.inner_text(f"{card} .reef-app-plan-body h1")
        waits = page.is_disabled(f"{card} button[data-act=reject]")
        page.fill(f"{card} textarea", "Name the test that covers it.")
        page.screenshot(path=str(SHOTS / "reef-app-plan.png"))
        page.click(f"{card} button[data-act=reject]")
        # the plan again, revised from the feedback
        card = ".reef-app-plan-card[data-prompt=plan-P1-2]"
        page.wait_for_selector(card, timeout=20000)
        revised = page.inner_text(f"{card} .reef-app-plan-body")
        page.click(f"{card} button[data-act=approve]")
        page.wait_for_function("() => document.querySelector('.reef-app-transcript').innerText"
                               ".includes('Plan 2 was approved.')", timeout=20000)
        page.wait_for_function("() => document.querySelector('.reef-app-runner.picked .reef-app-session-state')"
                               ".innerText === 'idle'", timeout=20000)
        kept = page.locator(".reef-app-entry.asked details").count()
        said = page.inner_text(".reef-app-transcript")
        check("C47 a rejected plan comes back revised, and an approved one carries on",
              heading == "Plan for P1" and waits and "revised" in revised
              and "Name the test that covers it." in revised and kept == 2
              and "Answered: rejected: Name the test that covers it." in said
              and "Answered: approved" in said and "nothing else is ready to take" in said
              and "status: done" in reef("node", "get", "P1", "--plan", "plans"),
              {"heading": heading, "reject waits for feedback": waits, "revised": revised, "plans kept": kept,
               "said": said[-500:]})
    finally:
        stop_runner(process)


def api(method, path, body=None):
    """Calls the JSON API as reef's own clients do, returning its status and its answer."""
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(f"{URL}{path}", data=data, method=method,
                                     headers={"content-type": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=10) as answer:
            return answer.status, json.load(answer)
    except urllib.error.HTTPError as error:
        return error.code, json.load(error)


def c84_an_attached_session_streams_its_transcript(page):
    """C84: a terminal's own Claude Code session, attached through the API as `! reef loop run`
    attaches it, shows beside the runners with its pane and its loop's node, its transcript
    growing as its follower reports in steps, with no reload, and no buttons, prompt cards or
    reply box, since its terminal answers it; once detached it shows ended."""
    reef("plan", "new", "terminal")
    put("terminal", node_yaml("T1", "T1 Worked in a terminal"))
    pane = "claude:4242"
    # the terminal's loop, holding T1, as `! reef loop start` and next_task leave it
    reef("loop", "start", "--pane", pane, "--plan", "terminal", "--no-open")
    reef("loop", "take", "--pane", pane)
    session = "c84-check"
    attach = {"id": session, "pane": pane, "project": "check", "plan": "terminal",
              "host": "check-host", "cwd": str(WORK / "repo"), "state": "working", "claude": "s1"}
    attached, _ = api("POST", "/api/v1/sessions", attach)
    row = f".reef-app-runner.attached[data-session={session}]"
    try:
        page.goto(URL + "/plans/check/terminal")
        wait_drawn(page)
        page.evaluate("() => { window.__kept = 'same page'; }")
        page.click(".reef-app-sessions-toggle")
        page.wait_for_selector(row, timeout=20000)
        page.click(row)
        shown = page.inner_text(row)
        # entries reported in steps, each shown as it comes
        question = {"question": "Which greeting?", "header": "Greeting", "multiSelect": False,
                    "options": [{"label": "Hello"}, {"label": "Hi"}]}
        steps = [
            [{"at": 1, "kind": "user", "text": "Add a greeting"}],
            [{"at": 2, "kind": "text", "text": "step one"},
             {"at": 3, "kind": "asked", "prompt": {"id": "toolu_q1", "kind": "question", "questions": [question]}}],
            [{"at": 4, "kind": "answered", "prompt": "toolu_q1",
              "answer": {"answer": "answers", "answers": {"Which greeting?": "Hi"}}},
             {"at": 5, "kind": "text", "text": "step two"}],
        ]
        counts = []
        for k, entries_now in enumerate(steps):
            api("POST", f"/api/v1/sessions/{session}", {"entries": entries_now, "state": "working", "claude": "s1"})
            total = sum(len(step) for step in steps[:k + 1])
            page.wait_for_function(
                f"() => document.querySelectorAll('.reef-app-transcript .reef-app-entry').length === {total}",
                timeout=10000)
            counts.append(entries(page))
        said = page.inner_text(".reef-app-transcript")
        summary = page.inner_text(".reef-app-session-sum")
        acts = page.locator(f"{row} button").count()
        cards = page.locator(".reef-app-prompt").count() + page.locator(".reef-app-reply").count()
        # the app's act is refused, since its terminal answers it
        refused, acted = api("POST", f"/api/v1/sessions/{session}/orders", {"act": "start"})
        page.screenshot(path=str(SHOTS / "reef-app-attached-session.png"))
        # detached, it shows ended
        api("DELETE", f"/api/v1/sessions/{session}")
        page.wait_for_selector(f".reef-app-ended {row}", state="attached", timeout=10000)
        check("C84 an attached session's transcript shows live, to read",
              attached == 200 and "On T1" in shown and "Claude Code process 4242" in shown
              and counts == [1, 3, 5] and "Asked:" in said and "Which greeting? → Hi" in said
              and "Terminal session on" in summary and "answered in its terminal" in summary
              and acts == 0 and cards == 0 and refused == 409 and "in-terminal" in acted
              and page.evaluate("() => window.__kept") == "same page",
              {"row": shown, "counts": counts, "summary": summary, "buttons": acts, "cards": cards,
               "refused": refused, "said": said[-400:]})
    finally:
        reef("loop", "stop", "--pane", pane)


def keep_decisions(plan, node, decisions):
    """Keeps a node's decisions through the JSON API, as reef's hooks do."""
    body = json.dumps({"decisions": decisions}).encode()
    url = f"{URL}/api/v1/projects/check/plans/{plan}/nodes/{node}/decisions"
    request = urllib.request.Request(url, data=body, headers={"content-type": "application/json"})
    with urllib.request.urlopen(request, timeout=10) as answer:
        return json.load(answer)


def c76_a_nodes_decisions_show_in_its_sidebar(page):
    """C76: a node's Decisions tab shows each question with the choice picked, each plan with its
    verdict and feedback, the last approved one open, and a new decision appears live."""
    reef("plan", "new", "decide")
    put("decide", node_yaml("D1", "D1 Decided on"))
    question = {"question": "Which greeting?", "header": "Greeting", "multiSelect": False,
                "options": [{"label": "Hello", "description": "The usual"}, {"label": "Hi", "description": "Shorter"}]}
    keep_decisions("decide", "D1", [
        {"call": "q1", "session": "s1", "via": "terminal", "asked_at": 1795400000000, "answered_at": 1795400005000,
         "kind": "question", "questions": [question],
         "outcome": {"outcome": "answered", "answers": {"Which greeting?": "Hi"}}},
        {"call": "p1", "session": "s1", "via": "terminal", "asked_at": 1795400010000, "answered_at": 1795400020000,
         "kind": "plan", "plan": "# Plan\n\nWrite it.",
         "outcome": {"outcome": "rejected", "feedback": "In capitals, please."}},
        {"call": "p2", "session": "s1", "via": "app", "asked_at": 1795400030000, "answered_at": 1795400040000,
         "kind": "plan", "plan": "# Plan, revised\n\nWrite it in capitals.", "outcome": {"outcome": "approved"}},
    ])
    page.goto(URL + "/plans/check/decide")
    wait_drawn(page)
    page.locator(".reef-viewport .reef-node", has_text="D1 Decided on").click()
    page.wait_for_selector(".reef-side", timeout=5000)
    page.click("button[data-tab=decisions]")
    page.wait_for_selector(".reef-app-decision[data-decision='3']", timeout=10000)
    page.evaluate("() => { window.__kept = 'same page'; }")
    cards = page.locator(".reef-app-decision").evaluate_all(
        "els => els.map(e => [e.dataset.decision, e.querySelector('.reef-app-verdict').innerText])")
    picked = page.locator(".reef-app-decision-choices li.picked").evaluate_all("els => els.map(e => e.dataset.choice)")
    feedback = page.inner_text(".reef-app-decision.rejected .reef-app-feedback")
    opened = page.locator(".reef-app-decision-plan[open]").evaluate_all(
        "els => els.map(e => e.closest('.reef-app-decision').dataset.decision)")
    heading = page.inner_text(".reef-app-decision[data-decision='3'] .reef-app-plan-body h1")
    where = page.inner_text(".reef-app-decision[data-decision='3'] .reef-app-decision-when")
    page.screenshot(path=str(SHOTS / "reef-app-decisions.png"))
    # a question answered in the user's own words, kept while the tab is open
    keep_decisions("decide", "D1", [
        {"call": "q2", "session": "s2", "via": "terminal", "asked_at": 1795400050000, "answered_at": 1795400055000,
         "kind": "question", "questions": [question],
         "outcome": {"outcome": "answered", "answers": {"Which greeting?": "Howdy"}}},
    ])
    page.wait_for_selector(".reef-app-decision[data-decision='4']", timeout=10000)
    own = page.inner_text(".reef-app-decision[data-decision='4'] .reef-app-own-answer")
    check("C76 a node's decisions show in its sidebar, a new one live",
          cards == [["1", "Answered"], ["2", "Rejected"], ["3", "Approved"]] and picked == ["Hi"]
          and feedback == "In capitals, please." and opened == ["3"] and heading == "Plan, revised"
          and where.startswith("in reef's app") and own.endswith("Howdy")
          and page.evaluate("() => window.__kept") == "same page",
          {"cards": cards, "picked": picked, "feedback": feedback, "open": opened, "heading": heading,
           "where": where, "own words": own})


def c13_every_plan_opens(page):
    """C13: every plan the picker lists opens its own page."""
    page.goto(URL + "/")
    page.wait_for_selector(".reef-app-plans a")
    links = page.eval_on_selector_all(".reef-app-plans a", "links => links.map(a => a.textContent)")
    opened = []
    for name in links:
        # open it from the picker, and see the page name its plan
        page.goto(URL + "/")
        page.click(f".reef-app-plans a:text-is('{name}')")
        page.wait_for_selector(".reef-app-plan-name")
        opened.append(page.inner_text(".reef-app-plan-name"))
    page.screenshot(path=str(SHOTS / "reef-app-plan.png"))
    check("C13 every plan opens from the picker", opened == [f"check/{name}" for name in links] and len(links) >= 2,
          {"listed": links, "opened": opened})


def signed_reef(*args):
    """Runs the reef CLI against C48's server, with a credentials file of the check's own."""
    return run(REEF / "target/debug/reef", "--conf", SIGN_IN_WORK / "reef.toml",
               "--credentials", SIGN_IN_WORK / "credentials.toml", *args, cwd=SIGN_IN_WORK / "repo")


def c48_a_browser_signs_in_and_reaches_its_plan(browser):
    """C48: a browser that signs in reaches its plan, and signing out sends it back to sign in."""
    # a server of its own that asks every caller to sign in, with a user, a project and a plan
    if SIGN_IN_WORK.exists():
        shutil.rmtree(SIGN_IN_WORK)
    repo = SIGN_IN_WORK / "repo"
    repo.mkdir(parents=True)
    run("git", "-C", repo, "-c", "init.defaultBranch=main", "init", "-q")
    (SIGN_IN_WORK / "reef.toml").write_text(
        f'[api]\naddress = "{SIGN_IN_URL.removeprefix("http://")}"\n\n'
        f'[shoal]\ndata = "{SIGN_IN_WORK / "shoal"}"\ncores = 1\n\n[auth]\nrequired = true\n'
    )
    server = start(SIGN_IN_WORK / "reef.toml", SIGN_IN_URL, SIGN_IN_WORK / "server.log")
    try:
        password = SIGN_IN_WORK / "password"
        password.write_text(PASSWORD + "\n")
        signed_reef("user", "add", "ada", "--password-file", password)
        signed_reef("login", "--user", "ada", "--password-file", password)
        signed_reef("project", "add", "check", "--repo", repo)
        signed_reef("plan", "new", "first")
        first = SIGN_IN_WORK / "a1.yaml"
        first.write_text(node_yaml("A1", "A1 The first"))
        signed_reef("node", "put", first)
        context = browser.new_context(viewport={"width": 1400, "height": 900})
        page = context.new_page()
        page.on("pageerror", lambda e: logs.append(f"pageerror: {e}"))
        # a plan's page sends a browser that isn't signed in to sign in, and back after
        page.goto(SIGN_IN_URL + "/plans/check/first")
        landed = page.url.removeprefix(SIGN_IN_URL)
        # a wrong password stays there, saying so
        page.fill("#name", "ada")
        page.fill("#password", "not the password")
        page.click("button[type=submit]")
        page.wait_for_selector(".problem")
        wrong = (page.url.removeprefix(SIGN_IN_URL).startswith("/sign-in"), page.inner_text(".problem"))
        page.screenshot(path=str(SHOTS / "reef-app-sign-in.png"))
        # the right one reaches the plan, signed in as ada
        page.fill("#password", PASSWORD)
        page.click("button[type=submit]")
        page.wait_for_selector(".reef-app-plan-name", timeout=20000)
        wait_drawn(page)
        reached = (page.url.removeprefix(SIGN_IN_URL), page.inner_text(".reef-app-plan-name"),
                   page.inner_text(".reef-app-user-name"))
        cookie = next(c for c in context.cookies() if c["name"] == "reef_sign_in")
        flags = {"httpOnly": cookie["httpOnly"], "sameSite": cookie["sameSite"], "secure": cookie["secure"]}
        # the change stream carries the cookie: a put from the terminal shows without a reload
        second = SIGN_IN_WORK / "a2.yaml"
        second.write_text(node_yaml("A2", "A2 The second"))
        signed_reef("node", "put", second)
        page.wait_for_function("() => document.querySelector('.reef-viewport').textContent.includes('A2 The second')",
                               timeout=10000)
        # the page's own fetches carry it, and another browser's don't
        inside = page.evaluate("() => fetch('/api/v1/projects').then(r => r.status)")
        other = browser.new_context()
        stranger = other.new_page()
        # from the stylesheet, a page of this origin anyone may open, since the sign-in page's
        # policy lets no script fetch anything
        stranger.goto(SIGN_IN_URL + "/reef.css")
        outside = stranger.evaluate("() => fetch('/api/v1/projects').then(r => r.status)")
        blocked = stranger.goto(SIGN_IN_URL + "/sign-in") and stranger.evaluate(
            "() => fetch('/api/v1/me').then(() => 'fetched', () => 'blocked')")
        other.close()
        page.screenshot(path=str(SHOTS / "reef-app-signed-in.png"))
        # signing out sends it back to sign in, and its old cookie signs nothing in
        kept = context.cookies()
        page.click(".reef-app-user button[type=submit]")
        page.wait_for_url("**/sign-in**")
        signed_out = page.url.removeprefix(SIGN_IN_URL)
        context.add_cookies(kept)
        page.goto(SIGN_IN_URL + "/plans/check/first")
        again = page.url.removeprefix(SIGN_IN_URL)
        context.close()
        check("C48 a browser that signs in reaches its plan, and signing out sends it back to sign in",
              landed == "/sign-in?next=%2Fplans%2Fcheck%2Ffirst" and wrong[0] and "don't match" in wrong[1]
              and reached == ("/plans/check/first", "check/first", "ada")
              and flags == {"httpOnly": True, "sameSite": "Strict", "secure": False}
              and inside == 200 and outside == 401 and blocked == "blocked" and signed_out == "/sign-in"
              and again.startswith("/sign-in?next="),
              {"landed": landed, "wrong": wrong, "reached": reached, "cookie": flags, "fetch inside": inside,
               "fetch outside": outside, "sign-in page fetches": blocked, "signed out": signed_out, "old cookie": again})
    finally:
        stop(server)


def origin_reef(*args):
    """Runs the reef CLI against C51's server, over its loopback port, with credentials of its own."""
    return run(REEF / "target/debug/reef", "--conf", ORIGIN_WORK / "reef.toml",
               "--credentials", ORIGIN_WORK / "credentials.toml", *args, cwd=ORIGIN_WORK / "repo")


class QuietPages(http.server.SimpleHTTPRequestHandler):
    """Serves the page of another origin, saying nothing of each request."""

    def log_message(self, format, *args):
        """Logs nothing, so the check's output stays its own."""


ELSEWHERE_PAGE = """<!doctype html>
<title>Another origin</title>
<form id="out" method="post" action="{url}/sign-out"><button>Sign out</button></form>
<form id="queue" method="post" action="{url}/app/queue" enctype="text/plain">
<input name='{{"project":"check","plan":"first","' value='"}}'><button>Queue</button></form>
"""


def c51_a_cross_origin_write_is_refused(browser):
    """C51: a page on another port of this host posts with the cookie and is refused, while the app's own writes
    go through, and over TLS the app is reached with a Secure cookie."""
    # a server of its own, over plain HTTP and TLS, asking every caller to sign in, with a user and a plan
    if ORIGIN_WORK.exists():
        shutil.rmtree(ORIGIN_WORK)
    repo = ORIGIN_WORK / "repo"
    repo.mkdir(parents=True)
    run("git", "-C", repo, "-c", "init.defaultBranch=main", "init", "-q")
    run("openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
        "-keyout", ORIGIN_WORK / "key.pem", "-out", ORIGIN_WORK / "cert.pem", "-days", "1", "-subj", "/CN=localhost",
        "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1")
    (ORIGIN_WORK / "reef.toml").write_text(
        f'[api]\naddress = "{ORIGIN_URL.removeprefix("http://")}"\n\n'
        f'[api.tls]\naddress = "{ORIGIN_TLS.removeprefix("https://")}"\n'
        f'cert = "{ORIGIN_WORK / "cert.pem"}"\nkey = "{ORIGIN_WORK / "key.pem"}"\n\n'
        f'[shoal]\ndata = "{ORIGIN_WORK / "shoal"}"\ncores = 1\n\n[auth]\nrequired = true\n'
    )
    server = start(ORIGIN_WORK / "reef.toml", ORIGIN_URL, ORIGIN_WORK / "server.log")
    # the page of another origin, on another port of the same host, so the same site
    pages = ORIGIN_WORK / "elsewhere"
    pages.mkdir()
    (pages / "index.html").write_text(ELSEWHERE_PAGE.format(url=ORIGIN_URL))
    handler = functools.partial(QuietPages, directory=str(pages))
    elsewhere = http.server.ThreadingHTTPServer(("127.0.0.1", int(ELSEWHERE.rsplit(":", 1)[1])), handler)
    threading.Thread(target=elsewhere.serve_forever, daemon=True).start()
    try:
        password = ORIGIN_WORK / "password"
        password.write_text(PASSWORD + "\n")
        origin_reef("user", "add", "ada", "--password-file", password)
        origin_reef("login", "--user", "ada", "--password-file", password)
        origin_reef("project", "add", "check", "--repo", repo)
        origin_reef("plan", "new", "first")
        first = ORIGIN_WORK / "a1.yaml"
        first.write_text(node_yaml("A1", "A1 The first"))
        origin_reef("node", "put", first)

        def signed_in(context, root):
            """Signs a context in on a root's sign-in page, and waits for its plan to draw."""
            page = context.new_page()
            page.on("pageerror", lambda e: logs.append(f"pageerror: {e}"))
            page.goto(root + "/plans/check/first")
            page.fill("#name", "ada")
            page.fill("#password", PASSWORD)
            page.click("button[type=submit]")
            page.wait_for_selector(".reef-app-plan-name", timeout=20000)
            wait_drawn(page)
            return page

        def still_signed_in(page):
            """Whether the page's cookie still signs ada in, as the app's own call says."""
            return page.evaluate("() => fetch('/app/me').then(r => r.json()).then(me => me.user && me.user.name)")

        # over plain HTTP: signed in, then the other origin's forms post with the cookie
        context = browser.new_context(viewport={"width": 1400, "height": 900})
        page = signed_in(context, ORIGIN_URL)
        cookie = next(c for c in context.cookies() if c["name"] == "reef_sign_in")
        refused = {}
        for form in ["out", "queue"]:
            page.goto(ELSEWHERE + "/")
            with page.expect_navigation() as answer:
                page.click(f"#{form} button")
            refused[form] = (answer.value.status, "its own pages" in page.content())
        page.goto(ORIGIN_URL + "/plans/check/first")
        page.wait_for_selector(".reef-app-plan-name", timeout=20000)
        kept = still_signed_in(page)
        # the app's own page writes: a put from its own fetch, and its own Sign out
        puts = {"puts": [{"node": {"spec": {"id": "A2", "title": "A2 From the page", "description": {
            "why": "Without A2, something is missing.", "what": "With A2, it is there."}},
            "body": "# A2\n", "order": 1}, "read": None}]}
        own = page.evaluate("""(puts) => fetch('/api/v1/projects/check/plans/first/nodes', {method: 'POST',
            headers: {'content-type': 'application/json'}, body: JSON.stringify(puts)}).then(r => r.status)""", puts)
        page.screenshot(path=str(SHOTS / "reef-app-cross-origin.png"))
        page.click(".reef-app-user button[type=submit]")
        page.wait_for_url("**/sign-in**")
        signed_out = page.url.removeprefix(ORIGIN_URL)
        context.close()
        # over TLS: the app reached on its other listener, its cookie Secure
        secure = browser.new_context(viewport={"width": 1400, "height": 900}, ignore_https_errors=True)
        page = signed_in(secure, ORIGIN_TLS)
        tls_cookie = next(c for c in secure.cookies() if c["name"] == "reef_sign_in")
        tls_name = page.inner_text(".reef-app-user-name")
        page.screenshot(path=str(SHOTS / "reef-app-over-tls.png"))
        secure.close()
        check("C51 a cross-origin write is refused, and the same page's own writes go through",
              refused == {"out": (403, True), "queue": (403, True)} and kept == "ada" and own == 200
              and signed_out == "/sign-in" and not cookie["secure"] and tls_cookie["secure"] and tls_name == "ada",
              {"refused": refused, "still signed in": kept, "own put": own, "signed out": signed_out,
               "plain cookie secure": cookie["secure"], "tls cookie secure": tls_cookie["secure"], "over tls": tls_name})
    finally:
        elsewhere.shutdown()
        stop(server)


def account_reef(who, *args, check_ok=True):
    """Runs the reef CLI against C77's server as one user, each with a credentials file of their own."""
    line = [REEF / "target/debug/reef", "--conf", ACCOUNT_WORK / "reef.toml",
            "--credentials", ACCOUNT_WORK / f"{who}.toml", *args]
    done = subprocess.run(line, cwd=ACCOUNT_WORK / "repo", capture_output=True, text=True)
    if check_ok and done.returncode != 0:
        raise RuntimeError(f"{' '.join(map(str, line))} failed: {done.stderr}")
    return done.stdout + done.stderr


def c77_the_account_page_manages_passwords_and_tokens(browser):
    """C77: the account page changes a password, revokes a token and, for an admin, sets another's password and
    removes a user, and the sign-in page says to wait after repeated wrong passwords."""
    # a server of its own that asks every caller to sign in, with ada, an admin, and bob, each logged in at a terminal
    if ACCOUNT_WORK.exists():
        shutil.rmtree(ACCOUNT_WORK)
    repo = ACCOUNT_WORK / "repo"
    repo.mkdir(parents=True)
    run("git", "-C", repo, "-c", "init.defaultBranch=main", "init", "-q")
    (ACCOUNT_WORK / "reef.toml").write_text(
        f'[api]\naddress = "{ACCOUNT_URL.removeprefix("http://")}"\n\n'
        f'[shoal]\ndata = "{ACCOUNT_WORK / "shoal"}"\ncores = 1\n\n[auth]\nrequired = true\n'
    )
    server = start(ACCOUNT_WORK / "reef.toml", ACCOUNT_URL, ACCOUNT_WORK / "server.log")
    try:
        old = ACCOUNT_WORK / "old"
        old.write_text(PASSWORD + "\n")
        new_password = "a newer horse battery"
        new = ACCOUNT_WORK / "new"
        new.write_text(new_password + "\n")
        account_reef("ada", "user", "add", "ada", "--password-file", old)
        account_reef("ada", "login", "--user", "ada", "--password-file", old, "--label", "ada's terminal")
        account_reef("ada", "user", "add", "bob", "--password-file", old)
        account_reef("bob", "login", "--user", "bob", "--password-file", old, "--label", "bob's terminal")
        terminal = next(line.split()[0] for line in account_reef("ada", "token", "ls").splitlines()
                        if "(this reef)" in line)
        # ada signs in in a browser, and comes to her account page
        context = browser.new_context(viewport={"width": 1200, "height": 900})
        page = context.new_page()
        page.on("pageerror", lambda e: logs.append(f"pageerror: {e}"))
        page.goto(ACCOUNT_URL + "/account")
        page.fill("#name", "ada")
        page.fill("#password", PASSWORD)
        page.click("button[type=submit]")
        page.wait_for_selector(".reef-app-account table", timeout=20000)
        landed = page.url.removeprefix(ACCOUNT_URL)
        # once the page's script runs, a click opens bob's password field; before it, a click does nothing
        bob_row = 'tr[data-user="bob"]'
        for _ in range(40):
            page.click(f"{bob_row} button[data-act=set-password]")
            if page.query_selector(f"{bob_row} input[type=password]"):
                break
            time.sleep(0.25)
        page.click(f"{bob_row} button[data-act=keep]")
        # her tokens: the terminal's and this browser's, which is marked and has no revoke
        rows = page.eval_on_selector_all(".reef-app-account-part:nth-of-type(2) tbody tr",
                                         "rows => rows.map(r => [r.dataset.token, r.innerText])")
        marked = [text for _, text in rows if "this browser" in text]
        # the terminal's revoked from here, and the terminal is signed in no more
        page.click(f'tr[data-token="{terminal}"] button[data-act=revoke]')
        page.wait_for_selector(f'tr[data-token="{terminal}"]', state="detached", timeout=10000)
        revoked_said = page.inner_text(".reef-app-account-said")
        terminal_after = account_reef("ada", "whoami")
        # her own password changed with her current one
        page.fill("#account-current", PASSWORD)
        page.fill("#account-new", new_password)
        page.fill("#account-again", new_password)
        page.click("button[data-act=change-password]")
        page.wait_for_function(
            "() => [...document.querySelectorAll('.reef-app-account-said')].some(p => p.textContent.includes('password is changed'))",
            timeout=10000)
        old_login = account_reef("ada", "login", "--user", "ada", "--password-file", old, check_ok=False)
        new_login = account_reef("ada", "login", "--user", "ada", "--password-file", new)
        # as an admin she sets bob's, which revokes his terminal's token
        page.click(f"{bob_row} button[data-act=set-password]")
        page.fill(f"{bob_row} input[type=password]", new_password)
        page.click(f"{bob_row} button[data-act=confirm-password]")
        page.wait_for_function(
            "() => [...document.querySelectorAll('.reef-app-account-said')].some(p => p.textContent.includes('Set the password of bob'))",
            timeout=10000)
        set_said = [text for text in page.eval_on_selector_all(".reef-app-account-said", "ps => ps.map(p => p.textContent)")
                    if "bob" in text]
        bob_after = account_reef("bob", "whoami")
        page.screenshot(path=str(SHOTS / "reef-app-account.png"))
        # and removes him, once she confirms it
        page.click(f"{bob_row} button[data-act=remove]")
        page.click(f"{bob_row} button[data-act=confirm-remove]")
        page.wait_for_selector(bob_row, state="detached", timeout=10000)
        users = account_reef("ada", "user", "ls")
        own_row = page.inner_text('tr[data-user="ada"]')
        context.close()
        # a browser that tries a wrong password over and over is told to wait, even with the right one
        stranger = browser.new_context()
        guess = stranger.new_page()
        guess.goto(ACCOUNT_URL + "/sign-in")
        for _ in range(5):
            guess.fill("#name", "ada")
            guess.fill("#password", "not the password")
            guess.click("button[type=submit]")
            guess.wait_for_selector(".problem")
        guess.fill("#name", "ada")
        guess.fill("#password", new_password)
        with guess.expect_navigation() as answer:
            guess.click("button[type=submit]")
        waited = (answer.value.status, guess.inner_text(".problem"))
        guess.screenshot(path=str(SHOTS / "reef-app-sign-in-wait.png"))
        stranger.close()
        check("C77 the account page changes a password, revokes a token and, for an admin, removes a user",
              landed == "/account" and len(rows) == 2 and len(marked) == 1 and "Revoke" not in marked[0]
              and f"Revoked {terminal}" in revoked_said and terminal_after.startswith("nobody")
              and "don't match" in old_login and "logged in to" in new_login
              and any("revoking 1 token" in text for text in set_said) and bob_after.startswith("nobody")
              and users.split() == ["ada", "admin"] and "(you)" in own_row
              and waited[0] == 429 and "try again in" in waited[1],
              {"landed": landed, "tokens": rows, "revoked": revoked_said, "terminal after": terminal_after.strip(),
               "old login": old_login.strip(), "new login": new_login.strip(), "set": set_said,
               "bob after": bob_after.strip(), "users": users.strip(), "own row": own_row, "waited": waited})
    finally:
        stop(server)


def main():
    setup()
    server = start()
    try:
        # the check's project, with two plans of one node each, all through the CLI
        reef("project", "add", "check", "--repo", str(WORK / "repo"))
        for plan in ["first", "second"]:
            reef("plan", "new", plan)
            put(plan, node_yaml("A1", f"A1 The first of {plan}"))
        with sync_playwright() as p:
            browser = p.chromium.launch()
            page = browser.new_page(viewport={"width": 1400, "height": 900})
            # a debug client looks for dx's hot-reload socket, which only `dx serve` answers
            page.on("console", lambda m: "/_dioxus" in m.text or logs.append(f"console.{m.type}: {m.text}"))
            page.on("pageerror", lambda e: logs.append(f"pageerror: {e}"))
            checks = [
                c13_every_plan_opens,
                c54_draws_as_in_mdbook,
                c54_selecting_opens_the_panel,
                c54_one_viewer_listens,
                c55_matches_the_viewer,
                c55_answers_the_pointer,
                c15_a_put_shows_live,
                c20_a_stale_save_shows_the_newer,
                c25_a_drag_writes_one_row,
                c25_keys_and_the_sidebar_queue,
                c28_a_linked_page_shows_in_the_sidebar,
                c28_an_edited_page_shows_live,
                c68_a_node_link_opens_its_node,
                c69_a_body_put_shows_in_the_sidebar,
                c33_changes_in_the_sidebar,
                c35_comment_threads_in_the_diff,
                c36_comments_follow_the_code,
                c73_a_review_holds_comments_until_submitted,
                c56_drafts_in_the_sidebar,
                c45_a_loop_started_from_the_app_streams_its_transcript,
                c46_a_question_is_answered_in_the_app,
                c47_a_rejected_plan_comes_back_revised,
                c84_an_attached_session_streams_its_transcript,
                c76_a_nodes_decisions_show_in_its_sidebar,
            ]
            for step in checks:
                # every check, or those `--only` names
                if not ONLY or any(step.__name__.startswith(name) for name in ONLY):
                    step(page)
            # C48's on a server of its own, once the first has let go of nothing it needs
            stop(server)
            if not ONLY or any("c48_a_browser_signs_in_and_reaches_its_plan".startswith(name) for name in ONLY):
                c48_a_browser_signs_in_and_reaches_its_plan(browser)
            if not ONLY or any("c51_a_cross_origin_write_is_refused".startswith(name) for name in ONLY):
                c51_a_cross_origin_write_is_refused(browser)
            if not ONLY or any("c77_the_account_page_manages_passwords_and_tokens".startswith(name) for name in ONLY):
                c77_the_account_page_manages_passwords_and_tokens(browser)
            browser.close()
    finally:
        stop(server)
    # anything the page logged, then what failed
    print("--- console and errors")
    for line in logs:
        print(line)
    print("--- failed:", json.dumps(failed) if failed else "none")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
