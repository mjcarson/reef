"""Drives reef-app in headless Chromium, over plans put through the `reef` CLI.

Each step checks one of M4's criteria in the browser, printing `ok` or `FAILED` and what it saw;
the script exits 1 if any failed.

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
    """Every box and group the diagram draws, as its classes and its text, in order."""
    return page.evaluate(
        "() => [...document.querySelectorAll('.reef-viewport .reef-node, .reef-viewport .reef-group')]"
        ".map(e => [e.getAttribute('class'), e.textContent]).sort()"
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
    """C54: clicking a node in the diagram opens it in the node panel, and letting it go closes it."""
    page.goto(URL + "/plans/check/draws")
    wait_drawn(page)
    page.click("button[data-act=expand]")
    page.wait_for_timeout(300)
    page.locator(".reef-viewport .reef-node", has_text="W2 Build on it").click()
    page.wait_for_selector(".reef-app-panel", timeout=5000)
    shown = (page.inner_text(".reef-app-panel-id"), page.inner_text(".reef-app-panel-title"))
    page.screenshot(path=str(SHOTS / "reef-app-panel.png"))
    page.click(".reef-side [data-act=close]")
    page.wait_for_timeout(300)
    closed = page.locator(".reef-app-panel").count() == 0
    check("C54 selecting a node opens it in the node panel", shown == ("W2", "W2 Build on it") and closed,
          {"panel": shown, "closed with the viewer's sidebar": closed})


def c54_one_viewer_listens(page):
    """C58 in the app: leaving a plan for the picker and coming back leaves one viewer listening."""
    page.goto(URL + "/plans/check/draws")
    wait_drawn(page)
    page.evaluate("() => { window.__old = document.querySelector('.reef'); window.__kept = 'same page'; }")
    page.click("text=← Plans")
    page.wait_for_selector(".reef-app-plans a")
    page.click(".reef-app-plans a:text-is('draws')")
    wait_drawn(page)
    page.wait_for_timeout(300)
    # a filter change reaches every live view through a document event, and a live view draws again
    page.evaluate("() => { window.__old.removeAttribute('data-layout-ms');"
                  " document.querySelector('.reef').removeAttribute('data-layout-ms'); }")
    chip = page.locator(".reef-acc-filter button").first
    chip.click()
    page.wait_for_timeout(500)
    drew = page.evaluate("() => [window.__old.getAttribute('data-layout-ms') !== null,"
                         " document.querySelector('.reef').getAttribute('data-layout-ms') !== null,"
                         " window.__old.isConnected, window.__kept]")
    chip.click()
    check("C54 a plan left and opened again leaves one viewer listening",
          drew == [False, True, False, "same page"],
          {"left view drew": drew[0], "new view drew": drew[1], "left element in the page": drew[2],
           "no reload": drew[3]})


def c15_a_put_shows_live(page):
    """C15: a status set from the CLI shows up without a reload, keeping the groups and selection."""
    page.goto(URL + "/plans/check/draws")
    wait_drawn(page)
    page.click("button[data-act=expand]")
    page.wait_for_timeout(300)
    page.locator(".reef-viewport .reef-node", has_text="W2 Build on it").click()
    page.wait_for_selector(".reef-app-panel", timeout=5000)
    page.evaluate("() => { window.__kept = 'same page'; }")
    badges = "() => [...document.querySelectorAll('.reef-side .reef-badge')].map(b => b.textContent)"
    before = (page.get_attribute(".reef", "data-boxes"), page.evaluate(badges))
    # Claude, or anyone, sets the status from the terminal
    reef("status", "W2", "done", "--plan", "draws")
    page.wait_for_function("() => [...document.querySelectorAll('.reef-side .reef-badge')]"
                           ".some(b => b.textContent === 'done')", timeout=10000)
    after = (page.get_attribute(".reef", "data-boxes"), page.evaluate(badges))
    kept = page.evaluate("() => window.__kept")
    panel = page.inner_text(".reef-app-panel-id") if page.locator(".reef-app-panel").count() else None
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
            c15_a_put_shows_live(page)
            c20_a_stale_save_shows_the_newer(page)
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
