# Isolated agents: design

Status: Phase 1 done, 2026-11-05; see "Phase 1 results". This turns Phases 1 and 2 of *Spec-Driven Agent
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
| Reads | The slice's spec, existing tests, and the source at the stub commit, which holds no implementation of the slice | The slice's spec, the interface stub, the tests (read-only), the rest of the source |
| Writes | Test paths only | Source paths only |
| Never has | Any implementation of the slice | Write access to tests or `.spool/` |
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

1. Exports the base commit into a fresh directory with `git archive <base>`.
   For the test agent the base is the stub commit: its tests have to compile
   against the crate, and at that commit the crate holds no implementation of
   the slice. For the implementation agent it's the tests commit.
2. Records that directory as "baseline" in a throwaway git repository whose
   `.git` lives beside it, outside the container, so the agent's changes can
   be read back as a diff and the agent can't rewrite the baseline.
3. Mounts it read-only at `/work`, then mounts the role's write paths again on
   top, writable: `tests/` for the test agent, `src/` for the implementation
   agent. A write anywhere else fails at the filesystem, whatever the agent
   tries.

   Go keeps each test beside the code it tests, so there's no subtree to
   mount writable. There the workspace is writable, and every tracked file
   the role mustn't write is mounted read-only on top, one mount per file,
   generated from `git ls-files`: every non-test file for the test agent,
   and every `_test.go` file plus the spec and module files for the
   implementation agent. Existing files still fail closed. A new file of the
   wrong kind can be created, so the path check after the run is what
   rejects it.
4. After the run, takes the diff against baseline, untracked files included,
   as the patch.
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

The image build pre-compiles them, and a fresh clone of slice builds on its
own. worktools is a private repository, so Cargo fetches it with the git CLI
and your own credentials (`net.git-fetch-with-cli` in slice's
`.cargo/config.toml`); see "Images" for how the image gets it without them.
For work on both repositories at once, a `[patch]` entry in your user-level
`~/.cargo/config.toml` points slice back at `../worktools`. It lives outside
the repository, so it's never committed, but it overrides the pin in every
build, so add it only while changing both and remove it afterwards. A
repository with a sibling path dependency follows the same rule.

## Images

One image per language stack, built from the repository's lockfile, so a run
needs no package registry:

- **Rust:** `rust:<pinned>-slim`, plus `git` and the Claude Code CLI at a
  pinned version. Dependencies are vendored on the host, not fetched in the
  build: `cargo vendor` runs against `Cargo.lock` with your credentials, and
  the vendored source is copied into the image along with the config that
  points Cargo at it. Neither the image build nor a run ever holds a GitHub
  credential, even though worktools is private. `CARGO_NET_OFFLINE=true` is
  set at run time, so a missing crate fails fast instead of reaching the
  network.
- **Go:** `golang:<pinned>-bookworm`, plus the Claude Code CLI at a pinned
  version, running as an unprivileged user. Modules are downloaded on the
  host into a module cache that's copied into the image, so, as for Rust,
  neither the build nor a run holds a credential. `GOPROXY=off`,
  `GOFLAGS=-mod=readonly` and `GOTOOLCHAIN=local` keep a run offline and on
  the image's toolchain, and the standard library is pre-built. The tag
  hashes `go.mod`, `go.sum`, the Containerfile and the Claude Code version.
  `agents/build-go-image.sh` builds it.
- **Node:** `node:<pinned>-bookworm-slim`, plus `git` and the Claude Code CLI
  at a pinned version, running as the base image's `node` user. The npm
  registry is public, so `npm ci --ignore-scripts` runs in the image build,
  and the result moves to `/node_modules`, above the workspace at `/work`.
  Node and TypeScript find it by walking up from the workspace, so the
  workspace holds only the repository. `npm_config_offline=true` keeps a run
  from installing anything. The tag hashes `package.json`,
  `package-lock.json`, the Containerfile and the Claude Code version.
  `agents/build-node-image.sh` builds it.

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

## Phase 1 results

Run on 2026-11-04 for SLICE-7 (`slice abandon --reason`). Both agents
finished `green`, with no questions, and every check passed again on the host.

