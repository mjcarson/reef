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
    check("C13 every plan opens from the picker", opened == [f"check/{name}" for name in links] and len(links) == 2,
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
