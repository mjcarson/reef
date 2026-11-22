"""Drives reef-app in headless Chromium, over plans put through the `reef` CLI.

Each step checks one of M4's or M5's criteria in the browser, printing `ok` or `FAILED` and what
it saw; the script exits 1 if any failed.

It runs everything itself: it builds the CLI, writes a config of its own (the app on
127.0.0.1:12190, shoal's data under central/target/tmp/reef-app-check), registers a scratch repo,
starts the server dx built, and stops the server's whole process group at the end, since an
orphaned server keeps shoal's data directory locked.

Build the app first, from central/reef-app, with no `RUSTFLAGS` exported:

    env -u RUSTFLAGS dx build @server --platform server --features embedded

then, with Playwright installed:

    python check.py [screenshot dir]
"""

import json
import os
import shutil
import signal
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

# reef's checkout, the app's crate, and where dx put the server it built
APP = Path(__file__).resolve().parent
REEF = APP.parent.parent
SERVER = REEF / "central/target/dx/reef-app/debug/web/server"
# where the check keeps its config, repo and shoal's data: under target, never a tmpfs /tmp
WORK = REEF / "central/target/tmp/reef-app-check"
ADDRESS = "127.0.0.1:12190"
URL = f"http://{ADDRESS}"
SHOTS = Path(sys.argv[1]) if len(sys.argv) > 1 else WORK
# the checks that failed, and everything the page logged
failed = []
logs = []
# the mdbook build of the plan C54 draws, which reef-view draws, for C55 to compare against
BOOK = {}


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


def start():
    """Starts the built server in a process group of its own, and waits for its JSON API."""
    if not SERVER.exists():
        sys.exit(f"no server at {SERVER}: build it with dx first (see this script's docs)")
    env = dict(os.environ, REEF_CONFIG=str(WORK / "reef.toml"))
    env.pop("RUSTFLAGS", None)
    log = open(WORK / "server.log", "w")
    server = subprocess.Popen([SERVER], cwd=SERVER.parent, env=env, stdout=log, stderr=subprocess.STDOUT,
                              start_new_session=True)
    # shoal starts in the process, so the first answer takes a moment
    for _ in range(120):
        try:
            with urllib.request.urlopen(f"{URL}/api/v1/projects", timeout=1) as answer:
                if answer.status == 200:
                    return server
        except OSError:
            time.sleep(0.5)
    stop(server)
    sys.exit(f"the server never answered; see {WORK / 'server.log'}")


def stop(server):
    """Stops the server's whole process group, as a terminal's ctrl-c would."""
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
            c13_every_plan_opens(page)
            c54_draws_as_in_mdbook(page)
            c54_selecting_opens_the_panel(page)
            c54_one_viewer_listens(page)
            c55_matches_the_viewer(page)
            c55_answers_the_pointer(page)
            c15_a_put_shows_live(page)
            c20_a_stale_save_shows_the_newer(page)
            c25_a_drag_writes_one_row(page)
            c25_keys_and_the_sidebar_queue(page)
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