| | Test agent | Implementation agent |
| --- | --- | --- |
| Model | `opus` | `sonnet` |
| Time | 2 min 41 s | 42 s |
| Turns | 22 | 19 |
| Reported cost | $1.21 | $0.23 |
| Patch | 311 lines, `tests/` only | 28 lines, `src/` only |
| Outcome | 6 new tests failing against the stub, 3 passing for declared reasons | All 83 tests passing, clippy and fmt clean |

Neither run went in circles, and neither came near the timeout, so there's no
data yet for the churn guard's thresholds.

### Manual steps, in order

What `slice run` and `slice verify` have to automate:

1. Build a Linux Spool for the container's architecture.
2. Export the workspace and record its baseline.
3. Write the role's prompt, task and report schema.
4. Run the container: the workspace mounts, the token's env file, the model
   for the role, and a timeout.
5. Read the report from the JSON output.
6. Take the patch and check that it touches only the role's write paths.
7. Apply it to the branch with `git apply --index`.
8. On the host: `cargo fmt --check`, `cargo clippy --all-targets`,
   `spool`, and `cargo test --no-fail-fast`. For the test agent, also
   check which new tests fail against the stub.
9. Show the diff, and commit once it's approved.

### What went wrong, and what changed

- **The test agent needs the whole crate.** Rust tests don't compile without
  the source, so its workspace is the whole tree at the stub commit. The
  Roles table and "Building a workspace" now say so.
- **Per-path read-only mounts were the wrong way round.** Mounting the
  workspace read-only with one writable path on top is simpler and fails
  closed. "Building a workspace" now says so.
- **Full model IDs weren't recognized** by Claude Code 2.1.291. The aliases
  `opus` and `sonnet` worked.
- **`--append-system-prompt-file` isn't in this version's help**, so the prompt
  is passed inline. `--json-schema` takes the schema itself, not a path.
- **The image needed four fixes:**
  - `cargo vendor --quiet` also drops the source-replacement config it prints.
  - Vendored files kept owner-only permissions from the host.
  - The stand-in crate used to pre-build dependencies looked newer than the
    exported files, which carry their commit times, so Cargo reused it. The
    image now cleans it out.
  - It had no `rustfmt` or `clippy`, so the test agent couldn't check its own
    formatting.

  The tag now hashes `Cargo.lock`, the Containerfile and the Claude Code
  version, because a Containerfile change used to keep the old tag.
- **File ownership was fine without `--userns=keep-id`.** Podman's file
  sharing on macOS returns the agent's files owned by you. Files the agent
  edited came back owner-only (`0600`), which git doesn't record.
- **Some tests can't fail against a stub.** Behavior that already exists
  before the slice (here, reading older manifests, and the reason never
  leaking because the stub drops it) passes from the start. The test agent
  declared each one in `assumptions`. `slice verify` should accept a passing
  new test only when the report declares it.
- **Output has to be testable.** ACs about printed output need that output
  built in the library, not in `main.rs`, so the stub moved it there. That's
  worth doing in every stub.
- **Command-line tests need an isolated home.** The test agent ran the binary
  with a cleared environment, temporary `HOME` and XDG directories, and a
  broken config file, so a run that got past the argument checks couldn't
  reach Harvest or a ticket system. Its temporary directories aren't cleaned
  up yet.
- **Spool doesn't track invariants.** The test agent used the repository's
  `// invariant:` convention, which `spool` ignores. Fixed since:
  Spool tracks invariants (INVARIANT-1, built in the Go runs below), and
  slice's invariants use `SLICE-<n>-INV-<n>` IDs, so a missing invariant
  test fails `spool`.
- **The token can leak through the editor.** Selecting the token file's line
  in the editor sent it into the conversation. The token now lives at
  `~/.config/slice/agent.env`, mode 600, outside every repository.

## Go runs

Run on 2026-11-05 and 2026-11-06 against Spool, in the Go image: INVARIANT-1
(Spool tracks invariants like ACs) and INVARIANT-2 (editor support for them).
All four runs finished `green`, with no questions, and every check passed
again on the host.

