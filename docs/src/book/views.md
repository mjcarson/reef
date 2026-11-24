# Drawing a view

````markdown
```reef
root = "scans"          # this group's members (default: the whole plan)
focus = "m23"           # or: this group open, with its outside neighbours faded around it
direction = "LR"        # TB (default) or LR
expand = ["m23steps"]   # groups open at first; or "all" / "none"
height = "80vh"
page = "../plan.md"     # a "Full page" button linking to that chapter
```
````

A fence with `layout = "page"` fills the page instead of the text column. mdbook keeps its
pages 750px wide, and a large plan needs more than that. Such a view takes everything to the
right of the book's sidebar and is as tall as the window. The sidebar stays, so readers leave
the page the way they leave any other. A plain scroll zooms there, and the prev/next arrows are
hidden. Give the view a chapter of its own and link to it from the embedded views with `page`:

````markdown
# The scans plan

```reef
root = "scans-plan"
layout = "page"
```
````

A page holds at most one `layout = "page"` view, and it takes no `height`.

Opening and closing groups is remembered for each page in the browser.

| In the diagram | |
|---|---|
| Click a box | open its sidebar |
| ▸ / ▾, or double-click a collapsed group | open or close a group |
| Drag | pan |
| Ctrl + scroll, or pinch | zoom (plain scroll zooms in full screen and in `layout = "page"`) |
| `/` | search by id or title; a match inside a closed group opens it |
| Hover | lights up everything upstream and downstream |
| `+` `-` `0` `f` `Esc` | zoom in, zoom out, fit, full screen, close the sidebar |
