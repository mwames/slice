//! `slice run` as a library: the configuration, write sets, workspace,
//! container spec, report, path check and staging (SLICE-8, SLICE-9), on real
//! git repositories in temporary directories with a fake container runtime.

mod common;

use std::path::{Path, PathBuf};

use common::agent::*;
use serde_json::{Value, json};
use slice::config::Config;
use slice::run::*;

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// A run that started no container and created no run directory.
fn assert_refused(fake: &FakeContainers, request: &RunRequest) {
    assert_eq!(fake.calls(), 0, "no container started");
    assert_eq!(run_dirs(&request.runs_dir), Vec::<PathBuf>::new(), "no run directory");
}

/// Run `fake` as `role` on a fresh standard repository and return the
/// repository, its root, the request and the result.
fn run_on_repo(name: &str, role: Role, fake: &FakeContainers) -> (TempDir, PathBuf, RunRequest, RunResult) {
    let (root, repo) = repo(name);
    let request = request(&root, &repo, role);
    let result = run(fake, Ok(agent_settings(&root)), &request).expect("the run starts a container");
    (root, repo, request, result)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

// SLICE-8-1
#[test]
fn role_is_test_or_impl() {
    assert_eq!(Role::parse("test"), Some(Role::Test));
    assert_eq!(Role::parse("impl"), Some(Role::Impl));
    for other in ["", "tests", "Test", "IMPL", "implementation", "source", " test"] {
        assert_eq!(Role::parse(other), None, "{other:?}");
    }
}

/// A run refused because the working tree differs from the index, naming
/// exactly `paths`.
fn assert_unstaged(fake: &FakeContainers, request: &RunRequest, err: RunError, paths: &[&str]) {
    let message = err.message();
    let RunError::Dirty(unstaged) = err else {
        panic!("refused as dirty, not {err:?}");
    };
    assert_eq!(unstaged.paths, strings(paths));
    for p in paths {
        assert!(message.contains(p), "names {p}: {message}");
    }
    assert_refused(fake, request);
}

// SLICE-8-2
#[test]
fn unstaged_change_to_a_tracked_file_is_refused() {
    let (root, repo) = repo("dirty-unstaged");
    write(&repo, "src/lib.rs", "pub fn f() -> u32 {\n    2\n}\n");
    let fake = FakeContainers::unused();
    let request = request(&root, &repo, Role::Test);

    let err = run(&fake, Ok(agent_settings(&root)), &request).unwrap_err();

    assert!(matches!(err, RunError::Dirty(_)), "{err:?}");
    assert!(!err.message().trim().is_empty());
    assert_unstaged(&fake, &request, err, &["src/lib.rs"]);
}

// SLICE-8-2
#[test]
fn staged_changes_are_allowed() {
    let (root, repo) = repo("staged-allowed");
    stage_work(&repo);
    let fake = FakeContainers::reporting(vec![], green(&[]));
    let request = request(&root, &repo, Role::Impl);

    let result = run(&fake, Ok(agent_settings(&root)), &request);

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fake.calls(), 1, "the container ran");
}

// SLICE-8-2
#[test]
fn deleted_tracked_file_is_refused() {
    let (root, repo) = repo("dirty-deleted");
    std::fs::remove_file(repo.join("tests/a.rs")).unwrap();
    let fake = FakeContainers::unused();
    let request = request(&root, &repo, Role::Test);

    let err = run(&fake, Ok(agent_settings(&root)), &request).unwrap_err();

    assert!(matches!(err, RunError::Dirty(_)), "{err:?}");
    assert_unstaged(&fake, &request, err, &["tests/a.rs"]);
}

// SLICE-8-2
#[test]
fn only_the_unstaged_paths_are_named_beside_staged_ones() {
    // Staged work, a staged file changed again, a staged new file changed
    // again, and an unstaged change and deletion.
    let (root, repo) = repo("dirty-mixed");
    stage_work(&repo);
    write(&repo, "tests/a.rs", "// changed again after staging\n");
    write(&repo, "docs/staged.md", "changed again after staging\n");
    write(&repo, "src/lib.rs", "// unstaged\n");
    std::fs::remove_file(repo.join("src/deep/mod.rs")).unwrap();
    let fake = FakeContainers::unused();
    let request = request(&root, &repo, Role::Test);

    let err = run(&fake, Ok(agent_settings(&root)), &request).unwrap_err();

    assert_unstaged(
        &fake,
        &request,
        err,
        &["docs/staged.md", "src/deep/mod.rs", "src/lib.rs", "tests/a.rs"],
    );
}

// SLICE-8-2
#[test]
fn path_that_is_not_a_repository_is_refused() {
    let root = temp_dir("not-a-repo");
    let plain = root.join("plain");
    write(&plain, "file.txt", "hello\n");
    let fake = FakeContainers::unused();
    let request = request(&root, &plain, Role::Test);

    let err = run(&fake, Ok(agent_settings(&root)), &request).unwrap_err();

    assert!(matches!(err, RunError::NotARepository(_)), "{err:?}");
    assert!(!err.message().trim().is_empty());
    assert_refused(&fake, &request);
}

// SLICE-8-2
#[test]
fn untracked_files_do_not_count_as_changes() {
    let (root, repo) = repo("untracked-ok");
    write(&repo, "notes.txt", "scratch\n");
    write(&repo, "tests/scratch.rs", "// not committed\n");
    let fake = FakeContainers::reporting(vec![], green(&[]));
    let request = request(&root, &repo, Role::Test);

    let result = run(&fake, Ok(agent_settings(&root)), &request);

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fake.calls(), 1, "the container ran");
}

/// A run refused for incomplete setup, whose message contains each of
/// `names`.
fn assert_setup_error(
    name: &str,
    slice_toml: Option<&str>,
    role: Role,
    settings: impl FnOnce(&Path) -> Result<AgentSettings, String>,
    names: &[&str],
) {
    let (root, repo) = repo_with(name, slice_toml);
    let fake = FakeContainers::unused();
    let request = request(&root, &repo, role);

    let err = run(&fake, settings(&root), &request).unwrap_err();

    let RunError::Setup(what) = &err else {
        panic!("{name}: a setup error, not {err:?}");
    };
    for n in names {
        assert!(what.contains(n), "{name}: names {n:?}: {what}");
        assert!(
            err.message().contains(n),
            "{name}: message names {n:?}: {}",
            err.message()
        );
    }
    assert_refused(&fake, &request);
}

fn ok_settings(root: &Path) -> Result<AgentSettings, String> {
    Ok(agent_settings(root))
}

// SLICE-8-3
#[test]
fn missing_slice_toml_is_named() {
    assert_setup_error("no-toml-test", None, Role::Test, ok_settings, &[".slice.toml"]);
    assert_setup_error("no-toml-impl", None, Role::Impl, ok_settings, &[".slice.toml"]);
}

// SLICE-8-3
#[test]
fn missing_globs_for_the_role_are_named() {
    assert_setup_error(
        "no-test-globs",
        Some("[paths]\nsource = [\"src/**\"]\n"),
        Role::Test,
        ok_settings,
        &["test"],
    );
    assert_setup_error(
        "empty-test-globs",
        Some("[paths]\ntest = []\nsource = [\"src/**\"]\n"),
        Role::Test,
        ok_settings,
        &["test"],
    );
    assert_setup_error(
        "no-source-globs",
        Some("[paths]\ntest = [\"tests/**\"]\n"),
        Role::Impl,
        ok_settings,
        &["source"],
    );
    assert_setup_error(
        "empty-source-globs",
        Some("[paths]\ntest = [\"tests/**\"]\nsource = []\n"),
        Role::Impl,
        ok_settings,
        &["source"],
    );
}

// SLICE-8-3
#[test]
fn role_prompt_missing_from_the_index_is_named() {
    // The default prompt path, present in the working tree but not tracked.
    let (root, repo) = repo("no-prompt-at-head");
    git(&repo, &["rm", "-q", "--cached", ".slice/test-agent.md"]);
    git(&repo, &["commit", "-q", "-m", "untrack the prompt"]);
    assert!(repo.join(".slice/test-agent.md").exists());
    let fake = FakeContainers::unused();
    let request = request(&root, &repo, Role::Test);

    let err = run(&fake, Ok(agent_settings(&root)), &request).unwrap_err();

    let RunError::Setup(what) = &err else {
        panic!("a setup error, not {err:?}");
    };
    assert!(what.contains(".slice/test-agent.md"), "{what}");
    assert_refused(&fake, &request);

    // A configured prompt path that doesn't exist.
    assert_setup_error(
        "no-configured-prompt",
        Some("[paths]\ntest = [\"tests/**\"]\nsource = [\"src/**\"]\n[agent]\nimpl_prompt = \"prompts/impl.md\"\n"),
        Role::Impl,
        ok_settings,
        &["prompts/impl.md"],
    );
}

// SLICE-8-3
#[test]
fn role_prompt_whose_deletion_is_staged_is_named() {
    // At HEAD, but its deletion is staged: removed from the index and the
    // working tree, or only from the index, leaving an untracked file.
    for (name, cached) in [("prompt-deletion-staged", false), ("prompt-untracked-staged", true)] {
        let (root, repo) = repo(name);
        if cached {
            git(&repo, &["rm", "-q", "--cached", ".slice/test-agent.md"]);
        } else {
            git(&repo, &["rm", "-q", ".slice/test-agent.md"]);
        }
        let fake = FakeContainers::unused();
        let request = request(&root, &repo, Role::Test);

        let err = run(&fake, Ok(agent_settings(&root)), &request).unwrap_err();

        let RunError::Setup(what) = &err else {
            panic!("{name}: a setup error, not {err:?}");
        };
        assert!(what.contains(".slice/test-agent.md"), "{name}: {what}");
        assert_refused(&fake, &request);
    }
}

