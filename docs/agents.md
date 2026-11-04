# Isolated agents: design

Status: draft, 2026-11-03. This turns Phases 1 and 2 of *Spec-Driven Agent
Pipeline — Conceptual Design* into something buildable. The conceptual design
says what and why; this says how, on this machine, with these tools.

## Goal

A test agent and an implementation agent each run in a throwaway container
that holds only the files its role may see. The test agent never sees the
implementation. The implementation agent can't change the tests. Each returns a
patch and a structured report; nothing else leaves the container.

Phase 1 runs one real slice with the containers started by hand, to find the
friction. Phase 2 automates what Phase 1 shows is worth automating, as
`slice run` and `slice verify`.

## Decisions

| Decision | Choice | Consequences |
| --- | --- | --- |
| Container runtime | Podman | Free, rootless, Docker-compatible CLI, already installed. Its VM (`podman-machine-default`, 2 GB, never started) needs more memory for Rust and Node builds: about 8 GB and 6 CPUs. |
| Agent auth | Subscription token | A long-lived token from `claude setup-token`, passed in as `CLAUDE_CODE_OAUTH_TOKEN`. Runs use your plan and share its limits. `--bare` is ruled out, because it accepts only an API key, so a predictable start comes from the flags and the container instead (see "Running the agent"). |
| Where the design lives | `docs/agents.md` in slice | Next to the launcher and verifier it leads to. Their specs go in `.spool/` as usual. |
| Token in the container | Accepted | The agent can read `CLAUDE_CODE_OAUTH_TOKEN` from its own environment, but the network allow-list leaves it nowhere to send it except the API it's already for. No token-injecting proxy. |
| Run limits | Wall-clock timeout and the churn guard | `--max-budget-usd` isn't relied on: on a subscription token it's unlikely to stop a run. The churn guard is the important one, because it catches a run going in circles long before a timeout would. |
| Models | Per role | A strong model for the test agent, since the tests define the behavior; a fast one for the implementation agent. Revisited after Phase 1. |
| Phase 1 slice | `slice abandon --reason` | Small, Rust, and deliberately left out of SLICE-6, so its spec is a natural follow-on. |

## Roles and what each container holds

| | Test agent | Implementation agent |
| --- | --- | --- |
| Reads | The slice's spec, the interface stub, existing tests | The slice's spec, the interface stub, the tests (read-only), the rest of the source |
| Writes | Test paths only | Source paths only |
| Never has | Any implementation beyond the stub | Write access to tests or `.spool/` |
| Done when | The tests compile, fail against the stub, and Spool shows every slice ID covered | The tests pass, unchanged |

"Source paths" and "test paths" are per repository. A repository declares them
once, for example in `.slice.toml`:

```toml
[paths]
test = ["tests/**"]
source = ["src/**"]
spec = [".spool/**", ".spool.yaml"]
shared = ["Cargo.toml", "Cargo.lock", "rustfmt.toml"]   # read by both, written by neither
```

slice's own pre-commit hook already enforces the same source/test split on the
host.

## Building a workspace

The launcher never mounts the repository. For each run it:

1. Exports the base commit's allowed paths into a fresh directory with
   `git archive <base> -- <paths>`. For the test agent that means the spec,
   the stub file(s), existing tests and shared files, but no other source.
2. Makes that directory a throwaway git repository and commits it as
   "baseline", so the agent's changes can be read back as a diff.
3. Mounts it at `/work`. For the implementation agent, the test and spec paths
   are bind-mounted again on top, read-only (`:ro`). A write to them fails at
   the filesystem, whatever the agent tries.
4. After the run, takes `git diff baseline` in the workspace as the patch.
   A patch that touches a path outside the role's write list is rejected
   before anything reaches the host repository.

The workspace has no remote and no credentials, so nothing can be pushed from
inside it.

**Dependencies come from the image, never the workspace.** A workspace holds
only the repository's own allowed paths. Anything the build needs from
elsewhere has to arrive through the lockfile, so it's fetched into the image
like any other dependency.

slice links worktools' `harvest`, `clickup` and `monday` crates as libraries,
so it needs their source to compile; the installed `hv`, `cu` and `mon`
binaries wouldn't help, and agents never run them (they'd need your
credentials, and housekeeping is the orchestrator's job). Today those are path
dependencies on `../worktools`. Before Phase 1 they become git dependencies
pinned in `Cargo.lock`:

```toml
clickup = { git = "https://github.com/mwames/worktools", rev = "…" }
```

The image build fetches and pre-compiles them, which also lets a fresh clone
of slice build on its own. For work on both repositories at once, a `[patch]`
entry in your user-level `~/.cargo/config.toml` points slice back at
`../worktools`; it lives outside the repository, so it's never committed. A
repository with a sibling path dependency follows the same rule.

