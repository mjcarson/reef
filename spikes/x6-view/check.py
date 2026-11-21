"""Drives X6's app in headless Chromium and prints what each check saw.

Steps 1 to 5 print what they saw, as X6 ran them. Steps 6 to 8 check C12's handle, and step 5's
last check C58's teardown: each prints `ok` or `FAILED`, and the script exits 1 if any failed.

Run it against `dx serve` with Playwright installed (`pip install playwright` and
`python -m playwright install chromium`):

    python check.py [url] [screenshot dir]
"""

import json
import sys

from playwright.sync_api import sync_playwright

# where the app serves, and where to keep the screenshots
URL = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:8080/"
SHOTS = sys.argv[2] if len(sys.argv) > 2 else "."
# everything the page logged, failed or fetched from the books
logs = []
# the checks that failed
failed = []


def check(label, ok, seen):
    """Prints whether a check held and what it saw, and remembers it if it didn't."""
    print(f"{'ok' if ok else 'FAILED'} {label}: {seen}")
    if not ok:
        failed.append(label)


def transform(page):
    """The diagram's pan and zoom, as the viewport's transform."""
    return page.evaluate("() => document.querySelector('.reef-viewport').getAttribute('transform')")


def selected(page):
    """The node the viewer told the app it selected, as the header shows it."""
    return page.inner_text("#x6-selected")


def drawn_text(page):
    """Every label the diagram draws."""
    return page.evaluate("() => document.querySelector('.reef-viewport').textContent")


def pick(page, label):
    """Picks a plan with the header's buttons, and waits for it to be drawn."""
    page.click(f"text={label}")
    page.wait_for_timeout(2000)


def view(page):
    """The boxes drawn, the last layout's time in ms, and the view's classes."""
    return page.evaluate(
        "() => { const e = document.querySelector('.reef');"
        " return e && [e.getAttribute('data-boxes'), e.getAttribute('data-layout-ms'), e.className]; }"
    )


def goto_node(page, node_id):
    """Selects a node through the viewer's own search, which opens the groups hiding it."""
    page.fill(".reef-search", node_id)
    page.press(".reef-search", "Enter")
    page.wait_for_timeout(300)


def side(page):
    """What the sidebar shows now: its title, status badges, sections and *Read here* text."""
    return page.evaluate(
        """() => { const s = document.querySelector('.reef-side');
        return { hidden: s.hidden, title: (s.querySelector('h2,h3') || {}).textContent,
          why: !!s.querySelector('.reef-desc, .reef-ww'),
          badges: [...s.querySelectorAll('.reef-badge')].map(b => b.textContent),
          acceptance: !!s.querySelector('.reef-acc'),
          charts: s.querySelectorAll('.reef-perf svg, svg.reef-chart').length,
          reads: [...s.querySelectorAll('details.reef-doc-read')]
            .map(d => (d.querySelector('.reef-doc-text') || {}).innerText?.trim().slice(0, 80)) }; }"""
    )


def wait_drawn(page):
    """Waits for the view to be mounted and to hold boxes."""
    page.wait_for_selector(".reef.reef-mounted .reef-node", timeout=20000)