// SLICE-8-3
#[test]
fn role_prompt_staged_but_not_committed_is_enough() {
    let toml = "[paths]\ntest = [\"tests/**\"]\nsource = [\"src/**\"]\n[agent]\ntest_prompt = \"prompts/test.md\"\n";
    let (root, repo) = repo_with("prompt-staged-new", Some(toml));
    write(&repo, "prompts/test.md", "Staged test prompt.\n");
    git(&repo, &["add", "prompts/test.md"]);
    let fake = FakeContainers::reporting(vec![], green(&[]));

    let result = run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(
        fake.seen().role.get("prompt.md").map(|b| b.as_slice()),
        Some(b"Staged test prompt.\n".as_slice())
    );
}

/// A run of `role` on `repo` refused for incomplete setup, whose message
/// contains each of `names`.
fn assert_setup_error_in(what: &str, root: &Path, repo: &Path, role: Role, names: &[&str]) {
    let fake = FakeContainers::unused();
    let request = request(root, repo, role);

    let err = run(&fake, Ok(agent_settings(root)), &request).unwrap_err();

    let RunError::Setup(said) = &err else {
        panic!("{what}: a setup error, not {err:?}");
    };
    for n in names {
        assert!(said.contains(n), "{what}: names {n:?}: {said}");
        assert!(
            err.message().contains(n),
            "{what}: message names {n:?}: {}",
            err.message()
        );
    }
    assert_refused(&fake, &request);
}

// SLICE-8-3
#[test]
fn slice_toml_missing_from_the_index_is_named() {
    // Committed, then its deletion staged: removed from the index and the
    // working tree, or only from the index, leaving an untracked file.
    for (name, cached) in [("toml-deletion-staged", false), ("toml-untracked-staged", true)] {
        let (root, repo) = repo(name);
        if cached {
            git(&repo, &["rm", "-q", "--cached", ".slice.toml"]);
        } else {
            git(&repo, &["rm", "-q", ".slice.toml"]);
        }
        assert_setup_error_in(name, &root, &repo, Role::Test, &[".slice.toml"]);
        assert_setup_error_in(name, &root, &repo, Role::Impl, &[".slice.toml"]);
    }

    // Never committed or staged, only in the working tree.
    let (root, repo) = repo_with("toml-untracked", None);
    write(&repo, ".slice.toml", SLICE_TOML);
    assert_setup_error_in("toml-untracked", &root, &repo, Role::Test, &[".slice.toml"]);
}

// SLICE-8-3
#[test]
fn slice_toml_staged_but_not_committed_is_used() {
    let (root, repo) = repo_with("toml-staged-new", None);
    write(
        &repo,
        ".slice.toml",
        "[paths]\ntest = [\"tests/**\"]\nsource = [\"src/**\"]\n[agent]\ntest_model = \"staged-model\"\ntimeout_secs = 60\n",
    );
    git(&repo, &["add", ".slice.toml"]);
    let fake = FakeContainers::reporting(vec![], green(&[]));

    let result = run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test));

    assert!(result.is_ok(), "{result:?}");
    let spec = fake.spec();
    assert_eq!(spec.timeout_secs, 60);
    let script = spec.command.join(" ");
    assert!(script.contains("staged-model"), "{script}");
    assert!(!script.contains("opus"), "{script}");
}

// SLICE-8-3
#[test]
fn staged_change_to_slice_toml_is_what_counts() {
    // HEAD's has no test globs; the staged one adds them, with a model,
    // while the working tree matches the index.
    let (root, repo) = repo_with("toml-staged-change", Some("[paths]\nsource = [\"src/**\"]\n"));
    write(
        &repo,
        ".slice.toml",
        "[paths]\ntest = [\"tests/**\"]\nsource = [\"src/**\"]\n[agent]\ntest_model = \"staged-model\"\n",
    );
    git(&repo, &["add", ".slice.toml"]);
    let fake = FakeContainers::reporting(vec![], green(&[]));

    let result = run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test));

    assert!(result.is_ok(), "{result:?}");
    let script = fake.spec().command.join(" ");
    assert!(script.contains("staged-model"), "{script}");
    assert!(!script.contains("opus"), "{script}");

    // HEAD's is complete; the staged one drops the test globs.
    let (root, repo) = repo_with("toml-staged-no-globs", Some(SLICE_TOML));
    write(&repo, ".slice.toml", "[paths]\nsource = [\"src/**\"]\n");
    git(&repo, &["add", ".slice.toml"]);
    assert_setup_error_in("toml-staged-no-globs", &root, &repo, Role::Test, &["test"]);
}

// SLICE-8-3
// SLICE-8-4
#[test]
fn unknown_key_in_staged_slice_toml_is_named() {
    let (root, repo) = repo("toml-staged-unknown-key");
    write(
        &repo,
        ".slice.toml",
        "[paths]\ntest = [\"tests/**\"]\nsource = [\"src/**\"]\nshared = [\"Cargo.toml\"]\n",
    );
    git(&repo, &["add", ".slice.toml"]);
    assert_setup_error_in("toml-staged-unknown-key", &root, &repo, Role::Test, &["shared"]);
}

// SLICE-8-3
#[test]
fn missing_spool_setting_is_named() {
    assert_setup_error(
        "no-spool",
        Some(SLICE_TOML),
        Role::Test,
        |_| Err("no Spool binary: set [agents] spool in the slice config".into()),
        &["spool"],
    );
}

// SLICE-8-3
#[test]
fn agent_settings_need_an_existing_spool_binary() {
    let root = temp_dir("agent-settings");

    let none = Config::parse("").unwrap().agent_settings().unwrap_err();
    assert!(none.to_lowercase().contains("spool"), "{none}");
    let empty = Config::parse("[agents]\nenv_file = \"/tmp/x.env\"\n")
        .unwrap()
        .agent_settings()
        .unwrap_err();
    assert!(empty.to_lowercase().contains("spool"), "{empty}");

    let missing = root.join("bin/no-such-spool");
    let config = Config::parse(&format!("[agents]\nspool = \"{}\"\n", missing.display())).unwrap();
    let err = config.agent_settings().unwrap_err();
    assert!(err.contains(&missing.display().to_string()), "{err}");
}

// SLICE-8-4
#[test]
fn repo_config_defaults() {
    let config = RepoConfig::parse(SLICE_TOML).unwrap();
    assert_eq!(
        config,
        RepoConfig {
            test_globs: strings(&["tests/**"]),
            source_globs: strings(&["src/**"]),
            test_prompt: ".slice/test-agent.md".into(),
            impl_prompt: ".slice/impl-agent.md".into(),
            test_model: "opus".into(),
            impl_model: "sonnet".into(),
            timeout_secs: 2700,
        }
    );
}

// SLICE-8-4
#[test]
fn repo_config_overrides() {
    let config = RepoConfig::parse(
        "[paths]\ntest = [\"**/*_test.go\", \"testdata/**\"]\nsource = [\"**/*.go\"]\n\
         [agent]\ntest_prompt = \"prompts/t.md\"\nimpl_prompt = \"prompts/i.md\"\n\
         test_model = \"opus-x\"\nimpl_model = \"haiku\"\ntimeout_secs = 600\n",
    )
    .unwrap();
    assert_eq!(
        config,
        RepoConfig {
            test_globs: strings(&["**/*_test.go", "testdata/**"]),
            source_globs: strings(&["**/*.go"]),
            test_prompt: "prompts/t.md".into(),
            impl_prompt: "prompts/i.md".into(),
            test_model: "opus-x".into(),
            impl_model: "haiku".into(),
            timeout_secs: 600,
        }
    );
}

// SLICE-8-4
#[test]
fn unknown_key_in_slice_toml_is_named() {
    for (text, key) in [
        (
            "[paths]\ntest = [\"t/**\"]\nsource = [\"s/**\"]\nspec = [\".spool/**\"]\n",
            "spec",
        ),
        (
            "[paths]\ntest = [\"t/**\"]\nsource = [\"s/**\"]\n[agent]\ntest_modle = \"opus\"\n",
            "test_modle",
        ),
        (
            "budget = 3\n[paths]\ntest = [\"t/**\"]\nsource = [\"s/**\"]\n",
            "budget",
        ),
        (
            "[paths]\ntest = [\"t/**\"]\nsource = [\"s/**\"]\n[agents]\nspool = \"x\"\n",
            "agents",
        ),
    ] {
        let err = RepoConfig::parse(text).unwrap_err();
        assert!(err.contains(key), "{key}: {err}");
    }
}

// SLICE-8-4
#[test]
fn unknown_key_in_slice_toml_stops_the_run() {
    assert_setup_error(
        "unknown-key",
        Some("[paths]\ntest = [\"tests/**\"]\nsource = [\"src/**\"]\n[agent]\nmax_turns = 5\n"),
        Role::Test,
        ok_settings,
        &["max_turns"],
    );
}

