//! The app's pages: the plan explorer, a plan's own page, and the account page

use crate::account::Account;
use crate::calls::{self, ProjectPlans};
use crate::canvas::{Canvas, Keyed};
use crate::explorer::Explorer;
use crate::live::use_changes;
use crate::queue::{QueuePanel, use_queue};
use crate::session::{SessionPanel, SessionsButton, use_runners};
use crate::sign_in::{CallFailed, SignedInAs};
use dioxus::prelude::*;
use reef_api::{Branched, PlanInfo, Project};

/// The theme reef.css reads with no fallback, which mdbook's themes define and a page outside
/// mdbook lacks, and the shell's own layout
const THEME: &str = r#"
:root { --fg: #333; --bg: #fff; --links: #2b79a2; --muted: #6b6b6b; --line: #ddd; --panel: #f6f6f6;
  --add-bg: #e6ffec; --add-fg: #1a7f37; --del-bg: #ffebe9; --del-fg: #cf222e; --hunk-bg: #ddf4ff; }
@media (prefers-color-scheme: dark) {
  :root { --fg: #ddd; --bg: #1f1f23; --links: #6fb3d9; --muted: #9a9a9a; --line: #3a3a40; --panel: #26262b;
    --add-bg: #12301d; --add-fg: #56d364; --del-bg: #3a1618; --del-fg: #f85149; --hunk-bg: #172a3d; }
}
html, body { margin: 0; height: 100%; font-family: system-ui, sans-serif; color: var(--fg); background: var(--bg); }
a { color: var(--links); }
code { font-size: 0.92em; }
.reef-app-picker { max-width: 52rem; margin: 0 auto; padding: 2rem 1rem; }
.reef-app-picker h1 { margin: 0; }
.reef-app-picker-head { display: flex; gap: 1rem; align-items: baseline; margin: 0 0 1.5rem; }
.reef-app-picker-head .reef-app-user { margin-left: auto; }
.reef-app-user { display: flex; gap: 0.6rem; align-items: baseline; font-size: 0.85rem; color: var(--muted); }
.reef-app-user strong { color: var(--fg); }
.reef-app-user form { margin: 0; }
.reef-app-user a { color: inherit; }
.reef-app-account-part { margin: 0 0 2rem; }
.reef-app-account-part h2 { font-size: 1.1rem; margin: 0 0 0.5rem; }
.reef-app-fields { display: grid; grid-template-columns: max-content minmax(0, 18rem); gap: 0.4rem 0.8rem; align-items: center; }
.reef-app-fields input, .reef-app-account-acts input { font: inherit; padding: 0.3rem 0.4rem; color: var(--fg); background: var(--bg); border: 1px solid var(--line); border-radius: 6px; }
.reef-app-account-table { border-collapse: collapse; width: 100%; font-size: 0.85rem; }
.reef-app-account-table th, .reef-app-account-table td { text-align: left; padding: 0.35rem 0.5rem; border-bottom: 1px solid var(--line); vertical-align: middle; white-space: nowrap; }
.reef-app-account-table td.reef-app-account-label { white-space: normal; overflow-wrap: anywhere; }
.reef-app-account-acts { display: flex; flex-wrap: wrap; gap: 0.4rem; align-items: center; }
.reef-app-account-said { margin: 0.4rem 0 0; font-size: 0.85rem; }
@media (max-width: 40rem) { .reef-app-fields { grid-template-columns: minmax(0, 1fr); } }
.reef-app-projects { list-style: none; padding: 0; margin: 0; display: grid; gap: 1rem; }
.reef-app-project { border: 1px solid var(--line); border-radius: 8px; padding: 0.9rem 1.1rem; background: var(--panel); }
.reef-app-project h2 { margin: 0; font-size: 1.15rem; }
.reef-app-repo { margin: 0.2rem 0 0.6rem; color: var(--muted); font-family: monospace; font-size: 0.85rem; overflow-wrap: anywhere; }
.reef-app-project-head { display: flex; flex-wrap: wrap; gap: 0.6rem; align-items: baseline; }
.reef-app-plan-count { color: var(--muted); font-size: 0.85rem; }
.reef-app-plans { list-style: none; margin: 0; padding: 0; display: grid; gap: 0.4rem; }
.reef-app-plan-row { display: grid; gap: 0.15rem; padding: 0.4rem 0.6rem; border: 1px solid var(--line); border-left: 3px solid var(--line); border-radius: 6px; background: var(--bg); }
.reef-app-plan-row.active { border-left-color: var(--reef-progress); }
.reef-app-plan-head { display: flex; flex-wrap: wrap; gap: 0.5rem; align-items: baseline; }
.reef-app-plan-head .reef-app-branch { margin-left: 0; }
.reef-app-plan-state { font-size: 0.72rem; color: var(--muted); border: 1px solid var(--line); border-radius: 4px; padding: 0 0.3rem; }
.reef-app-plan-state.s-testing { color: var(--reef-progress); border-color: var(--reef-progress); }
.reef-app-plan-state.s-sessions { color: var(--links); border-color: var(--links); }
.reef-app-listed { margin-left: auto; font-size: 0.8rem; color: var(--muted); }
.reef-app-plan-facts, .reef-app-plan-work { display: flex; flex-wrap: wrap; gap: 0.2rem 0.8rem; align-items: baseline; font-size: 0.8rem; color: var(--muted); }
.reef-app-plan-work { color: var(--fg); }
.reef-app-done-today.some { color: var(--reef-done); }
.reef-app-work-kind { font-size: 0.68rem; color: var(--muted); text-transform: uppercase; letter-spacing: 0.04em; }
.reef-app-branch { margin-left: 0.5rem; color: var(--muted); font-family: monospace; font-size: 0.8rem; }
.reef-app-plan-switch { display: flex; flex-wrap: wrap; gap: 0.3rem; align-items: baseline; }
.reef-app-plan-tab { font-size: 0.85rem; padding: 0.1rem 0.5rem; border: 1px solid var(--line); border-radius: 999px; text-decoration: none; color: var(--fg); background: var(--panel); }
.reef-app-plan-tab.on { border-color: var(--links); }
.reef-app-plan-tab .reef-app-branch { margin-left: 0.35rem; }
.reef-app-branch-pick { display: flex; gap: 0.4rem; align-items: baseline; font-size: 0.85rem; color: var(--muted); }
.reef-app-branch-pick select { font: inherit; color: var(--fg); background: var(--bg); border: 1px solid var(--line); border-radius: 6px; padding: 0.1rem 0.3rem; }
.reef-app-branch-said { max-width: 32rem; overflow-wrap: anywhere; }
.reef-app-muted { color: var(--muted); }
.reef-app-error { color: #c0392b; }
.reef-app-plan { display: flex; flex-direction: column; height: 100vh; }
.reef-app-bar { display: flex; gap: 1rem; align-items: baseline; padding: 0.5rem 1rem; border-bottom: 1px solid var(--line); }
.reef-app-stage { flex: 1; min-height: 0; display: flex; position: relative; }
.reef-app-stage > .reef.reef-page { margin: 0; border: 0; border-radius: 0; min-width: 0; }
.reef-app-over { position: absolute; top: 3rem; left: 0; right: 0; text-align: center; }
.reef-app-stage > .reef.reef-page { flex: 1; }
.reef-app-tabs { display: flex; flex-wrap: wrap; gap: 0.2rem; margin: 0.6rem 0 0.2rem; border-bottom: 1px solid var(--line); }
.reef-app-tab { font: inherit; font-size: 0.85rem; color: var(--muted); background: none; border: 0; border-bottom: 2px solid transparent; padding: 0.25rem 0.6rem; margin-bottom: -1px; cursor: pointer; white-space: nowrap; }
.reef-app-tab:hover { color: var(--fg); }
.reef-app-tab.on { color: var(--fg); border-bottom-color: var(--links); }
.reef-app-editor { margin-top: 0.6rem; }
.reef-app-html { display: contents; }
.reef-app-yaml { width: 100%; box-sizing: border-box; min-height: 24rem; resize: vertical; font-family: monospace; font-size: 0.85rem; color: var(--fg); background: var(--bg); border: 1px solid var(--line); border-radius: 6px; padding: 0.5rem; }
.reef-app-actions { display: flex; gap: 0.5rem; margin: 0.5rem 0; }
.reef-app-problems { margin: 0.3rem 0 0.6rem; padding-left: 1.2rem; color: #c0392b; font-size: 0.9rem; }
.reef-app-newer { white-space: pre-wrap; font-size: 0.8rem; background: var(--bg); border: 1px solid var(--line); border-radius: 6px; padding: 0.5rem; }
.reef-app-queue { flex: none; width: 15rem; overflow-y: auto; box-sizing: border-box; padding: 0.6rem 0.7rem; border-right: 1px solid var(--line); background: var(--panel); font-size: 0.85rem; }
.reef-app-queue-head { margin: 0 0 0.5rem; font-size: 0.95rem; }
.reef-app-queue-list { list-style: none; margin: 0; padding: 0; display: grid; gap: 0.3rem; }
.reef-app-queue-list.dragging { user-select: none; cursor: grabbing; }
.reef-app-queue-entry { display: flex; gap: 0.35rem; align-items: flex-start; padding: 0.35rem 0.3rem; border: 1px solid var(--line); border-radius: 6px; background: var(--bg); }
.reef-app-queue-entry:focus { outline: 2px solid var(--links); outline-offset: 1px; }
.reef-app-queue-entry.dragged { opacity: 0.45; }
.reef-app-queue-entry.drop-before { box-shadow: 0 -3px 0 var(--links); }
.reef-app-queue-entry.drop-after { box-shadow: 0 3px 0 var(--links); }
.reef-app-queue-grip { cursor: grab; touch-action: none; color: var(--muted); padding: 0 0.1rem; }
.reef-app-queue-place { color: var(--muted); min-width: 1.4rem; text-align: right; }
.reef-app-queue-body { flex: 1; min-width: 0; }
.reef-app-queue-top { display: flex; gap: 0.4rem; align-items: baseline; flex-wrap: wrap; }
.reef-app-queue-id { overflow-wrap: anywhere; }
.reef-app-queue-state { font-size: 0.75rem; color: var(--muted); }
.reef-app-queue-state.s-ready { color: var(--reef-ready); }
.reef-app-queue-state.s-done { color: var(--reef-done); }
.reef-app-queue-state.s-blocked, .reef-app-queue-state.s-missing { color: var(--reef-blocked); }
.reef-app-queue-state.s-in-progress { color: var(--reef-progress); }
.reef-app-queue-title { margin-top: 0.1rem; overflow-wrap: anywhere; }
.reef-app-queue-needs { margin-top: 0.1rem; color: var(--muted); font-size: 0.78rem; }
.reef-app-queue-placed { margin-top: 0.1rem; color: var(--muted); font-size: 0.72rem; font-style: italic; }
.reef-app-queue-off { flex: none; font: inherit; line-height: 1; color: var(--muted); background: none; border: 0; cursor: pointer; padding: 0 0.15rem; }
.reef-app-queue-off:hover { color: var(--fg); }
.reef-app-queue-problem { margin: 0 0 0.5rem; }
.reef-app-changes { margin-top: 0.6rem; font-size: 0.85rem; }
.reef-app-revision-bar { display: flex; gap: 0.5rem; align-items: center; }
.reef-app-revision-pick { flex: 1; min-width: 0; display: flex; gap: 0.4rem; align-items: center; }
.reef-app-mode { flex: none; display: inline-flex; border: 1px solid var(--line); border-radius: 6px; overflow: hidden; }
.reef-app-mode-pick { font: inherit; font-size: 0.75rem; color: var(--muted); background: var(--bg); border: 0; padding: 0.2rem 0.55rem; cursor: pointer; }
.reef-app-mode-pick + .reef-app-mode-pick { border-left: 1px solid var(--line); }
.reef-app-mode-pick[aria-pressed="true"] { color: var(--fg); background: var(--panel); font-weight: 600; }
.reef-app-revision { flex: 1; min-width: 0; font: inherit; color: var(--fg); background: var(--bg); border: 1px solid var(--line); border-radius: 6px; padding: 0.2rem 0.3rem; }
.reef-app-revision-sum { margin: 0.4rem 0; }
.reef-app-plus { color: var(--add-fg); }
.reef-app-minus { color: var(--del-fg); }
.reef-app-conflicts { margin: 0.4rem 0; color: var(--reef-blocked); }
.reef-app-files { list-style: none; margin: 0; padding: 0; display: grid; gap: 0.3rem; }
.reef-app-file { border: 1px solid var(--line); border-radius: 6px; background: var(--bg); min-width: 0; }
.reef-app-file-head { display: flex; gap: 0.45rem; align-items: baseline; width: 100%; box-sizing: border-box; font: inherit; color: inherit; text-align: left; background: var(--panel); border: 0; border-radius: 6px; padding: 0.3rem 0.45rem; cursor: pointer; }
.reef-app-file.open > .reef-app-file-head { border-bottom: 1px solid var(--line); border-radius: 6px 6px 0 0; }
.reef-app-file-head:hover .reef-app-file-path { text-decoration: underline; }
.reef-app-status { flex: none; font-family: monospace; font-weight: 600; width: 1ch; }
.reef-app-status.s-added { color: var(--add-fg); }
.reef-app-status.s-deleted { color: var(--del-fg); }
.reef-app-status.s-modified { color: var(--reef-progress); }
.reef-app-status.s-renamed { color: var(--links); }
.reef-app-file-path { flex: 1; min-width: 0; font-family: monospace; overflow-wrap: anywhere; }
.reef-app-file-counts { flex: none; font-family: monospace; font-size: 0.8rem; }
.reef-app-tag { flex: none; font-size: 0.72rem; color: var(--muted); border: 1px solid var(--line); border-radius: 4px; padding: 0 0.3rem; }
.reef-app-tag.conflicted { color: var(--reef-blocked); border-color: var(--reef-blocked); }
.reef-app-tag.draft { color: var(--reef-progress); border-color: var(--reef-progress); }
.reef-app-tag.new { color: var(--add-fg); border-color: var(--add-fg); }
.reef-app-tag.changed { color: var(--reef-progress); border-color: var(--reef-progress); }
.reef-app-tag.removed { color: var(--del-fg); border-color: var(--del-fg); }
.reef-app-ofile-head { display: flex; gap: 0.45rem; align-items: baseline; background: var(--panel); border-bottom: 1px solid var(--line); border-radius: 6px 6px 0 0; padding: 0.3rem 0.45rem; }
.reef-app-groups { list-style: none; margin: 0; padding: 0; }
.reef-app-group + .reef-app-group { border-top: 1px solid var(--line); }
.reef-app-group-head { display: flex; gap: 0.45rem; align-items: baseline; width: 100%; box-sizing: border-box; font: inherit; color: inherit; text-align: left; background: none; border: 0; padding: 0.25rem 0.45rem 0.25rem 1.1rem; cursor: pointer; }
.reef-app-group-head:hover .reef-app-group-name { text-decoration: underline; }
.reef-app-group.open > .reef-app-group-head { background: var(--panel); }
.reef-app-kind { flex: none; font-family: monospace; font-size: 0.78rem; color: var(--muted); }
.reef-app-group-name { flex: 1; min-width: 0; font-family: monospace; font-weight: 600; overflow-wrap: anywhere; }
.reef-app-group-body { border-top: 1px solid var(--line); }
.reef-app-member + .reef-app-member { border-top: 1px dashed var(--line); }
.reef-app-member-head { display: flex; gap: 0.45rem; align-items: baseline; padding: 0.2rem 0.45rem 0.2rem 1.6rem; font-size: 0.8rem; }
.reef-app-member-name { flex: 1; min-width: 0; font-family: monospace; overflow-wrap: anywhere; }
.reef-app-via { flex: none; font-family: monospace; font-size: 0.75rem; }
.reef-app-diff { overflow-x: auto; }
.reef-app-diff-note { margin: 0.4rem 0.5rem; }
.reef-app-diff-wait { margin: 0.4rem 0.5rem; }
.reef-app-hunks { border-collapse: collapse; width: 100%; font-family: monospace; font-size: 0.78rem; line-height: 1.45; }
.reef-app-hunk-head td { background: var(--hunk-bg); color: var(--muted); padding: 0.1rem 0.5rem; white-space: pre; }
.reef-app-ln { width: 1%; min-width: 2.5ch; padding: 0 0.4rem; text-align: right; color: var(--muted); user-select: none; vertical-align: top; }
.reef-app-code { padding: 0 0.5rem 0 0.2rem; white-space: pre; }
.reef-app-sign { display: inline-block; width: 1.4ch; user-select: none; }
.reef-app-line.added { background: var(--add-bg); }
.reef-app-line.removed { background: var(--del-bg); }
.reef-app-line.added .reef-app-sign { color: var(--add-fg); }
.reef-app-line.removed .reef-app-sign { color: var(--del-fg); }
.reef-app-eof td { color: var(--muted); font-style: italic; padding: 0 0.5rem; }
.reef-app-diff.commenting .reef-app-ln { cursor: pointer; position: relative; }
.reef-app-diff.commenting .reef-app-line:hover .reef-app-ln:first-child::before { content: "+"; position: absolute; left: 0.1rem; color: #fff; background: var(--links); border-radius: 3px; width: 1.1em; text-align: center; font-weight: 600; }
.reef-app-line.focus td { box-shadow: inset 0 1px 0 var(--links), inset 0 -1px 0 var(--links); }
.reef-app-thread-row > td, .reef-app-compose-row > td { padding: 0.35rem 0.5rem; background: var(--panel); border-top: 1px solid var(--line); border-bottom: 1px solid var(--line); font-family: system-ui, sans-serif; font-size: 0.85rem; white-space: normal; }
.reef-app-count { display: inline-block; min-width: 1.1em; margin-left: 0.3rem; padding: 0 0.3rem; font-size: 0.72rem; text-align: center; border-radius: 9px; color: var(--muted); border: 1px solid var(--line); }
.reef-app-count.some, .some > .reef-app-count { color: #fff; background: var(--links); border-color: var(--links); }
.reef-app-comments { margin-top: 0.6rem; font-size: 0.85rem; display: grid; gap: 0.5rem; }
.reef-app-comments-sum { margin: 0; }
.reef-app-comments-head { margin: 0.4rem 0 0; font-size: 0.9rem; }
.reef-app-thread { border: 1px solid var(--line); border-left: 3px solid var(--links); border-radius: 6px; padding: 0.4rem 0.55rem; background: var(--bg); }
.reef-app-thread.resolved { border-left-color: var(--reef-done); opacity: 0.8; }
.reef-app-thread.focus { box-shadow: 0 0 0 2px var(--links); }
.reef-app-thread.outdated { border-left-style: dashed; }
.reef-app-thread.pending { border-left-style: dotted; }
.reef-app-tag.reef-app-pending { color: var(--reef-progress); border-color: var(--reef-progress); }
.reef-app-review-bar { display: flex; flex-wrap: wrap; align-items: center; gap: 0.5rem; margin: 0 0 0.6rem; padding: 0.4rem 0.5rem; border: 1px solid var(--reef-progress); border-radius: 6px; }
.reef-app-review-sum { flex: 1; font-size: 0.85rem; }
.reef-app-comment + .reef-app-comment { margin-top: 0.4rem; padding-top: 0.4rem; border-top: 1px dashed var(--line); }
.reef-app-comment-head { color: var(--muted); font-size: 0.75rem; }
.reef-app-comment-number { font-family: monospace; }
.reef-app-author { color: var(--fg); font-weight: 600; }
.reef-app-comment-body { overflow-wrap: anywhere; }
.reef-app-comment-body > :first-child { margin-top: 0.15rem; }
.reef-app-comment-body > :last-child { margin-bottom: 0.15rem; }
.reef-app-comment-body pre { overflow-x: auto; background: var(--panel); padding: 0.3rem 0.4rem; border-radius: 4px; }
.reef-app-thread-foot { display: flex; gap: 0.4rem; align-items: center; margin-top: 0.35rem; }
.reef-app-thread-state { font-size: 0.75rem; margin-right: auto; }
.reef-app-thread-state.open { color: var(--links); }
.reef-app-thread-state.resolved { color: var(--reef-done); }
.reef-app-thread-resolution { margin: 0.25rem 0 0; font-size: 0.75rem; color: var(--muted); overflow-wrap: anywhere; }
.reef-app-thread-marked { margin: 0.15rem 0 0; font-size: 0.75rem; color: var(--muted); }
.reef-app-history { list-style: none; margin: 0; padding: 0; }
.reef-app-history-entry { padding: 0.35rem 0; border-bottom: 1px dashed var(--line); }
.reef-app-history-entry:last-child { border-bottom: none; }
.reef-app-history-head { font-size: 0.75rem; color: var(--muted); }
.reef-app-history-version { font-family: monospace; }
.reef-app-history-what { overflow-wrap: anywhere; }
.reef-app-history-more { margin: 0.5rem 0 0; }
.reef-app-timeline { list-style: none; margin: 0; padding: 0; }
.reef-app-timeline-entry { padding: 0.35rem 0 0.35rem 0.55rem; border-bottom: 1px dashed var(--line); border-left: 3px solid var(--line); }
.reef-app-timeline-entry:last-child { border-bottom: none; }
.reef-app-timeline-entry.start { border-left-color: var(--reef-progress); }
.reef-app-timeline-entry.capture { border-left-color: var(--links); }
.reef-app-timeline-entry.comment, .reef-app-timeline-entry.mark, .reef-app-timeline-entry.resolution { border-left-color: var(--reef-done); }
.reef-app-timeline-head { font-size: 0.75rem; color: var(--muted); }
.reef-app-timeline-kind { font-size: 0.65rem; }
.reef-app-timeline-what { overflow-wrap: anywhere; }
button.reef-app-timeline-open { display: block; width: 100%; padding: 0; border: none; background: none; color: var(--links); font: inherit; text-align: left; cursor: pointer; }
button.reef-app-timeline-open:hover { text-decoration: underline; }
.reef-app-composer { margin-top: 0.35rem; }
.reef-app-compose { width: 100%; box-sizing: border-box; resize: vertical; font: inherit; font-size: 0.85rem; color: var(--fg); background: var(--bg); border: 1px solid var(--line); border-radius: 6px; padding: 0.35rem 0.45rem; }
.reef-app-composer .reef-app-actions { margin: 0.3rem 0 0; }
.reef-app-line-thread { display: grid; gap: 0.25rem; }
.reef-app-line-thread-head { display: flex; gap: 0.5rem; align-items: center; justify-content: space-between; }
.reef-app-line-thread-head code { overflow-wrap: anywhere; }
.reef-app-open-box { position: relative; }
.reef-app-open-comments .reef-app-count { margin-left: 0; }
.reef-results.reef-app-open-list { left: auto; right: 0; width: min(380px, 80vw); }
.reef-app-open-where { font-size: 0.8rem; color: var(--muted); }
.reef-app-open-text { overflow-wrap: anywhere; }
.reef-app-open-none { padding: 4px 10px; }
.reef-app-menu-backdrop { position: fixed; inset: 0; z-index: 19; }
.reef-app-menu { position: absolute; z-index: 20; width: 248px; max-width: calc(100% - 8px); box-sizing: border-box; padding: 4px 0; font-size: 0.85rem; color: var(--fg); background: var(--theme-popup-bg, var(--bg)); border: 1px solid var(--theme-popup-border, var(--reef-border)); border-radius: 6px; box-shadow: 0 6px 20px rgba(0, 0, 0, 0.25); }
.reef-app-menu-head { padding: 4px 10px 6px; color: var(--muted); font-size: 0.8rem; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; border-bottom: 1px solid var(--line); }
.reef-app-menu-items { display: flex; flex-direction: column; padding: 2px 0; }
.reef-app-menu-item { display: block; width: 100%; padding: 4px 10px; border: none; background: none; color: var(--fg); font: inherit; text-align: left; cursor: pointer; }
.reef-app-menu-item:hover, .reef-app-menu-item:focus-visible { background: var(--theme-hover, var(--reef-frame)); outline: none; }
.reef-app-menu-label { padding: 4px 10px 2px; font-size: 0.7rem; color: var(--muted); text-transform: uppercase; letter-spacing: 0.04em; border-top: 1px solid var(--line); }
.reef-app-menu-items + .reef-app-menu-label, .reef-app-menu-head + .reef-app-menu-label { border-top: none; }
.reef-app-menu-chips { display: flex; flex-wrap: wrap; gap: 4px; padding: 2px 10px 6px; }
.reef-app-menu-chips .reef-button.on { border-color: var(--reef-ready); font-weight: 600; }
.reef-app-menu-chips .reef-button:disabled { cursor: default; }
.reef-app-menu-chips + .reef-app-menu-items { border-top: 1px solid var(--line); }
.reef-app-menu-problem { margin: 4px 10px; font-size: 0.8rem; overflow-wrap: anywhere; }
.reef-app-sessions, .reef-app-sessions-toggle, .reef-app-user, .reef-app-account, .reef-app-picker { --reef-done: #2e7d32; --reef-progress: #e0a100; --reef-blocked: #d32f2f; --reef-ready: var(--links); --reef-border: color-mix(in srgb, var(--fg) 40%, var(--bg)); }
.reef-app-sessions-toggle { margin-left: auto; }
.reef-app-waiting { margin-left: 0.35rem; font-size: 0.75rem; color: var(--links); }
.reef-app-sessions-toggle.some .reef-app-waiting { font-weight: 600; }
.reef-app-sessions { flex: none; width: 30rem; max-width: 45vw; overflow-y: auto; box-sizing: border-box; padding: 0.6rem 0.8rem; border-left: 1px solid var(--line); background: var(--panel); font-size: 0.85rem; display: flex; flex-direction: column; gap: 0.5rem; }
.reef-app-sessions-head { display: flex; align-items: baseline; justify-content: space-between; }
.reef-app-sessions-head h2, .reef-app-sessions-sub { margin: 0; font-size: 0.95rem; }
.reef-app-sessions-sub { font-size: 0.85rem; color: var(--muted); }
.reef-app-runners { list-style: none; margin: 0; padding: 0; display: grid; gap: 0.3rem; }
.reef-app-runner { border: 1px solid var(--line); border-radius: 6px; background: var(--bg); padding: 0.35rem 0.5rem; cursor: pointer; }
.reef-app-runner.terminal { cursor: default; }
.reef-app-runner.picked { border-color: var(--links); box-shadow: 0 0 0 1px var(--links); }
.reef-app-runner-top { display: flex; gap: 0.45rem; align-items: baseline; flex-wrap: wrap; }
.reef-app-runner-where { color: var(--muted); font-family: monospace; font-size: 0.78rem; overflow-wrap: anywhere; }
.reef-app-runner-loop { margin-top: 0.15rem; }
.reef-app-runner .reef-app-actions { margin: 0.3rem 0 0; }
.reef-app-session-state { font-size: 0.75rem; color: var(--muted); }
.reef-app-session-state.s-working { color: var(--reef-progress); }
.reef-app-session-state.s-waiting { color: var(--links); }
.reef-app-session-state.s-gone, .reef-app-session-state.s-detached { color: var(--reef-blocked); }
.reef-app-session { display: grid; gap: 0.45rem; }
.reef-app-session-sum { margin: 0; color: var(--muted); }
.reef-app-transcript { list-style: none; margin: 0; padding: 0.4rem; max-height: 55vh; overflow-y: auto; border: 1px solid var(--line); border-radius: 6px; background: var(--bg); display: grid; gap: 0.3rem; font-size: 0.8rem; }
.reef-app-entry { overflow-wrap: anywhere; white-space: pre-wrap; }
.reef-app-entry.user { color: var(--links); }
.reef-app-entry.tool, .reef-app-entry.turn, .reef-app-entry.session { color: var(--muted); font-size: 0.75rem; white-space: normal; }
.reef-app-entry.output summary { color: var(--muted); cursor: pointer; font-size: 0.75rem; }
.reef-app-entry.output pre { margin: 0.2rem 0; max-height: 12rem; overflow: auto; background: var(--panel); padding: 0.3rem; border-radius: 4px; white-space: pre-wrap; }
.reef-app-entry.error summary { color: var(--reef-blocked); }
.reef-app-entry.note { color: var(--muted); font-style: italic; }
.reef-app-entry.asked, .reef-app-entry.answered { border-left: 3px solid var(--links); padding-left: 0.4rem; }
.reef-app-prompt { border: 1px solid var(--links); border-radius: 6px; padding: 0.5rem 0.6rem; background: var(--bg); display: grid; gap: 0.35rem; }
.reef-app-prompt-head { margin: 0; font-weight: 600; }
.reef-app-prompt-input { margin: 0; max-height: 14rem; overflow: auto; font-size: 0.75rem; background: var(--panel); padding: 0.3rem; border-radius: 4px; }
.reef-app-prompt .reef-app-actions { margin: 0; }
.reef-app-ask { margin: 0; padding: 0.35rem 0.5rem; border: 1px solid var(--line); border-radius: 6px; display: grid; gap: 0.25rem; }
.reef-app-ask legend { padding: 0 0.2rem; font-weight: 600; }
.reef-app-choice { display: flex; gap: 0.4rem; align-items: baseline; cursor: pointer; }
.reef-app-choice-text { overflow-wrap: anywhere; }
.reef-app-own { font: inherit; font-size: 0.85rem; color: var(--fg); background: var(--bg); border: 1px solid var(--line); border-radius: 4px; padding: 0.2rem 0.35rem; }
.reef-app-entry-question { margin-left: 0.6rem; }
.reef-app-plan-body { max-height: 26rem; overflow-y: auto; padding: 0.2rem 0.5rem; background: var(--panel); border-radius: 4px; white-space: normal; }
.reef-app-plan-body h1 { font-size: 1.05rem; }
.reef-app-plan-body h2 { font-size: 0.95rem; }
.reef-app-entry.asked summary { cursor: pointer; }
.reef-app-ended > summary { cursor: pointer; color: var(--muted); }
.reef-app-unchanged { margin: 0; color: var(--reef-progress); font-size: 0.8rem; }
.reef-app-decisions { margin-top: 0.6rem; font-size: 0.85rem; display: grid; gap: 0.5rem; }
.reef-app-decision { border: 1px solid var(--line); border-left: 3px solid var(--line); border-radius: 6px; padding: 0.4rem 0.55rem; background: var(--bg); display: grid; gap: 0.3rem; }
.reef-app-decision.approved, .reef-app-decision.answered { border-left-color: var(--reef-done); }
.reef-app-decision.rejected { border-left-color: var(--reef-blocked); }
.reef-app-decision.waiting { border-left-color: var(--reef-progress); }
.reef-app-decision-head { display: flex; gap: 0.45rem; align-items: baseline; flex-wrap: wrap; }
.reef-app-decision-number { font-family: monospace; color: var(--muted); }
.reef-app-decision-when { font-size: 0.75rem; margin-left: auto; }
.reef-app-verdict.approved, .reef-app-verdict.answered { color: var(--reef-done); border-color: var(--reef-done); }
.reef-app-verdict.rejected { color: var(--reef-blocked); border-color: var(--reef-blocked); }
.reef-app-verdict.waiting { color: var(--reef-progress); border-color: var(--reef-progress); }
.reef-app-feedback { margin: 0; padding: 0.2rem 0.5rem; border-left: 3px solid var(--reef-blocked); background: var(--panel); white-space: pre-wrap; }
.reef-app-decision-plan > summary { cursor: pointer; color: var(--muted); }
.reef-app-decision-asked { margin: 0; }
.reef-app-decision-choices { margin: 0.15rem 0 0; padding-left: 1.1rem; }
.reef-app-decision-choices li.picked { color: var(--reef-done); }
.reef-app-own-answer { margin: 0.15rem 0 0; font-style: italic; }
.reef-app-ended > .reef-app-runners { margin-top: 0.3rem; opacity: 0.75; }
"#;

/// The app's pages, by path
#[derive(Debug, Clone, PartialEq, Routable)]
#[rustfmt::skip]
pub enum Route {
    /// Every project and its plans, each summed up
    #[route("/")]
    Explorer {},
    /// One plan, full-window
    #[route("/plans/:project/:plan")]
    PlanPage { project: String, plan: String },
    /// Your password and tokens, and every user to an admin
    #[route("/account")]
    Account {},
}

/// The app: its stylesheets, then whichever page the path names
#[component]
pub fn App() -> Element {
    rsx! {
        Head {}
        Router::<Route> {}
    }
}

/// The page's stylesheets: reef.css as mdbook-reef installs it, and the theme it expects
///
/// It takes no props, so it never renders again: `document::Style` warns when its props change,
/// and a plain `style` element gets a hydration marker in its text that breaks its first rule.
#[component]
fn Head() -> Element {
    rsx! {
        document::Stylesheet { href: "/reef.css" }
        document::Style { {THEME} }
    }
}

/// What a plan follows, as the front page and the switcher show it: its branch and the commit
/// last fetched of it, or its project's working tree
///
/// # Arguments
///
/// * `project` - The plan's project
/// * `info` - The plan
pub(crate) fn follows(project: &Project, info: &PlanInfo) -> String {
    let branch = info.branch.as_deref();
    match (&project.remote, branch) {
        // a remote project's branch, where it was last fetched
        (Some(remote), branch) => {
            let name = branch.unwrap_or(&remote.branch);
            match remote.commit_of(branch) {
                Some(commit) => format!("{name} @ {}", reef_api::project::short(commit)),
                None => format!("{name}, not fetched yet"),
            }
        }
        // a local project's branch, or its working tree
        (None, Some(branch)) => branch.to_string(),
        (None, None) => "working tree".to_string(),
    }
}

/// The plan page's switcher: each plan of the project, with the branch it follows, the one shown
/// marked and each other a link to its page
///
/// # Arguments
///
/// * `listing` - The project and its plans
/// * `plan` - The plan shown
#[component]
fn PlanSwitch(listing: ProjectPlans, plan: String) -> Element {
    // each plan's name, its route, what it follows, and whether it is the one shown
    let tabs: Vec<(String, Route, String, bool)> = listing
        .plans
        .iter()
        .map(|info| {
            let route = Route::PlanPage {
                project: info.name.project.clone(),
                plan: info.name.plan.clone(),
            };
            let follows = follows(&listing.project, info);
            (
                info.name.plan.clone(),
                route,
                follows,
                info.name.plan == plan,
            )
        })
        .collect();
    rsx! {
        nav { class: "reef-app-plan-switch", "aria-label": "The plans of {listing.project.name}",
            for (name, route, follows, on) in tabs {
                if on {
                    span { key: "{name}", class: "reef-app-plan-tab on", "aria-current": "page",
                        "{name}"
                        span { class: "reef-app-branch", "{follows}" }
                    }
                } else {
                    Link { key: "{name}", class: "reef-app-plan-tab", to: route,
                        "{name}"
                        span { class: "reef-app-branch", "{follows}" }
                    }
                }
            }
        }
    }
}

/// The plan page's branch picker: the branches of the project's repo, the one the plan follows
/// chosen, and picking another moves the plan there, as `reef plan branch` does, saying why if
/// it can't
///
/// # Arguments
///
/// * `listing` - The project and its plans
/// * `plan` - The plan shown
/// * `siblings` - The project's plans as the bar read them, read again once the plan moves
#[component]
fn BranchPicker(
    listing: ProjectPlans,
    plan: String,
    siblings: Resource<Result<ProjectPlans, ServerFnError>>,
) -> Element {
    // the repo's branches, read once the page is up, and what came of the last pick
    let project = listing.project.name.clone();
    let named = project.clone();
    let branches = use_resource(move || calls::branches(named.clone()));
    let mut said = use_signal(|| None::<String>);
    // the branch the plan follows, empty for its project's own, and what that own one is
    let current = listing
        .plans
        .iter()
        .find(|info| info.name.plan == plan)
        .and_then(|info| info.branch.clone())
        .unwrap_or_default();
    let own = match &listing.project.remote {
        Some(remote) => format!("its project's own, {}", remote.branch),
        None => "its project's working tree".to_string(),
    };
    // each branch to pick, the one followed among them even if the repo has it no longer
    let mut options: Vec<String> = match &*branches.read() {
        Some(Ok(listed)) => listed.clone(),
        _ => Vec::new(),
    };
    if !current.is_empty() && !options.contains(&current) {
        options.push(current.clone());
    }
    let shown = plan.clone();
    let on_pick = move |event: FormEvent| {
        // the branch picked, or none for the project's own, sent as a change of the plan's
        let picked = event.value();
        let branch = (!picked.is_empty()).then_some(picked);
        let (project, plan) = (project.clone(), plan.clone());
        let mut siblings = siblings;
        spawn(async move {
            let outcome = calls::set_plan_branch(project, plan, branch).await;
            said.set(match outcome {
                Ok(Branched::Applied { .. }) => None,
                Ok(Branched::NoBranch { branch, reason }) => {
                    Some(format!("It can't follow `{branch}`: {reason}"))
                }
                Ok(Branched::Refused(problems)) => {
                    let named: Vec<String> = problems.iter().map(ToString::to_string).collect();
                    Some(format!(
                        "It follows what it did, since its docs there would add: {}",
                        named.join("; ")
                    ))
                }
                Err(error) => Some(format!("Moving the plan failed: {error}")),
            });
            // the bar reads what it follows again, whatever came of it
            siblings.restart();
        });
    };
    rsx! {
        label { class: "reef-app-branch-pick",
            "follows"
            select {
                "aria-label": "The branch {shown} follows",
                onchange: on_pick,
                option { value: "", selected: current.is_empty(), "{own}" }
                for branch in options {
                    option {
                        key: "{branch}",
                        value: "{branch}",
                        selected: branch == current,
                        "{branch}"
                    }
                }
            }
            if let Some(said) = said() {
                span { class: "reef-app-error reef-app-branch-said", "{said}" }
            }
        }
    }
}

/// A plan's page: a bar naming the plan, or saying why it can't be read, over its project's
/// queue and the plan's canvas
///
/// Its props are signals, so a route to another plan reads that plan again, and the canvas is
/// keyed by the plan, so another plan starts it afresh. The queue is the project's, so it sits
/// outside that key, and the canvas's sidebar reaches it through context.
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
#[component]
pub fn PlanPage(project: ReadSignal<String>, plan: ReadSignal<String>) -> Element {
    // the plan, read again when the route names another plan, so a plan that isn't there says so
    let info = use_server_future(move || calls::plan_info(project(), plan()))?;
    // the plan's changes, which read its payload again
    let changes = use_changes(project, plan);
    // its payload, read in the client alone, since only the client draws it, named for its plan,
    // and read again after each change
    let payload = use_resource(move || async move {
        let _ = changes();
        let (project, plan) = (project(), plan());
        let name = format!("{project}/{plan}");
        calls::payload(project, plan)
            .await
            .map(|payload| -> Keyed { (name, payload) })
    });
    // the project and its plans, each with the branch it follows, for the bar's switcher
    let siblings = use_resource(move || calls::project_plans(project()));
    // the plan's queue, which the panel shows and the sidebar edits
    use_queue(project, plan);
    // and its runners, which the bar counts and the sessions panel shows
    use_runners(project);
    let (project_now, plan_now) = (project(), plan());
    rsx! {
        div { class: "reef-app-plan",
            header { class: "reef-app-bar",
                Link { to: Route::Explorer {}, "← Plans" }
                strong { class: "reef-app-plan-name", "{project}/{plan}" }
                if let Some(Ok(listing)) = &*siblings.read() {
                    PlanSwitch { listing: listing.clone(), plan: plan_now.clone() }
                    BranchPicker { listing: listing.clone(), plan: plan_now.clone(), siblings }
                }
                if let Some(Err(error)) = &*info.read() {
                    CallFailed { error: error.clone(), doing: "Reading the plan" }
                }
                SessionsButton {}
                SignedInAs {}
            }
            div { class: "reef-app-stage",
                QueuePanel { project, plan }
                // a key holds only on a block's first node, so the canvas is one of its own
                {
                    rsx! {
                        Canvas {
                            key: "{project_now}/{plan_now}",
                            project: project_now.clone(),
                            plan: plan_now.clone(),
                            payload,
                            changes,
                        }
                    }
                }
                SessionPanel {}
            }
        }
    }
}
