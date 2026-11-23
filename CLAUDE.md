# reef

## This repo's plan lives in central reef

reef's own plan is `reef/central`, held by central reef: a `reef-serve` running as the systemd
user unit `reef` (`central/README.md`, "Running it for real"). The reef skill
(`.claude/skills/reef`) says how to keep it in step with the work. What only this repo adds:

- **`plan/` is the plan's export.** It is generated, so never edit it by hand: the next export
  writes over it. After changing the plan, write the book with `reef export plan --replace`, and
  commit `plan/` in the same change as the work. `cargo run -q --release -p mdbook-reef -- check
  plan` still checks it.
- **The skill is written, not edited.** `reef skill` writes `crates/reef/assets/skill` to
  `.claude/skills/reef`, so change the assets and run it again. The mdbook skill's source is
  `crates/mdbook-reef/assets/skill`. `mdbook-reef install` and `mdbook-reef skill` would write
  it over the central one, so pass `install` `--no-skill`, and don't run `skill` here.
- **Run `reef` from the repo's root.** If it says nothing answers, start the server with
  `systemctl --user start reef`. A new binary is a reinstall and a restart
  (`central/README.md`).
