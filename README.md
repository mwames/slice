# slice

Housekeeping for one slice of work on a ticket: the Harvest timer, the ticket's
status, and the ticket's branch and worktree. Each command reads everything it
needs before changing anything, and undoes its own steps if one fails.

```
slice status [--json]
slice open <TICKET> [--type TYPE] [--repo PATH] [--base REF] [--json]
slice pause [--json]
slice resume [--json]
slice close [--done] [--json]
```

- Harvest has one entry per ticket per day; every slice of the ticket that day
  logs to it.
- Git has one branch per ticket, `<type>/<yyyymmdd>-<REF>-<short-title>`, in a
  worktree under `<repo>/../<repo>.slices/`.
- Tickets are ClickUp or monday, routed by ref prefix. `open` sets the ticket
  in progress; `close` sets it to review, or done with `--done`.
- The slice id (`<REF>-<n>`) is internal. It belongs in commit messages only.

The requirements are in [`.spool/slice.req`](.spool/slice.req).

## Setup

slice builds against the `harvest`, `clickup` and `monday` crates from
worktools, which must be checked out next to this repo:

```
private-repos/
  slice/
  worktools/
```

After cloning, turn on the repo's hooks:

```
git config core.hooksPath .githooks
```

The pre-commit hook rejects a commit that changes both `src/` and `tests/`.
Tests and implementation are written and reviewed separately.

## Build and install

```
cargo build --release
cp target/release/slice ~/bin/slice
```

Credentials and workspace settings come from the same files the worktools CLIs
read, in your home directory: `~/.config/harvest/credentials`, `~/.clickup-env`
and `~/.monday-env` (see the worktools README). Which ticket goes to ClickUp and
which to monday follows `HARVEST_CLICKUP_PREFIXES` and `HARVEST_MONDAY_PREFIXES`.

## Configuration

`~/.config/slice/config.toml` is optional, and so is every key in it:

```toml
worktree_root = "~/work/slices"

[status.in_progress]
clickup = "development"
monday = "In Progress"

[status.review]
clickup = "dev complete"
monday = "Waiting for review"

[status.done]
clickup = "complete"
monday = "Done"

[harvest]
project = "Acme Product"
task = "New Development"
```

The status names shown are the defaults. `worktree_root` defaults to
`<repo>/../<repo>.slices/`. Without `[harvest]`, timers go to
`HARVEST_DEFAULT_PROJECT` and `HARVEST_DEFAULT_TASK`, the same place `hv start`
uses. State such as the
active slice and the manifests lives in `~/.local/state/slice/`.

## Tests

```
cargo test
~/bin/spool
```

Every acceptance criterion in `.spool/` must have an annotated test. Spool exits
non-zero if one doesn't. Property tests are marked `// invariant: <ID>` until
Spool supports invariant IDs.