with sync_playwright() as p:
    browser = p.chromium.launch()
    page = browser.new_page(viewport={"width": 1400, "height": 900})
    page.on("console", lambda m: logs.append(f"console.{m.type}: {m.text}"))
    page.on("pageerror", lambda e: logs.append(f"pageerror: {e}"))
    page.on("requestfailed", lambda r: logs.append(f"requestfailed: {r.url}"))
    page.on("requestfinished", lambda r: "/book/" in r.url and logs.append(f"fetched: {r.url}"))

    # 1. the example book draws through mount, from a clean slate
    page.goto(URL)
    wait_drawn(page)
    page.evaluate("() => localStorage.clear()")
    page.reload()
    wait_drawn(page)
    # a marker that only survives as long as the page isn't reloaded
    page.evaluate("() => { window.__x6 = 'same page'; }")
    print("1 drawn:", view(page), "views:", page.locator(".reef-mounted").count())
    page.screenshot(path=f"{SHOTS}/x6-drawn.png")

    # 2. the sidebar: why and what, a doc read here, acceptance and perf charts
    goto_node(page, "O149")
    page.wait_for_timeout(1500)
    print("2a O149:", json.dumps(side(page)))
    page.screenshot(path=f"{SHOTS}/x6-side.png")
    goto_node(page, "O151")
    page.wait_for_timeout(1500)
    print("2b O151:", json.dumps(side(page)))
    print("2c docs fetched:", sorted({l for l in logs if l.startswith("fetched")}))

    # 3. saved view state: open every group, reload, and see them still open
    page.keyboard.press("Escape")
    page.click("button[data-act=expand]")
    page.wait_for_timeout(300)
    expanded = view(page)
    keys = page.evaluate("() => Object.keys(localStorage)")
    page.reload()
    wait_drawn(page)
    page.wait_for_timeout(300)
    print("3 expanded:", expanded, "after a reload:", view(page), "storage keys:", keys)
    page.evaluate("() => { window.__x6 = 'same page'; }")

    # 4. second payloads, swapped in through the handle without a reload
    goto_node(page, "O171")
    page.wait_for_timeout(500)
    print("4a O171 before:", json.dumps(side(page)["badges"]))
    page.click("text=Example, one more done")
    page.wait_for_timeout(2000)
    print("4b swapped:", view(page), "page kept:", page.evaluate("() => window.__x6"))
    goto_node(page, "O171")
    page.wait_for_timeout(500)
    print("4c O171 after:", json.dumps(side(page)["badges"]))
    page.click("text=This plan")
    page.wait_for_timeout(2000)
    print("4d this plan:", view(page), "page kept:", page.evaluate("() => window.__x6"), "views:", page.locator(".reef-mounted").count())
    page.screenshot(path=f"{SHOTS}/x6-plan.png")
    goto_node(page, "X1")
    page.wait_for_timeout(1500)
    print("4e X1 in this plan:", json.dumps(side(page)))
    page.click("text=Example book")
    page.wait_for_timeout(2000)
    print("4f the example again:", view(page), "page kept:", page.evaluate("() => window.__x6"))

    # 5. hide and show: the hidden view's element is gone, but is the view still listening?
    page.evaluate("() => { window.__old = document.querySelector('.reef'); }")
    page.click("text=Hide the view")
    page.wait_for_timeout(500)
    page.click("text=Show the view")
    wait_drawn(page)
    page.wait_for_timeout(500)
    print("5a shown again:", view(page), "old element in the page:", page.evaluate("() => window.__old.isConnected"))
    # a filter change reaches every view through a document event; a live view draws again
    page.evaluate("() => window.__old.removeAttribute('data-layout-ms')")
    chip = page.locator(".reef-acc-filter button").first
    chip.click()
    page.wait_for_timeout(500)
    print("5b the hidden view drew again:", page.evaluate("() => window.__old.getAttribute('data-layout-ms') !== null"))
    chip.click()

    # 6. a newer revision keeps the selection, the open groups, and the pan and zoom
    pick(page, "Example book")
    goto_node(page, "O171")
    page.wait_for_timeout(500)
    before = (view(page)[0], transform(page), side(page)["title"], selected(page))
    pick(page, "Example, one more done")
    after = (view(page)[0], transform(page), side(page)["title"], selected(page))
    check("6a a swap keeps the boxes, the pan and zoom, and the sidebar's node", before[:3] == after[:3] and not side(page)["hidden"], [before, after])
    check("6b the app heard the selection", "O171" in after[3], after[3])
    # the sidebar's close button, since the key goes to whatever has focus, here a button
    page.click(".reef-side [data-act=close]")
    page.wait_for_timeout(300)
    check("6c the app hears the selection let go", "None" in selected(page), selected(page))

    # 7. another plan is drawn from its own view: `central`'s members, not the whole plan
    pick(page, "This plan")
    text = drawn_text(page)
    check("7 this plan is drawn from its root, central", "M4 reef-app" in text and "Centralized reef" not in text, text[:120])

    # 8. two plans drawn on one path keep their own open groups
    pick(page, "Example book")
    page.click("button[data-act=expand]")
    page.wait_for_timeout(300)
    example_open = view(page)[0]
    pick(page, "This plan")
    page.click("button[data-act=collapse]")
    page.wait_for_timeout(300)
    plan_closed = view(page)[0]
    pick(page, "Example book")
    example_again = view(page)[0]
    pick(page, "This plan")
    plan_again = view(page)[0]
    keys = page.evaluate("() => Object.keys(localStorage).filter(k => k.startsWith('reef:plan:')).sort()")
    check("8a the example's groups come back", example_again == example_open, [example_open, example_again])
    check("8b this plan's groups come back", plan_again == plan_closed, [plan_closed, plan_again])
    check("8c each plan is saved under its own key", keys == ["reef:plan:example", "reef:plan:plan"], keys)

    # anything the page logged or failed to load
    print("--- console and errors")
    for line in logs:
        if not line.startswith("fetched"):
            print(line)
    browser.close()

print("--- failed:", failed if failed else "none")
sys.exit(1 if failed else 0)