// SLICE-8-4
#[test]
fn agent_settings_from_slice_config() {
    let root = temp_dir("agent-settings-ok");
    let settings = agent_settings(&root);

    let config = Config::parse(&format!("[agents]\nspool = \"{}\"\n", settings.spool.display())).unwrap();
    assert_eq!(
        config.agent_settings().unwrap(),
        AgentSettings {
            spool: settings.spool.clone(),
            env_file: slice::config::home().join(".config/slice/agent.env"),
        },
        "the env file defaults to ~/.config/slice/agent.env"
    );

    let config = Config::parse(&format!(
        "[agents]\nspool = \"{}\"\nenv_file = \"{}\"\n",
        settings.spool.display(),
        settings.env_file.display()
    ))
    .unwrap();
    assert_eq!(config.agent_settings().unwrap(), settings);

    let err = Config::parse("[agents]\nspool = \"/x\"\ntoken = \"abc\"\n").unwrap_err();
    assert!(err.to_string().contains("token"), "{err}");
}

// SLICE-8-4
#[test]
fn configured_prompt_model_and_timeout_are_used() {
    let toml = "[paths]\ntest = [\"tests/**\"]\nsource = [\"src/**\"]\n\
                [agent]\nimpl_prompt = \"prompts/impl.md\"\nimpl_model = \"model-from-toml\"\ntimeout_secs = 90\n";
    let (root, repo) = repo_with("configured", Some(toml));
    write(&repo, "prompts/impl.md", "Configured implementation prompt.\n");
    git(&repo, &["add", "prompts/impl.md"]);
    git(&repo, &["commit", "-q", "-m", "prompt"]);
    let fake = FakeContainers::reporting(vec![], green(&[]));

    run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Impl)).unwrap();

    let spec = fake.spec();
    assert_eq!(spec.timeout_secs, 90);
    let script = spec.command.join(" ");
    assert!(script.contains("model-from-toml"), "{script}");
    assert!(!script.contains("sonnet"), "{script}");
    assert_eq!(
        fake.seen()
            .role
            .get("prompt.md")
            .map(|b| String::from_utf8_lossy(b).to_string()),
        Some("Configured implementation prompt.\n".to_string())
    );
}

// SLICE-8-4
#[test]
fn default_models_and_timeout_are_used() {
    let test = FakeContainers::reporting(vec![], green(&[]));
    run_on_repo("default-test-model", Role::Test, &test);
    let spec = test.spec();
    assert_eq!(spec.timeout_secs, 2700);
    let script = spec.command.join(" ");
    assert!(script.contains("opus"), "{script}");
    assert!(!script.contains("sonnet"), "{script}");

    let imp = FakeContainers::reporting(vec![], green(&[]));
    run_on_repo("default-impl-model", Role::Impl, &imp);
    let spec = imp.spec();
    assert_eq!(spec.timeout_secs, 2700);
    let script = spec.command.join(" ");
    assert!(script.contains("sonnet"), "{script}");
    assert!(!script.contains("opus"), "{script}");
}

// SLICE-8-5
#[test]
fn globs_match_whole_segments() {
    let cases: &[(&str, &str, bool)] = &[
        ("**/*_test.go", "pkg/b_test.go", true),
        ("**/*_test.go", "b_test.go", true),
        ("**/*_test.go", "a/b/c/d_test.go", true),
        ("**/*_test.go", "pkg/b.go", false),
        ("**/*.go", "a.go", true),
        ("**/*.go", "pkg/b.go", true),
        ("**/*.go", "pkg/b_test.go", true),
        ("**/*.go", "a.go.bak", false),
        ("**/*.go", "a.rs", false),
        ("tests/**", "tests/x.rs", true),
        ("tests/**", "tests/common/mod.rs", true),
        ("tests/**", "src/x.rs", false),
        ("tests/**", "tests.rs", false),
        ("tests/**", "other/tests/x.rs", false),
        ("src/**/mod.rs", "src/mod.rs", true),
        ("src/**/mod.rs", "src/a/b/mod.rs", true),
        ("src/**/mod.rs", "src/a/b/lib.rs", false),
        ("*.go", "a.go", true),
        ("*.go", "pkg/a.go", false),
        ("src/*.rs", "src/a.rs", true),
        ("src/*.rs", "src/a/b.rs", false),
        ("Cargo.toml", "Cargo.toml", true),
        ("Cargo.toml", "Cargo.lock", false),
        ("Cargo.toml", "sub/Cargo.toml", false),
        ("a?.rs", "a?.rs", true),
        ("a?.rs", "ab.rs", false),
        ("[ab].rs", "[ab].rs", true),
        ("[ab].rs", "a.rs", false),
        ("a.rs", "abrs", false),
        ("x*y/z", "xay/z", true),
        ("x*y/z", "xa/y/z", false),
    ];
    for (glob, path, expected) in cases {
        assert_eq!(glob_match(glob, path), *expected, "{glob} against {path}");
    }
}

// SLICE-8-5
#[test]
fn write_sets_for_go_layout() {
    let config = RepoConfig::parse("[paths]\ntest = [\"**/*_test.go\"]\nsource = [\"**/*.go\"]\n").unwrap();
    for path in ["a.go", "pkg/b.go"] {
        assert!(config.writable(Role::Impl, path), "{path} is an impl path");
        assert!(!config.writable(Role::Test, path), "{path} is not a test path");
    }
    for path in ["pkg/b_test.go", "a_test.go"] {
        assert!(config.writable(Role::Test, path), "{path} is a test path");
        assert!(!config.writable(Role::Impl, path), "{path} matches a test glob");
    }
    for path in ["go.mod", "README.md", ".slice.toml"] {
        assert!(!config.writable(Role::Test, path), "{path}");
        assert!(!config.writable(Role::Impl, path), "{path}");
    }
}

// SLICE-8-5
#[test]
fn write_sets_for_rust_layout() {
    let config = RepoConfig::parse("[paths]\ntest = [\"tests/**\"]\nsource = [\"src/**\"]\n").unwrap();
    for path in ["tests/x.rs", "tests/common/mod.rs"] {
        assert!(config.writable(Role::Test, path), "{path}");
        assert!(!config.writable(Role::Impl, path), "{path}");
    }
    for path in ["src/x.rs", "src/adapters/podman.rs"] {
        assert!(!config.writable(Role::Test, path), "{path}");
        assert!(config.writable(Role::Impl, path), "{path}");
    }
    for path in ["Cargo.toml", ".spool/slice.req", "testsx.rs"] {
        assert!(!config.writable(Role::Test, path), "{path}");
        assert!(!config.writable(Role::Impl, path), "{path}");
    }
}

// SLICE-8-5
#[test]
fn any_matching_glob_puts_a_path_in_the_set() {
    let config =
        RepoConfig::parse("[paths]\ntest = [\"tests/**\", \"**/testdata/**\"]\nsource = [\"src/**\", \"build.rs\"]\n")
            .unwrap();
    assert!(config.writable(Role::Test, "tests/a.rs"));
    assert!(config.writable(Role::Test, "src/testdata/in.txt"));
    assert!(
        !config.writable(Role::Impl, "src/testdata/in.txt"),
        "matches a test glob"
    );
    assert!(config.writable(Role::Impl, "build.rs"));
    assert!(config.writable(Role::Impl, "src/lib.rs"));
}

// SLICE-8-6
#[test]
fn run_directory_is_new_and_named_for_the_role() {
    let (root, repo) = repo("run-dirs");
    let settings = agent_settings(&root);
    let test = FakeContainers::reporting(vec![], green(&[]));
    let first = run(&test, Ok(settings.clone()), &request(&root, &repo, Role::Test)).unwrap();
    let imp = FakeContainers::reporting(vec![], green(&[]));
    let second = run(&imp, Ok(settings), &request(&root, &repo, Role::Impl)).unwrap();

    let runs_dir = root.join("state/slice/runs");
    for (result, role) in [(&first, "test"), (&second, "impl")] {
        assert!(result.run_dir.starts_with(&runs_dir), "{}", result.run_dir.display());
        assert!(result.run_dir.is_dir(), "{}", result.run_dir.display());
        let name = result.run_dir.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.contains(role), "{name} names {role}");
    }
    assert_ne!(first.run_dir, second.run_dir);
    let test_again = FakeContainers::reporting(vec![], green(&[]));
    let third = run(
        &test_again,
        Ok(agent_settings(&root)),
        &request(&root, &repo, Role::Test),
    )
    .unwrap();
    assert_ne!(first.run_dir, third.run_dir, "every run gets a new directory");
    assert!(first.run_dir.is_dir(), "an earlier run directory is kept");
}

// SLICE-8-6
#[test]
fn workspace_holds_exactly_the_tracked_files() {
    let (root, repo) = repo("workspace");
    write(&repo, "notes.txt", "untracked\n");
    write(&repo, "target/debug/app", "ignored\n");
    let fake = FakeContainers::reporting(vec![], green(&[]));

    let result = run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();

    let workspace = fake.seen().workspace;
    let mut expected: Vec<(String, Vec<u8>)> = base_files()
        .into_iter()
        .map(|(p, c)| (p.to_string(), c.as_bytes().to_vec()))
        .collect();
    expected.push((".slice.toml".into(), SLICE_TOML.as_bytes().to_vec()));
    expected.sort();
    assert_eq!(workspace.into_iter().collect::<Vec<_>>(), expected);

    let work = mount_at(&fake.spec(), "/work").expect("a workspace mounted at /work");
    assert!(work.starts_with(&result.run_dir), "{}", work.display());
    assert!(
        !work.join(".git").exists(),
        "the baseline is kept outside the workspace"
    );
}