| | INVARIANT-1 tests | INVARIANT-1 code | INVARIANT-2 tests | INVARIANT-2 code |
| --- | --- | --- | --- | --- |
| Model | `opus` | `sonnet` | `opus` | `sonnet` |
| Time | 4 min 52 s | 40 s | 2 min 37 s | 36 s |
| Turns | 19 | 8 | 16 | 7 |
| Reported cost | $1.80 | $0.29 | $1.13 | $0.25 |
| Patch | 918 lines, 8 test files | 174 lines, 10 files | 719 lines, 1 test file | 147 lines, 5 files |
| Outcome | 22 tests failing against the stub, 6 passing as declared | All tests passing | 16 failing, 4 passing as declared | All tests passing |

Again nothing went in circles. The test agent is where the time and money go:
four to six times the implementation agent's cost, scaling with how many
packages a slice touches.

What was new:

- **Go needs a read-only mount per file.** See "Building a workspace". 44
  and 46 mounts worked without trouble, so `slice run` should generate them
  from the repository's path globs rather than expect a writable subtree.
- **An existing test can pin behavior the spec changes.** A test comparing
  every severity default had to learn the new one, and the implementation
  agent couldn't have edited it. The test agent updated it, annotated it and
  declared it. The role prompt now allows exactly that, and `slice verify`
  should accept a changed existing test only when the report declares it.
- **The workspace has no git history.** It's a `git archive` export, so the
  task can't say "the last two commits"; it has to name the files.
- **Formatting debt collides with the formatting check.** One file wasn't
  `gofmt`-clean before the slice, and the implementation agent had to edit
  it. The task told it to format the whole file; the alternative, exempting
  it, would leave the check meaningless for that file.

## Node run

Run on 2026-11-06 against spool-vscode, in the Node image: TREE-1
(invariants in the editor's Requirements tree). Both runs finished `green`,
with no questions, and every check passed again on the host.

| | TREE-1 tests | TREE-1 code |
| --- | --- | --- |
| Model | `opus` | `sonnet` |
| Time | 56 s | 17 s |
| Turns | 8 | 11 |
| Reported cost | $0.41 | $0.13 |
| Patch | 218 lines, 1 test file | 48 lines, 1 file |
| Outcome | 9 tests failing against the stub, 4 passing as declared | All tests passing |

The cheapest slice so far, because the stub had already narrowed it to one
module of pure functions.

What was new:

- **Code that imports `vscode` can't be tested in a container.** The module
  exists only inside the editor. The stub moved the tree's logic into
  `treeModel.ts`, which imports nothing from `vscode`, and left
  `treeView.ts` a thin wrapper. The test agent tested the model and said
  so. For any editor or UI code, the stub step is where that split has to
  happen; the test agent can't make it.
- **Node keeps tests beside the code too.** `src/*.test.ts` sits next to
  `src/*.ts`, so the workspace is mounted the Go way, with a read-only
  mount per file (16 for the test agent, 13 for the implementation agent).
- **Spool has to be in the container.** The agents run Spool to check their
  annotations, and the Node image has no Go toolchain to build it. A Linux
  build of Spool was mounted read-only at `/usr/local/bin/spool`. `slice
  run` should do the same for every image, from one pinned Spool build.
- **Compatibility tests pass against the stub.** TREE-1-6 says an older
  server changes nothing, so its tests describe today's behavior and pass
  before any code is written. The test agent declared all four such tests,
  as the role prompt asks, which is what `slice verify` will check.

## What `slice run` does first

1. Phase 1's manual steps 1 to 7 for one role: export, mounts, role files from
   templates, the run, the path check, and apply. The image is chosen from
   the repository's lockfile, and a pinned Linux build of Spool is mounted
   into every run.
2. `slice verify`: step 8, including the check of which tests fail against
   the stub, with declared exceptions.
3. The churn guard, which watches the run through `--output-format
   stream-json`. Thresholds wait until a run actually goes in circles.
4. The network allow-list proxy. Phase 1 used ordinary networking.
