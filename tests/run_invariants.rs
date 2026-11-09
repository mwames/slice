//! Property tests for the `slice run` invariants (SLICE-8-INV-1,
//! SLICE-9-INV-1) over generated roles, agent edits, report statuses and
//! container endings.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use common::agent::*;
use proptest::prelude::*;
use slice::run::*;

const PATHS: [&str; 8] = [
    "src/lib.rs",
    "src/deep/mod.rs",
    "src/new.rs",
    "tests/a.rs",
    "tests/new.rs",
    "tests/common/mod.rs",
    "README.md",
    "docs/new.md",
];

#[derive(Clone, Debug)]
enum Ending {
    Exited(i32),
    TimedOut,
    FailedToStart,
    NoReport,
}

#[derive(Clone, Debug)]
struct Case {
    role: Role,
    /// Indexes into PATHS, with whether to delete rather than write.
    edits: Vec<(usize, bool)>,
    status: &'static str,
    ending: Ending,
    /// Whether `stage_work` is staged before the run.
    staged_first: bool,
}

fn case() -> impl Strategy<Value = Case> {
    (
        prop_oneof![Just(Role::Test), Just(Role::Impl)],
        prop::collection::vec((0..PATHS.len(), any::<bool>()), 0..4),
        prop_oneof![Just("green"), Just("blocked")],
        prop_oneof![
            4 => Just(Ending::Exited(0)),
            1 => Just(Ending::Exited(1)),
            1 => Just(Ending::TimedOut),
            1 => Just(Ending::FailedToStart),
            1 => Just(Ending::NoReport),
        ],
        any::<bool>(),
    )
        .prop_map(|(role, edits, status, ending, staged_first)| Case {
            role,
            edits,
            status,
            ending,
            staged_first,
        })
}

/// Whether `role` may write `path` under the standard `.slice.toml`.
fn writable(role: Role, path: &str) -> bool {
    match role {
        Role::Test => path.starts_with("tests/"),
        Role::Impl => path.starts_with("src/"),
    }
}

fn fake(case: &Case) -> FakeContainers {
    let mut edits = Vec::new();
    let mut deleted = Vec::new();
    for &(i, delete) in &case.edits {
        let path = PATHS[i];
        let exists = base_files().iter().any(|(p, _)| *p == path) && !deleted.contains(&path);
        if delete && exists {
            edits.push(Edit::Delete(path));
            deleted.push(path);
        } else if !delete {
            edits.push(Edit::Write(path, "// written by the agent\n"));
            deleted.retain(|p| *p != path);
        }
    }
    let stdout = claude_output(report(case.status, &[]));
    let output = match case.ending {
        Ending::Exited(code) => exited(code, &stdout),
        Ending::TimedOut => ContainerOutput {
            exit: ContainerExit::TimedOut,
            stdout: String::new(),
            stderr: String::new(),
        },
        Ending::FailedToStart => ContainerOutput {
            exit: ContainerExit::FailedToStart("no podman".into()),
            stdout: String::new(),
            stderr: String::new(),
        },
        Ending::NoReport => exited(0, "{\"type\":\"result\",\"is_error\":true}"),
    };
    FakeContainers::new(edits, output)
}

/// A repository with untracked files beside the tracked ones, and staged work
/// if `staged_first`, and settings whose env file holds `SECRET` and can't be
/// read.
fn setup(name: &str, staged_first: bool) -> (TempDir, std::path::PathBuf, AgentSettings) {
    let (root, repo) = repo(name);
    if staged_first {
        stage_work(&repo);
    }
    write(&repo, "notes.txt", "untracked\n");
    let settings = agent_settings(&root);
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&settings.env_file, std::fs::Permissions::from_mode(0o000)).unwrap();
    }
    (root, repo, settings)
}

/// The index entries in `git ls-files --stage` output, by path.
fn index_entries(index: &str) -> BTreeMap<String, String> {
    index
        .lines()
        .map(|l| {
            let (entry, path) = l.split_once('\t').unwrap();
            (path.to_string(), entry.to_string())
        })
        .collect()
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn check_mounts(spec: &ContainerSpec, run_dir: &Path, settings: &AgentSettings) -> Result<(), TestCaseError> {
    prop_assert_eq!(&spec.env_file, &settings.env_file, "the env file is passed by path");
    for m in &spec.mounts {
        prop_assert!(m.host != settings.env_file, "the env file is never mounted: {:?}", m);
        if m.host == settings.spool {
            prop_assert!(m.read_only, "Spool is mounted read-only: {:?}", m);
        } else {
            prop_assert!(m.host.starts_with(run_dir), "{:?} is inside {}", m, run_dir.display());
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    // SLICE-8-INV-1
    #[test]
    fn container_never_sees_the_repository(case in case()) {
        let (_root, repo, settings) = setup("inv-mounts", case.staged_first);
        let fake = fake(&case);
        let request = request(&_root, &repo, case.role);

        let result = run(&fake, Ok(settings.clone()), &request).expect("a container is started");

        prop_assert_eq!(fake.calls(), 1);
        check_mounts(&fake.spec(), &result.run_dir, &settings)?;
        for (path, content) in files(&result.run_dir, false) {
            prop_assert!(!contains(&content, SECRET), "{} holds the token", path);
        }
        prop_assert!(!result.message().contains(SECRET));
        prop_assert!(!result.to_json().to_string().contains(SECRET));
    }

    // SLICE-9-INV-1
    #[test]
    fn only_the_write_set_is_staged(case in case()) {
        let (_root, repo, settings) = setup("inv-staging", case.staged_first);
        let before = repo_state(&repo);
        let staged_before = staged_paths(&repo);
        let fake = fake(&case);
        let request = request(&_root, &repo, case.role);

        let result = run(&fake, Ok(settings), &request).expect("a container is started");

        let after = repo_state(&repo);
        prop_assert_eq!(&after.head, &before.head, "HEAD never moves");
        prop_assert_eq!(&after.refs, &before.refs, "no branch moves, nothing is committed");
        for line in staged(&repo) {
            let path = line.split_once(' ').unwrap().1;
            prop_assert!(
                writable(case.role, path) || staged_before.iter().any(|p| p == path),
                "{} staged for {:?}", path, case.role
            );
        }
        let staged_after = staged_paths(&repo);
        for path in &staged_before {
            prop_assert!(staged_after.contains(path), "{} is still staged", path);
        }
        let (entries_before, entries_after) = (index_entries(&before.index), index_entries(&after.index));
        for path in entries_before.keys().chain(entries_after.keys()) {
            if !writable(case.role, path) {
                prop_assert_eq!(
                    entries_before.get(path), entries_after.get(path),
                    "what is staged for {} is unchanged", path
                );
            }
        }
        if result.exit_code() != 0 {
            prop_assert_eq!(&after, &before, "the repository is as it was");
        }
    }
}