/// Directories under `dir` that look like a git directory.
fn git_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if dir.join("HEAD").is_file() && dir.join("objects").is_dir() {
        out.push(dir.to_path_buf());
    }
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() && !path.is_symlink() {
            out.extend(git_dirs(&path));
        }
    }
    out
}

// SLICE-8-6
#[test]
fn baseline_is_a_git_directory_outside_the_workspace() {
    let fake = FakeContainers::reporting(vec![], green(&[]));
    let (_root, _repo, _request, result) = run_on_repo("baseline", Role::Test, &fake);

    let work = mount_at(&fake.spec(), "/work").unwrap();
    let baselines: Vec<PathBuf> = git_dirs(&result.run_dir)
        .into_iter()
        .filter(|d| !d.starts_with(&work))
        .collect();
    assert!(
        !baselines.is_empty(),
        "a git directory in {} outside the workspace",
        result.run_dir.display()
    );
}

// SLICE-8-6
#[test]
fn role_directory_holds_prompt_task_and_schema() {
    for (role, prompt) in [(Role::Test, TEST_PROMPT), (Role::Impl, IMPL_PROMPT)] {
        let fake = FakeContainers::reporting(vec![], green(&[]));
        let (_root, _repo, _request, result) = run_on_repo("role-dir", role, &fake);

        let role_dir = mount_at(&fake.spec(), "/role").expect("a role directory mounted at /role");
        assert!(role_dir.starts_with(&result.run_dir), "{}", role_dir.display());
        let seen: Vec<(String, String)> = fake
            .seen()
            .role
            .into_iter()
            .map(|(p, c)| (p, String::from_utf8(c).unwrap()))
            .collect();
        assert_eq!(
            seen,
            vec![
                ("prompt.md".to_string(), prompt.to_string()),
                ("report.schema.json".to_string(), REPORT_SCHEMA.to_string()),
                ("task.md".to_string(), TASK.to_string()),
            ],
            "{role:?}"
        );
    }
}

// SLICE-8-6
#[test]
fn prompt_is_taken_from_head() {
    // HEAD's prompt differs from an earlier commit's; the run uses HEAD's.
    let (root, repo) = repo("prompt-at-head");
    write(&repo, ".slice/test-agent.md", "Newer prompt.\n");
    git(&repo, &["commit", "-q", "-am", "newer prompt"]);
    let fake = FakeContainers::reporting(vec![], green(&[]));

    run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();

    assert_eq!(fake.seen().role.get("prompt.md").unwrap(), b"Newer prompt.\n");
    assert_eq!(
        fake.seen().workspace.get(".slice/test-agent.md").unwrap(),
        b"Newer prompt.\n"
    );
}

// SLICE-8-6
#[test]
fn prompt_is_taken_from_the_index() {
    // A staged change to the prompt, not committed.
    let (root, repo) = repo("prompt-staged");
    write(&repo, ".slice/test-agent.md", "Staged prompt.\n");
    git(&repo, &["add", ".slice/test-agent.md"]);
    let fake = FakeContainers::reporting(vec![], green(&[]));

    run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();

    assert_eq!(fake.seen().role.get("prompt.md").unwrap(), b"Staged prompt.\n");
    assert_eq!(
        fake.seen().workspace.get(".slice/test-agent.md").unwrap(),
        b"Staged prompt.\n"
    );
}

// SLICE-8-6
#[test]
fn workspace_holds_the_staged_state() {
    // Staged changes, a staged new file and a staged deletion, beside
    // untracked and ignored files.
    let (root, repo) = repo("workspace-staged");
    stage_work(&repo);
    write(&repo, "notes.txt", "untracked\n");
    write(&repo, "target/debug/app", "ignored\n");
    let expected = index_files(&repo);
    assert!(!expected.contains_key("Cargo.toml"));
    assert_eq!(expected.get("tests/staged.rs").unwrap(), b"// staged test\n");
    let fake = FakeContainers::reporting(vec![], green(&[]));

    run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();

    assert_eq!(fake.seen().workspace, expected);
}

/// Asserts the spec's mounts for a run of `role` on the standard repository,
/// with `stage_work` staged first if `with_staged`.
fn assert_mounts(role: Role, with_staged: bool, writable: impl Fn(&str) -> bool) {
    let fake = FakeContainers::reporting(vec![], green(&[]));
    let (root, repo) = repo("mounts");
    if with_staged {
        stage_work(&repo);
    }
    let index = index_files(&repo);
    let result =
        run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, role)).expect("the run starts a container");
    let settings = agent_settings(&root);
    let spec = fake.spec();

    assert_eq!(spec.image, IMAGE);
    assert_eq!(spec.env_file, settings.env_file);

    let work = spec.mounts.iter().find(|m| m.container == "/work").expect("/work");
    assert!(!work.read_only, "the workspace is writable");
    assert!(work.host.starts_with(&result.run_dir));

    let role_dir = spec.mounts.iter().find(|m| m.container == "/role").expect("/role");
    assert!(role_dir.host.starts_with(&result.run_dir));

    let spool = spec
        .mounts
        .iter()
        .find(|m| m.container == "/usr/local/bin/spool")
        .expect("spool");
    assert_eq!(spool.host, settings.spool);
    assert!(spool.read_only);

    let mut read_only: Vec<String> = Vec::new();
    for m in &spec.mounts {
        if let Some(path) = m.container.strip_prefix("/work/") {
            assert!(m.read_only, "{path} mounted read-only");
            assert!(m.host.starts_with(&result.run_dir), "{}", m.host.display());
            let staged = index.get(path).unwrap_or_else(|| panic!("{path} is in the index"));
            assert_eq!(
                &std::fs::read(&m.host).unwrap(),
                staged,
                "{path} holds its staged contents"
            );
            read_only.push(path.to_string());
        } else {
            assert!(
                ["/work", "/role", "/usr/local/bin/spool"].contains(&m.container.as_str()),
                "unexpected mount {m:?}"
            );
        }
    }
    let expected: Vec<String> = tracked(&repo).into_iter().filter(|p| !writable(p)).collect();
    assert_eq!(sorted(read_only), expected, "{role:?}");
    let containers: Vec<&str> = spec.mounts.iter().map(|m| m.container.as_str()).collect();
    for c in ["/work", "/role", "/usr/local/bin/spool"] {
        assert_eq!(containers.iter().filter(|x| **x == c).count(), 1, "{c} mounted once");
    }
    let work_index = containers.iter().position(|c| *c == "/work").unwrap();
    for (i, c) in containers.iter().enumerate() {
        if c.starts_with("/work/") {
            assert!(i > work_index, "{c} is mounted on top of /work");
        }
    }
}

// SLICE-8-7
#[test]
fn test_role_mounts_everything_but_tests_read_only() {
    assert_mounts(Role::Test, false, |p| p.starts_with("tests/"));
}

// SLICE-8-7
#[test]
fn impl_role_mounts_everything_but_source_read_only() {
    assert_mounts(Role::Impl, false, |p| p.starts_with("src/"));
}

// SLICE-8-7
#[test]
fn read_only_mounts_follow_the_index() {
    assert_mounts(Role::Test, true, |p| p.starts_with("tests/"));
    assert_mounts(Role::Impl, true, |p| p.starts_with("src/"));

    // Spelled out: a staged new file outside the write set is mounted
    // read-only with its staged contents, a staged deletion is not mounted,
    // and a staged new file in the write set is writable.
    let fake = FakeContainers::reporting(vec![], green(&[]));
    let (root, repo) = repo("mounts-staged");
    stage_work(&repo);
    run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();
    let spec = fake.spec();
    let doc = spec
        .mounts
        .iter()
        .find(|m| m.container == "/work/docs/staged.md")
        .expect("the staged new docs/staged.md is mounted");
    assert!(doc.read_only);
    assert_eq!(read(&doc.host), "staged doc\n");
    let readme = mount_at(&spec, "/work/README.md").expect("README.md is mounted");
    assert_eq!(read(&readme), "# app, staged\n");
    assert_eq!(mount_at(&spec, "/work/Cargo.toml"), None, "a staged deletion");
    assert_eq!(mount_at(&spec, "/work/tests/staged.rs"), None, "in the write set");
}

// SLICE-8-7
#[test]
fn container_runs_claude_headless() {
    let fake = FakeContainers::reporting(vec![], green(&[]));
    run_on_repo("command", Role::Test, &fake);
    let spec = fake.spec();

    assert_eq!(spec.command.len(), 3, "{:?}", spec.command);
    assert_eq!(&spec.command[..2], &["sh".to_string(), "-c".to_string()]);
    let script = &spec.command[2];
    assert!(script.contains("claude"), "{script}");
    assert!(script.contains("-p ") || script.contains("--print"), "{script}");
    assert!(
        script.contains("/role/task.md") || script.contains(TASK.trim()),
        "the task is the prompt: {script}"
    );
    assert!(script.contains("--append-system-prompt"), "{script}");
    assert!(
        script.contains("/role/prompt.md") || script.contains("Write the tests."),
        "the role prompt is appended: {script}"
    );
    assert!(script.contains("--json-schema"), "{script}");
    assert!(
        script.contains("/role/report.schema.json") || script.contains("files_touched"),
        "the report schema: {script}"
    );
    assert!(script.contains("--model"), "{script}");
    assert!(
        script.contains("bypassPermissions") || script.contains("--dangerously-skip-permissions"),
        "{script}"
    );
    assert!(script.contains("--setting-sources"), "{script}");
    assert!(script.contains("--strict-mcp-config"), "{script}");
    assert!(script.contains("--no-session-persistence"), "{script}");
}