## Images

One image per language stack, built from the repository's lockfile, so a run
needs no package registry:

- **Rust:** `rust:<pinned>-slim`, plus `git` and the Claude Code CLI at a
  pinned version. `cargo fetch` runs at build time against `Cargo.lock`, and
  `CARGO_NET_OFFLINE=true` is set at run time, so a missing crate fails fast
  instead of reaching the network.
- **Node** (provision-web and similar), later: `node:<.nvmrc>-slim`, `npm ci`
  at build time.

The image is rebuilt when the lockfile changes. Its tag carries a hash of the
lockfile, so a stale image is never used by mistake.

## Network

Containers reach the Claude API and nothing else:

- Each run joins an internal Podman network with no route out.
- A small proxy container on that network allows only `api.anthropic.com`
  (and whatever host the subscription token authenticates against; to confirm
  in Phase 1).
- The agent gets `HTTPS_PROXY` pointing at the proxy, which Claude Code honors.

Phase 1 may start with ordinary networking to get a run working, then close it
down. The proxy is in place before Phase 2.

## Running the agent

Inside the container, the agent runs headless:

```sh
claude -p "$(cat /role/task.md)" \
  --append-system-prompt-file /role/prompt.md \
  --model <per role> \
  --output-format json --json-schema /role/report.schema.json \
  --permission-mode bypassPermissions \
  --setting-sources "" --strict-mcp-config \
  --no-session-persistence
```

- **Permissions are bypassed because the container is the sandbox.** The
  read-only mounts and the network do the enforcing, not the prompt.
- **A predictable start without `--bare`:** no user settings exist in a fresh
  container, `--setting-sources ""` ignores any that do, and
  `--strict-mcp-config` loads no MCP servers. The repository's own `CLAUDE.md`
  is still read, which is wanted.
- **Limits come from outside the agent.** The CLI has no turn limit, and its
  dollar cap isn't relied on with a subscription token. The launcher stops the
  container at a wall-clock timeout, and the churn guard (below) stops a run
  that has stopped making progress.

## Role prompts and the report

Each role gets a short prompt file: what it may change, what "done" means,
and that it must end with the report. The report is checked against a JSON
Schema, so the orchestrator reads fields rather than prose:

```json
{
  "status": "green | blocked",
  "files_touched": ["tests/abandon.rs"],
  "assumptions": ["Abandon keeps the Harvest entry because SLICE-6-1 says so"],
  "questions": [],
  "summary": "≤ 500 characters"
}
```

`blocked` sends the slice back to the spec step, as the conceptual design says;
the agent isn't simply run again.

## What the host does afterwards

These are Phase 2. In Phase 1 they're done by hand.

- **Apply:** check the patch's paths against the role's write list, then apply
  it to the slice's worktree and commit it. The tests commit and the
  implementation commit stay separate, as the pre-commit hook requires.
- **Lock:** after the tests commit, record a hash of the test paths in the
  slice manifest.
- **Verify (`slice verify`):** in a clean container, check that:
  - the tests pass,
  - the test hash still matches,
  - Spool shows every slice ID covered,
  - the diff is under the size limit,
  - lint passes.

  It exits non-zero on any failure. It depends on Spool reporting coverage
  for a list of IDs as JSON, which is on the roadmap.
- **Churn guard:** stop a run early when the passing-test count stops rising,
  the same test keeps failing, or edits undo earlier edits. It's the main
  limit on a run, so it's the first thing `slice run` gets after applying
  patches. Thresholds come from what Phase 1 shows.

## Phase 1 plan

1. Give the Podman VM about 8 GB and 6 CPUs, and start it. Switch slice's
   worktools dependencies from paths to a pinned git revision.
2. Create the subscription token with `claude setup-token` and keep it in a
   file outside every repository, read only by the launcher.
3. Build the Rust image and check `cargo test` runs offline in it against
   slice.
4. Spec `slice abandon --reason` and approve its stub on the host as usual.
5. Test agent: export the workspace, start the container, run the agent,
   take the patch and report. Check that the tests fail against the stub and
   that Spool coverage is complete. Check who owns the files the agent wrote:
   a rootless container can hand them back under a mapped user ID, which
   `--userns=keep-id` fixes. Commit.
6. Implementation agent: the same, with tests mounted read-only. Check the
   tests pass. Commit.
7. Write down every manual step and everything that went wrong, including
   how each run ended and whether it was going in circles before it did. That
   list decides what `slice run` does first and sets the churn guard's
   thresholds.
