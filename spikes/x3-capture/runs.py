"""Writes one live run's timeline as markdown: each session's tool calls, the triggers' events
and the revisions they left.

    python3 -I runs.py <run dir> <title> <out.md> <session>...

<run dir> holds the state dir setup.sh wrote and each session's `<session>.jsonl` from
drive.sh; paths under it are written relative to it.
"""
import json, sys, re
run_dir, title, out = sys.argv[1], sys.argv[2], sys.argv[3]
sessions = sys.argv[4:]
events = [json.loads(l) for l in open(f"{run_dir}/state/events.jsonl") if l.strip()]
t0 = events[0]["at"]
def short(v):
    return v[:7] if isinstance(v, str) and re.fullmatch(r"[0-9a-f]{40}", v) else v
lines = [f"# {title}", ""]
for name in sessions:
    rows = [json.loads(l) for l in open(f"{run_dir}/{name}.jsonl") if l.strip()]
    init = next(r for r in rows if r.get("type") == "system" and r.get("subtype") == "init")
    result = next(r for r in rows if r.get("type") == "result")
    cwd = init["cwd"].replace(run_dir + "/", "")
    lines.append(f"## Session {name.upper()}, in `{cwd}`")
    lines.append("")
    lines.append(f"{result['num_turns']} turns, ${result['total_cost_usd']:.2f}, {result['duration_ms']/1000:.1f} s, model {init.get('model')}.")
    lines.append("")
    for r in rows:
        if r.get("type") != "assistant":
            continue
        for c in r["message"]["content"]:
            if c.get("type") != "tool_use":
                continue
            inp = c["input"]
            what = inp.get("command") or inp.get("file_path") or json.dumps(inp)
            what = what.replace(run_dir + "/", "").split("\n")[0][:110]
            denied = any(d["tool_use_id"] == c["id"] for d in result["permission_denials"])
            lines.append(f"- {c['name']}: `{what}`" + (" (denied)" if denied else ""))
    lines.append("")
lines.append("## The triggers")
lines.append("")
lines.append("| s | Event | What |")
lines.append("|---:|---|---|")
for e in events:
    b = dict(e["body"])
    for k in ("work", "session", "cwd"):
        b.pop(k, None)
    if isinstance(b.get("worktree"), str):
        b["worktree"] = b["worktree"].split("/")[-1]
    b = {k: short(v) for k, v in b.items() if v is not None}
    if e["kind"] == "PostToolUse":
        b = {"tool": b.get("tool"), "input": b.get("tool_input")}
    desc = ", ".join(f"{k}: {json.dumps(v) if isinstance(v,(dict,list,bool)) else v}" for k, v in b.items())
    desc = desc.replace(run_dir + "/", "")
    lines.append(f"| {(e['at']-t0)/1000:.1f} | {e['kind']} | {desc} |")
lines.append("")
state = json.load(open(f"{run_dir}/state/state.json"))
lines.append("## The revisions")
lines.append("")
for node, caps in state["captures"].items():
    for c in caps:
        files = ", ".join(f"`{f.get('new_path') or f.get('old_path')}`" for f in c["diff"]["files"])
        lines.append(f"- {node} r{c['revision']}, taken by {c['trigger']}, sealed by {c.get('sealed_by') or 'nothing'}, at {short(c['head'])}: {files}")
lines.append("")
open(out, "w").write("\n".join(lines))