// SLICE-8-8
#[test]
fn report_cost_duration_and_turns_are_read() {
    let report = green(&["tests/new.rs"]);
    let fake = FakeContainers::reporting(
        vec![Edit::Write("tests/new.rs", "#[test]\nfn new() {}\n")],
        report.clone(),
    );
    let (_root, _repo, _request, result) = run_on_repo("report", Role::Test, &fake);

    let expected: Report = serde_json::from_value(report).unwrap();
    assert_eq!(result.report, Some(expected));
    assert_eq!(result.cost_usd, Some(COST));
    assert_eq!(result.duration_ms, Some(DURATION_MS));
    assert_eq!(result.turns, Some(TURNS));
}

// SLICE-8-8
#[test]
fn output_log_and_patch_are_kept_in_the_run_directory() {
    let edits = vec![
        Edit::Write("tests/a.rs", "#[test]\nfn a() {\n    assert!(true);\n}\n"),
        Edit::Write("tests/new/b.rs", "// brand new file\n"),
        Edit::Delete("tests/common/mod.rs"),
    ];
    let fake = FakeContainers::reporting(edits, green(&["tests/a.rs", "tests/new/b.rs"]));
    let (_root, _repo, _request, result) = run_on_repo("kept", Role::Test, &fake);

    assert_eq!(read(&result.run_dir.join("result.json")), fake.output.stdout);
    assert_eq!(read(&result.run_dir.join("agent.log")), fake.output.stderr);

    let patch = read(&result.run_dir.join("changes.patch"));
    assert!(patch.contains("assert!(true);"), "{patch}");
    assert!(
        patch.contains("// brand new file"),
        "new files are in the patch: {patch}"
    );
    assert!(
        patch.contains("pub fn helper() {}"),
        "deletions are in the patch: {patch}"
    );
    assert_eq!(
        sorted(patch_paths(&patch)),
        strings(&["tests/a.rs", "tests/common/mod.rs", "tests/new/b.rs"])
    );
}

// SLICE-8-8
#[test]
fn no_changes_means_an_empty_patch() {
    let fake = FakeContainers::reporting(vec![], green(&[]));
    let (_root, repo, _request, result) = run_on_repo("empty-patch", Role::Test, &fake);

    let patch = read(&result.run_dir.join("changes.patch"));
    assert_eq!(patch_paths(&patch), Vec::<String>::new(), "{patch}");
    assert_eq!(staged(&repo), Vec::<String>::new());
}

/// A failed run: outcome, no report, exit 1, the run directory and its
/// agent.log named, nothing staged, and the message containing `why`.
fn assert_failed(name: &str, output: ContainerOutput, why: &[&str]) {
    let edits = vec![Edit::Write("tests/new.rs", "// half done\n")];
    let fake = FakeContainers::new(edits, output);
    let (root, repo) = repo(name);
    let before = repo_state(&repo);

    let result = run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();

    assert_eq!(result.outcome, Outcome::Failed, "{name}");
    assert_eq!(result.report, None, "{name}");
    assert!(result.reason.is_some(), "{name}: why");
    assert_eq!(result.exit_code(), 1, "{name}");
    let message = result.message();
    assert!(message.to_lowercase().contains("fail"), "{name}: {message}");
    assert!(
        message.contains(&result.run_dir.display().to_string()),
        "{name}: {message}"
    );
    assert!(message.contains("agent.log"), "{name}: {message}");
    for w in why {
        assert!(
            message.to_lowercase().contains(&w.to_lowercase()),
            "{name}: says {w:?}: {message}"
        );
    }
    assert_eq!(repo_state(&repo), before, "{name}: nothing staged or changed");
}

// SLICE-8-9
#[test]
fn container_that_cannot_start_fails_the_run() {
    assert_failed(
        "no-start",
        ContainerOutput {
            exit: ContainerExit::FailedToStart("podman: command not found".into()),
            stdout: String::new(),
            stderr: String::new(),
        },
        &["podman: command not found"],
    );
}

// SLICE-8-9
#[test]
fn non_zero_exit_fails_the_run() {
    // Even with a valid green report in its output.
    assert_failed("exit-3", exited(3, &claude_output(green(&[]))), &["3"]);
    assert_failed("exit-1", exited(1, ""), &["1"]);
}

// SLICE-8-9
#[test]
fn non_zero_exit_keeps_the_agent_log() {
    let fake = FakeContainers::new(vec![], exited(2, "partial"));
    let (_root, _repo, _request, result) = run_on_repo("exit-log", Role::Impl, &fake);
    assert_eq!(result.outcome, Outcome::Failed);
    assert_eq!(read(&result.run_dir.join("agent.log")), fake.output.stderr);
}

// SLICE-8-9
#[test]
fn timeout_fails_the_run() {
    assert_failed(
        "timeout",
        ContainerOutput {
            exit: ContainerExit::TimedOut,
            stdout: String::new(),
            stderr: "still working\n".into(),
        },
        &["time"],
    );
}

// SLICE-8-9
#[test]
fn output_without_a_valid_report_fails_the_run() {
    let mut missing_field = green(&[]);
    missing_field.as_object_mut().unwrap().remove("questions");
    let mut bad_status = green(&[]);
    bad_status["status"] = json!("done");
    let mut no_report: Value = serde_json::from_str(&claude_output(green(&[]))).unwrap();
    no_report.as_object_mut().unwrap().remove("structured_output");
    let mut null_report = no_report.clone();
    null_report["structured_output"] = Value::Null;

    for (name, stdout) in [
        ("empty-output", String::new()),
        ("not-json", "Error: rate limited\n".to_string()),
        ("no-structured-output", no_report.to_string()),
        ("null-structured-output", null_report.to_string()),
        ("missing-field", claude_output(missing_field.clone())),
        ("bad-status", claude_output(bad_status.clone())),
        ("report-not-object", claude_output(json!("green"))),
    ] {
        assert_failed(name, exited(0, &stdout), &[]);
    }
}

/// The message of a result has each of `parts`.
fn assert_says(message: &str, parts: &[&str]) {
    for p in parts {
        assert!(message.contains(p), "says {p:?}: {message}");
    }
}

// SLICE-9-1
#[test]
fn green_patch_is_staged_not_committed() {
    let edits = vec![
        Edit::Write("tests/a.rs", "#[test]\nfn a() {\n    assert_eq!(1, 1);\n}\n"),
        Edit::Write("tests/new/b.rs", "// brand new file\n"),
        Edit::Delete("tests/common/mod.rs"),
    ];
    let report = green(&["tests/a.rs", "tests/new/b.rs", "tests/common/mod.rs"]);
    let fake = FakeContainers::reporting(edits, report);
    let (root, repo) = repo("staged");
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let refs = git(&repo, &["for-each-ref"]);

    let result = run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();

    assert_eq!(result.outcome, Outcome::Staged);
    assert_eq!(result.exit_code(), 0);
    assert_eq!(result.rejected_paths, Vec::<String>::new());
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head, "nothing committed");
    assert_eq!(git(&repo, &["for-each-ref"]), refs, "no branch moved");
    assert_eq!(
        staged(&repo),
        strings(&["A tests/new/b.rs", "D tests/common/mod.rs", "M tests/a.rs"])
    );
    assert_eq!(
        git(&repo, &["diff", "--name-only"]),
        "",
        "the working tree matches the index"
    );
    assert_eq!(
        read(&repo.join("tests/a.rs")),
        "#[test]\nfn a() {\n    assert_eq!(1, 1);\n}\n"
    );
    assert_eq!(read(&repo.join("tests/new/b.rs")), "// brand new file\n");
    assert!(!repo.join("tests/common/mod.rs").exists());
}

// SLICE-9-1
#[test]
fn green_impl_patch_is_staged() {
    let edits = vec![
        Edit::Write("src/lib.rs", "pub fn f() -> u32 {\n    42\n}\n"),
        Edit::Write("src/run.rs", "pub fn run() {}\n"),
    ];
    let fake = FakeContainers::reporting(edits, green(&["src/lib.rs", "src/run.rs"]));
    let (_root, repo, _request, result) = run_on_repo("staged-impl", Role::Impl, &fake);

    assert_eq!(result.outcome, Outcome::Staged);
    assert_eq!(result.exit_code(), 0);
    assert_eq!(staged(&repo), strings(&["A src/run.rs", "M src/lib.rs"]));
    assert_eq!(
        git(&repo, &["log", "--oneline"]).lines().count(),
        1,
        "nothing committed"
    );
}

// SLICE-9-1
#[test]
fn staged_message_reports_the_run() {
    let mut report = green(&["tests/new.rs", "tests/other.rs"]);
    report["assumptions"] = json!(["first assumption", "second assumption"]);
    report["summary"] = json!("Eleven tests for SLICE-99.");
    let edits = vec![
        Edit::Write("tests/new.rs", "// new\n"),
        Edit::Write("tests/other.rs", "// other\n"),
    ];
    let fake = FakeContainers::reporting(edits, report);
    let (_root, _repo, _request, result) = run_on_repo("staged-message", Role::Test, &fake);

    assert_eq!(result.outcome, Outcome::Staged);
    let message = result.message();
    assert_says(
        &message,
        &[
            "green",
            "Eleven tests for SLICE-99.",
            "first assumption",
            "second assumption",
            "tests/new.rs",
            "tests/other.rs",
            "0.13",
            "16.8 s",
            &result.run_dir.display().to_string(),
        ],
    );
}

// SLICE-9-1
#[test]
fn green_patch_is_staged_on_top_of_staged_work() {
    let (root, repo) = repo("staged-on-top");
    stage_work(&repo);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let refs = git(&repo, &["for-each-ref"]);
    let mut expected = index_files(&repo);
    let edits = vec![
        Edit::Write("tests/a.rs", "#[test]\nfn a() {\n    // staged, then the agent\n}\n"),
        Edit::Write("tests/staged.rs", "// staged test, then the agent\n"),
        Edit::Write("tests/new.rs", "// new from the agent\n"),
    ];
    let fake = FakeContainers::reporting(edits, green(&["tests/a.rs", "tests/staged.rs", "tests/new.rs"]));

    let result = run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();

    assert_eq!(result.outcome, Outcome::Staged);
    assert_eq!(result.exit_code(), 0);
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head, "nothing committed");
    assert_eq!(git(&repo, &["for-each-ref"]), refs, "no branch moved");
    for (path, content) in [
        ("tests/a.rs", "#[test]\nfn a() {\n    // staged, then the agent\n}\n"),
        ("tests/staged.rs", "// staged test, then the agent\n"),
        ("tests/new.rs", "// new from the agent\n"),
    ] {
        expected.insert(path.to_string(), content.as_bytes().to_vec());
    }
    assert_eq!(index_files(&repo), expected, "the patch on top of the staged work");
    assert_eq!(
        staged(&repo),
        strings(&[
            "A docs/staged.md",
            "A tests/new.rs",
            "A tests/staged.rs",
            "D Cargo.toml",
            "M README.md",
            "M tests/a.rs",
        ])
    );
    assert_eq!(
        git(&repo, &["diff", "--name-only"]),
        "",
        "the working tree matches the index"
    );
    assert_eq!(read(&repo.join("tests/new.rs")), "// new from the agent\n");
}

// SLICE-9-1
#[test]
fn message_names_what_the_run_started_from() {
    let (root, repo) = repo("started-from");
    stage_work(&repo);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let short = git(&repo, &["rev-parse", "--short", "HEAD"]);
    let fake = FakeContainers::reporting(vec![Edit::Write("tests/new.rs", "// new\n")], green(&["tests/new.rs"]));

    let result = run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();

    assert_eq!(result.outcome, Outcome::Staged);
    assert_eq!(result.head, head);
    assert_eq!(result.staged, strings(&STAGED_WORK));
    let message = result.message();
    assert_names_short_hash(&message, &short);
    assert_says(&message, &STAGED_WORK);
}

/// The message names `short` on its own, not as the start of a longer hash.
fn assert_names_short_hash(message: &str, short: &str) {
    let hex = |c: Option<char>| c.is_some_and(|c| c.is_ascii_hexdigit());
    let standalone = message
        .match_indices(short)
        .any(|(i, _)| !hex(message[..i].chars().next_back()) && !hex(message[i + short.len()..].chars().next()));
    assert!(standalone, "names {short:?} as the short hash: {message}");
}

// SLICE-9-1
#[test]
fn message_names_head_as_rev_parse_short_prints_it() {
    // A repository whose own config abbreviates hashes to 12 characters.
    let (root, repo) = repo("started-from-abbrev");
    git(&repo, &["config", "core.abbrev", "12"]);
    let short = git(&repo, &["rev-parse", "--short", "HEAD"]);
    assert_eq!(short.len(), 12);
    let fake = FakeContainers::reporting(vec![], green(&[]));

    let result = run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();

    assert_eq!(result.outcome, Outcome::Staged);
    assert_names_short_hash(&result.message(), &short);
}

// SLICE-9-1
#[test]
fn message_names_head_when_nothing_was_staged() {
    let fake = FakeContainers::reporting(vec![], green(&[]));
    let (_root, repo, _request, result) = run_on_repo("started-from-head", Role::Impl, &fake);

    assert_eq!(result.outcome, Outcome::Staged);
    assert_eq!(result.head, git(&repo, &["rev-parse", "HEAD"]));
    assert_eq!(result.staged, Vec::<String>::new());
    assert_names_short_hash(&result.message(), &git(&repo, &["rev-parse", "--short", "HEAD"]));
}

/// A run of `role` whose `edits` touch paths outside its write set: nothing
/// staged or changed, every such path listed, the patch kept, exit 1.
fn assert_rejected(name: &str, slice_toml: &str, role: Role, edits: Vec<Edit>, status: &str, outside: &[&str]) {
    let fake = FakeContainers::reporting(edits, report(status, &[]));
    let (root, repo) = repo_with(name, Some(slice_toml));
    let before = repo_state(&repo);

    let result = run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, role)).unwrap();

    assert_eq!(result.exit_code(), 1, "{name}");
    assert_eq!(repo_state(&repo), before, "{name}: nothing staged or changed");
    let message = result.message();
    for p in outside {
        assert!(message.contains(p), "{name}: lists {p}: {message}");
    }
    let patch = read(&result.run_dir.join("changes.patch"));
    for p in outside {
        assert!(
            patch_paths(&patch).iter().any(|x| x == p),
            "{name}: patch kept with {p}"
        );
    }
    assert_eq!(result.outcome, Outcome::Rejected, "{name}");
    assert_eq!(sorted(result.rejected_paths.clone()), strings(outside), "{name}");
}

// SLICE-9-2
#[test]
fn test_role_adding_a_source_file_is_rejected() {
    assert_rejected(
        "reject-add",
        SLICE_TOML,
        Role::Test,
        vec![
            Edit::Write("tests/ok.rs", "// fine\n"),
            Edit::Write("src/sneaky.rs", "pub fn sneaky() {}\n"),
        ],
        "green",
        &["src/sneaky.rs"],
    );
}

// SLICE-9-2
#[test]
fn changing_a_path_outside_the_write_set_is_rejected() {
    assert_rejected(
        "reject-change",
        SLICE_TOML,
        Role::Impl,
        vec![
            Edit::Write("src/lib.rs", "pub fn f() -> u32 {\n    2\n}\n"),
            Edit::Write("tests/a.rs", "#[test]\nfn a() { /* weakened */ }\n"),
            Edit::Write("Cargo.toml", "[package]\nname = \"changed\"\n"),
        ],
        "green",
        &["Cargo.toml", "tests/a.rs"],
    );
}

// SLICE-9-2
#[test]
fn deleting_a_path_outside_the_write_set_is_rejected() {
    assert_rejected(
        "reject-delete",
        SLICE_TOML,
        Role::Test,
        vec![Edit::Delete("README.md")],
        "green",
        &["README.md"],
    );
}

// SLICE-9-2
#[test]
fn renaming_out_of_the_write_set_is_rejected() {
    assert_rejected(
        "reject-rename-out",
        SLICE_TOML,
        Role::Impl,
        vec![Edit::Rename("src/lib.rs", "lib.rs")],
        "green",
        &["lib.rs"],
    );
    assert_rejected(
        "reject-rename-in",
        SLICE_TOML,
        Role::Test,
        vec![Edit::Rename("src/lib.rs", "tests/lib.rs")],
        "green",
        &["src/lib.rs"],
    );
}

// SLICE-9-2
#[test]
fn impl_writing_a_test_file_is_rejected_where_source_globs_cover_it() {
    let toml = "[paths]\ntest = [\"**/*_test.go\"]\nsource = [\"**/*.go\"]\n";
    assert_rejected(
        "reject-go",
        toml,
        Role::Impl,
        vec![
            Edit::Write("pkg/b.go", "package pkg\n"),
            Edit::Write("pkg/b_test.go", "package pkg\n"),
        ],
        "green",
        &["pkg/b_test.go"],
    );
}

// SLICE-9-2
#[test]
fn outside_paths_are_rejected_whatever_the_status() {
    assert_rejected(
        "reject-blocked",
        SLICE_TOML,
        Role::Test,
        vec![Edit::Write("src/sneaky.rs", "pub fn sneaky() {}\n")],
        "blocked",
        &["src/sneaky.rs"],
    );
}

// SLICE-9-2
#[test]
fn patch_paths_lists_every_path_a_diff_touches() {
    let patch = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
 pub fn f() -> u32 {
-    1
+    2
 }
diff --git a/tests/new.rs b/tests/new.rs
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ b/tests/new.rs
@@ -0,0 +1 @@
+// new
diff --git a/README.md b/README.md
deleted file mode 100644
index 4444444..0000000
--- a/README.md
+++ /dev/null
@@ -1 +0,0 @@
-# app
diff --git a/src/old.rs b/lib/moved.rs
similarity index 100%
rename from src/old.rs
rename to lib/moved.rs
diff --git a/run.sh b/run.sh
old mode 100644
new mode 100755
";
    assert_eq!(
        sorted(patch_paths(patch)),
        strings(&[
            "README.md",
            "lib/moved.rs",
            "run.sh",
            "src/lib.rs",
            "src/old.rs",
            "tests/new.rs"
        ])
    );
    assert_eq!(patch_paths(""), Vec::<String>::new());
}

// SLICE-9-3
#[test]
fn blocked_report_stages_nothing_and_asks() {
    let edits = vec![Edit::Write("tests/partial.rs", "// partial tests\n")];
    let mut report = blocked(&["tests/partial.rs"]);
    report["questions"] = json!(["Is a byte a character?", "Which clock?"]);
    report["assumptions"] = json!(["The stub is final"]);
    report["summary"] = json!("Blocked on SLICE-99-3.");
    let fake = FakeContainers::reporting(edits, report.clone());
    let (root, repo) = repo("blocked");
    let before = repo_state(&repo);

    let result = run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();

    assert_eq!(result.outcome, Outcome::Blocked);
    assert_eq!(result.exit_code(), 1);
    assert_eq!(result.report, Some(serde_json::from_value(report).unwrap()));
    assert_eq!(repo_state(&repo), before, "nothing staged");
    assert_says(
        &result.message(),
        &[
            "Is a byte a character?",
            "Which clock?",
            "The stub is final",
            "Blocked on SLICE-99-3.",
        ],
    );
    let patch = read(&result.run_dir.join("changes.patch"));
    assert!(patch.contains("// partial tests"), "the patch is kept: {patch}");
}

/// A green run in the write set whose repository `during` changes while the
/// agent runs: nothing of the patch staged, why said, patch kept, exit 1.
fn assert_moved(name: &str, during: impl Fn(&Path) + 'static, prepare: impl FnOnce(&Path)) -> (PathBuf, TempDir) {
    let (root, repo) = repo(name);
    prepare(&repo);
    let edits = vec![Edit::Write("tests/new.rs", "// from the agent\n")];
    let mut fake = FakeContainers::reporting(edits, green(&["tests/new.rs"]));
    let during_repo = repo.clone();
    fake.during = Some(Box::new(move || during(&during_repo)));

    let result = run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();

    assert_eq!(result.outcome, Outcome::Moved, "{name}");
    assert_eq!(result.exit_code(), 1, "{name}");
    assert!(result.reason.is_some(), "{name}: why");
    assert!(!result.message().trim().is_empty(), "{name}");
    assert!(
        !staged(&repo).iter().any(|l| l.ends_with(" tests/new.rs")),
        "{name}: nothing of the patch staged"
    );
    let patch = read(&result.run_dir.join("changes.patch"));
    assert!(patch.contains("// from the agent"), "{name}: patch kept: {patch}");
    (repo, root)
}

// SLICE-9-4
#[test]
fn head_moved_during_the_run_stages_nothing() {
    let (repo, _root) = assert_moved(
        "moved-head",
        |repo| {
            write(repo, "docs/later.md", "committed during the run\n");
            git(repo, &["add", "docs/later.md"]);
            git(repo, &["commit", "-q", "-m", "meanwhile"]);
        },
        |_| {},
    );
    assert_eq!(staged(&repo), Vec::<String>::new());
    assert!(!repo.join("tests/new.rs").exists());
    assert_eq!(git(&repo, &["log", "--oneline"]).lines().count(), 2);
}

// SLICE-9-4
#[test]
fn tracked_file_changed_during_the_run_stages_nothing() {
    let (repo, _root) = assert_moved(
        "moved-file",
        |repo| write(repo, "README.md", "# app, edited meanwhile\n"),
        |_| {},
    );
    assert_eq!(staged(&repo), Vec::<String>::new());
    assert!(!repo.join("tests/new.rs").exists());
    assert_eq!(read(&repo.join("README.md")), "# app, edited meanwhile\n");
}

// SLICE-9-4
#[test]
fn patch_that_does_not_apply_stages_nothing() {
    // An untracked file in the repository where the patch adds one.
    let (repo, _root) = assert_moved(
        "moved-conflict",
        |_| {},
        |repo| write(repo, "tests/new.rs", "// someone else's untracked file\n"),
    );
    assert_eq!(staged(&repo), Vec::<String>::new());
    assert_eq!(read(&repo.join("tests/new.rs")), "// someone else's untracked file\n");
    assert_eq!(
        git(&repo, &["status", "--porcelain", "--untracked-files=all"]),
        "?? tests/new.rs"
    );
}

// SLICE-9-4
#[test]
fn index_changed_during_the_run_stages_nothing() {
    // More is staged while the agent runs, on top of staged work.
    let (repo, _root) = assert_moved(
        "moved-index-added",
        |repo| {
            write(repo, "docs/later.md", "staged during the run\n");
            git(repo, &["add", "docs/later.md"]);
        },
        stage_work,
    );
    let mut expected: Vec<String> = STAGED_WORK.iter().map(|p| p.to_string()).collect();
    expected.push("docs/later.md".into());
    assert_eq!(staged_paths(&repo), sorted(expected));
    assert!(!repo.join("tests/new.rs").exists());

    // A staged change is unstaged while the agent runs.
    let (repo, _root) = assert_moved(
        "moved-index-reset",
        |repo| {
            git(repo, &["reset", "-q", "--", "README.md"]);
        },
        stage_work,
    );
    let expected: Vec<String> = STAGED_WORK
        .iter()
        .filter(|p| **p != "README.md")
        .map(|p| p.to_string())
        .collect();
    assert_eq!(staged_paths(&repo), expected);
    assert_eq!(read(&repo.join("README.md")), "# app, staged\n");
    assert!(!repo.join("tests/new.rs").exists());
}

// SLICE-9-4
#[test]
fn repository_moved_under_staged_work_stages_nothing() {
    // HEAD moved: the staged work committed while the agent runs.
    let (repo, _root) = assert_moved(
        "moved-head-staged",
        |repo| {
            git(repo, &["commit", "-q", "-m", "staged work committed meanwhile"]);
        },
        stage_work,
    );
    assert_eq!(staged(&repo), Vec::<String>::new());
    assert!(!repo.join("tests/new.rs").exists());

    // A tracked file changed in the working tree.
    let (repo, _root) = assert_moved(
        "moved-file-staged",
        |repo| write(repo, "src/lib.rs", "// edited meanwhile\n"),
        stage_work,
    );
    assert_eq!(staged_paths(&repo), strings(&STAGED_WORK));
    assert_eq!(read(&repo.join("src/lib.rs")), "// edited meanwhile\n");
    assert!(!repo.join("tests/new.rs").exists());
}

fn json_of(result: &RunResult) -> Value {
    let value = result.to_json();
    assert!(value.is_object(), "{value}");
    value
}

// SLICE-9-5
#[test]
fn json_for_a_staged_run() {
    let report = green(&["tests/new.rs"]);
    let fake = FakeContainers::reporting(vec![Edit::Write("tests/new.rs", "// new\n")], report.clone());
    let (_root, repo, _request, result) = run_on_repo("json-staged", Role::Test, &fake);

    let value = json_of(&result);
    assert_eq!(value["outcome"], "staged");
    assert_eq!(value["run_dir"], json!(result.run_dir.display().to_string()));
    assert_eq!(value["report"], report);
    assert_eq!(value["rejected_paths"], json!([]));
    assert_eq!(value["cost_usd"], json!(COST));
    assert_eq!(value["duration_ms"], json!(DURATION_MS));
    assert_eq!(value["turns"], json!(TURNS));
    assert_eq!(value["head"], json!(git(&repo, &["rev-parse", "HEAD"])));
    assert_eq!(value["staged"], json!([]));
}

// SLICE-9-5
#[test]
fn json_for_a_rejected_run() {
    let edits = vec![
        Edit::Write("src/sneaky.rs", "// sneaky\n"),
        Edit::Write("README.md", "# changed\n"),
    ];
    let report = green(&["src/sneaky.rs"]);
    let fake = FakeContainers::reporting(edits, report.clone());
    let (_root, _repo, _request, result) = run_on_repo("json-rejected", Role::Test, &fake);

    let value = json_of(&result);
    assert_eq!(value["outcome"], "rejected");
    assert_eq!(value["report"], report);
    let mut rejected: Vec<String> = serde_json::from_value(value["rejected_paths"].clone()).unwrap();
    rejected.sort();
    assert_eq!(rejected, strings(&["README.md", "src/sneaky.rs"]));
    assert_eq!(value["cost_usd"], json!(COST));
    assert_eq!(result.exit_code(), 1);
}

// SLICE-9-5
#[test]
fn json_for_a_blocked_run() {
    let report = blocked(&[]);
    let fake = FakeContainers::reporting(vec![], report.clone());
    let (_root, _repo, _request, result) = run_on_repo("json-blocked", Role::Impl, &fake);

    let value = json_of(&result);
    assert_eq!(value["outcome"], "blocked");
    assert_eq!(value["report"], report);
    assert_eq!(value["rejected_paths"], json!([]));
    assert_eq!(value["turns"], json!(TURNS));
    assert_eq!(result.exit_code(), 1);
}

// SLICE-9-5
#[test]
fn json_for_a_moved_run() {
    let (root, repo) = repo("json-moved");
    let mut fake = FakeContainers::reporting(vec![Edit::Write("tests/x.rs", "// x\n")], green(&[]));
    let during_repo = repo.clone();
    fake.during = Some(Box::new(move || write(&during_repo, "README.md", "# meanwhile\n")));

    let result = run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();

    let value = json_of(&result);
    assert_eq!(value["outcome"], "moved");
    assert_eq!(value["report"], green(&[]));
    assert_eq!(value["rejected_paths"], json!([]));
    assert_eq!(result.exit_code(), 1);
}

// SLICE-9-5
#[test]
fn json_for_a_failed_run() {
    let fake = FakeContainers::new(
        vec![],
        ContainerOutput {
            exit: ContainerExit::TimedOut,
            stdout: String::new(),
            stderr: String::new(),
        },
    );
    let (_root, repo, _request, result) = run_on_repo("json-failed", Role::Test, &fake);

    let value = json_of(&result);
    assert_eq!(value["outcome"], "failed");
    assert_eq!(value["run_dir"], json!(result.run_dir.display().to_string()));
    assert_eq!(value["report"], Value::Null);
    assert_eq!(value["rejected_paths"], json!([]));
    assert_eq!(value["cost_usd"], Value::Null);
    assert_eq!(value["duration_ms"], Value::Null);
    assert_eq!(value["turns"], Value::Null);
    assert_eq!(value["head"], json!(git(&repo, &["rev-parse", "HEAD"])));
    assert_eq!(value["staged"], json!([]));
    assert_eq!(result.exit_code(), 1);
}

// SLICE-9-5
#[test]
fn json_has_null_for_what_the_output_does_not_say() {
    let mut output: Value = serde_json::from_str(&claude_output(green(&[]))).unwrap();
    let object = output.as_object_mut().unwrap();
    object.remove("total_cost_usd");
    object.remove("duration_ms");
    object.remove("num_turns");
    let fake = FakeContainers::new(vec![], exited(0, &output.to_string()));
    let (_root, _repo, _request, result) = run_on_repo("json-unknown", Role::Test, &fake);

    assert_eq!(result.outcome, Outcome::Staged);
    let value = json_of(&result);
    assert_eq!(value["outcome"], "staged");
    assert_eq!(value["cost_usd"], Value::Null);
    assert_eq!(value["duration_ms"], Value::Null);
    assert_eq!(value["turns"], Value::Null);
    assert_eq!(result.exit_code(), 0);
}

// SLICE-9-5
#[test]
fn json_names_head_and_the_staged_paths() {
    // Staged in an order other than sorted; the run is blocked, and still
    // says what it started from.
    let (root, repo) = repo("json-start");
    write(&repo, "tests/z.rs", "// z\n");
    git(&repo, &["add", "tests/z.rs"]);
    stage_work(&repo);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let fake = FakeContainers::reporting(vec![], blocked(&[]));

    let result = run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();

    assert_eq!(result.outcome, Outcome::Blocked);
    let mut expected = strings(&STAGED_WORK);
    expected.push("tests/z.rs".into());
    let value = json_of(&result);
    assert_eq!(value["outcome"], "blocked");
    assert_eq!(value["head"], json!(head));
    assert_eq!(value["staged"], json!(sorted(expected)));
}

// SLICE-9-5
#[test]
fn json_staged_lists_every_path_whose_index_entry_differs_from_head() {
    // A staged rename, a staged deletion, a staged mode change, and a change
    // staged and then staged back to HEAD's contents.
    use std::os::unix::fs::PermissionsExt;
    let (root, repo) = repo("json-staged-entries");
    git(&repo, &["mv", "tests/common/mod.rs", "tests/common/helpers.rs"]);
    git(&repo, &["rm", "-q", "Cargo.toml"]);
    std::fs::set_permissions(repo.join("README.md"), std::fs::Permissions::from_mode(0o755)).unwrap();
    git(&repo, &["add", "README.md"]);
    write(&repo, "src/lib.rs", "// changed, then changed back\n");
    git(&repo, &["add", "src/lib.rs"]);
    write(&repo, "src/lib.rs", "pub fn f() -> u32 {\n    1\n}\n");
    git(&repo, &["add", "src/lib.rs"]);
    assert_eq!(
        git(&repo, &["status", "--porcelain", "--untracked-files=no", "--", "src"]),
        ""
    );
    let fake = FakeContainers::reporting(vec![], green(&[]));

    let result = run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();

    let value = json_of(&result);
    assert_eq!(
        value["staged"],
        json!([
            "Cargo.toml",
            "README.md",
            "tests/common/helpers.rs",
            "tests/common/mod.rs"
        ])
    );
}

// SLICE-9-5
#[test]
fn json_names_head_and_the_staged_paths_whatever_the_outcome() {
    let failing = |exit: ContainerExit, stdout: &str| {
        FakeContainers::new(
            vec![],
            ContainerOutput {
                exit,
                stdout: stdout.to_string(),
                stderr: String::new(),
            },
        )
    };
    let cases: Vec<(&str, FakeContainers, &str)> = vec![
        (
            "staged",
            FakeContainers::reporting(vec![Edit::Write("tests/new.rs", "// new\n")], green(&["tests/new.rs"])),
            "staged",
        ),
        (
            "rejected",
            FakeContainers::reporting(vec![Edit::Write("src/sneaky.rs", "// sneaky\n")], green(&[])),
            "rejected",
        ),
        ("blocked", FakeContainers::reporting(vec![], blocked(&[])), "blocked"),
        ("moved", FakeContainers::reporting(vec![], green(&[])), "moved"),
        (
            "no-start",
            failing(ContainerExit::FailedToStart("no such image".into()), ""),
            "failed",
        ),
        (
            "non-zero",
            failing(ContainerExit::Exited(1), &claude_output(green(&[]))),
            "failed",
        ),
        ("timed-out", failing(ContainerExit::TimedOut, ""), "failed"),
        ("no-report", failing(ContainerExit::Exited(0), "not json"), "failed"),
    ];
    for (name, mut fake, outcome) in cases {
        let (root, repo) = repo(&format!("json-start-{name}"));
        stage_work(&repo);
        let head = git(&repo, &["rev-parse", "HEAD"]);
        if name == "moved" {
            let during_repo = repo.clone();
            fake.during = Some(Box::new(move || write(&during_repo, "README.md", "# meanwhile\n")));
        }

        let result = run(&fake, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();

        let value = json_of(&result);
        assert_eq!(value["outcome"], outcome, "{name}: {value}");
        assert_eq!(value["head"], json!(head), "{name}: {value}");
        assert_eq!(value["staged"], json!(STAGED_WORK), "{name}: {value}");
    }
}

// SLICE-9-6
#[test]
fn second_run_builds_on_the_first_runs_staged_patch() {
    let (root, repo) = repo("run-again");
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let first = FakeContainers::reporting(
        vec![
            Edit::Write("tests/a.rs", "#[test]\nfn a() {\n    // first run\n}\n"),
            Edit::Write("tests/new.rs", "// first run\n"),
            Edit::Delete("tests/common/mod.rs"),
        ],
        green(&["tests/a.rs", "tests/new.rs", "tests/common/mod.rs"]),
    );
    let one = run(&first, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();
    assert_eq!(one.outcome, Outcome::Staged);

    // The spec changes, and the change is staged.
    write(&repo, ".spool/app.req", "feature: APP\n");
    git(&repo, &["add", ".spool/app.req"]);
    let second = FakeContainers::reporting(
        vec![
            Edit::Write("tests/new.rs", "// first run\n// second run\n"),
            Edit::Write("tests/second.rs", "// second run\n"),
        ],
        green(&["tests/new.rs", "tests/second.rs"]),
    );
    let two = run(&second, Ok(agent_settings(&root)), &request(&root, &repo, Role::Test)).unwrap();

    assert_eq!(two.outcome, Outcome::Staged);
    assert_eq!(two.exit_code(), 0);
    assert_eq!(two.head, head);
    assert_eq!(
        two.staged,
        strings(&[".spool/app.req", "tests/a.rs", "tests/common/mod.rs", "tests/new.rs"])
    );

    // The second agent saw the first run's changes and the staged spec.
    let seen = second.seen().workspace;
    assert_eq!(
        seen.get("tests/a.rs").unwrap(),
        b"#[test]\nfn a() {\n    // first run\n}\n"
    );
    assert_eq!(seen.get("tests/new.rs").unwrap(), b"// first run\n");
    assert!(!seen.contains_key("tests/common/mod.rs"));
    assert_eq!(seen.get(".spool/app.req").unwrap(), b"feature: APP\n");

    // Its patch holds only what it changed on top of them.
    let patch = read(&two.run_dir.join("changes.patch"));
    assert_eq!(
        sorted(patch_paths(&patch)),
        strings(&["tests/new.rs", "tests/second.rs"]),
        "{patch}"
    );
    assert!(patch.contains("+// second run"), "{patch}");
    assert!(!patch.contains("+// first run"), "{patch}");

    // The index holds both, as if staged by hand.
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head, "nothing committed");
    assert_eq!(
        staged(&repo),
        strings(&[
            "A .spool/app.req",
            "A tests/new.rs",
            "A tests/second.rs",
            "D tests/common/mod.rs",
            "M tests/a.rs",
        ])
    );
    let index = index_files(&repo);
    assert_eq!(index.get("tests/new.rs").unwrap(), b"// first run\n// second run\n");
    assert_eq!(index.get("tests/second.rs").unwrap(), b"// second run\n");
    assert_eq!(
        index.get("tests/a.rs").unwrap(),
        b"#[test]\nfn a() {\n    // first run\n}\n"
    );
    assert_eq!(
        git(&repo, &["diff", "--name-only"]),
        "",
        "the working tree matches the index"
    );
}
