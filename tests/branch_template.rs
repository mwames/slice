//! Branch templates (SLICE-11): which template a repository's config entry
//! gives it, which templates are refused, the names a template writes and
//! recognizes, and slice open naming and finding branches with them.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use proptest::prelude::*;
use slice::config::{Config, home};
use slice::ports::*;

/// The template SLICE-11-2 and SLICE-11-3 configure: the ticket followed by
/// an underscore.
const UNDERSCORE: &str = "{type}/{date}-{ticket}_{title}";

fn template(text: &str) -> BranchTemplate {
    BranchTemplate::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn ticket(r: &str, title: &str) -> Ticket {
    Ticket {
        reference: tref(r),
        system: system_of(&tref(r)),
        title: title.into(),
        status: String::new(),
    }
}

fn parts(branch_type: Option<&str>, date: Option<&str>, ticket: &str, short_title: &str) -> BranchName {
    BranchName {
        branch_type: branch_type.map(|t| BranchType(t.into())),
        date: date.map(String::from),
        ticket: tref(ticket),
        short_title: short_title.into(),
    }
}

/// The parts of a new branch for `ticket`, titled `title`, made on `date`.
fn new_name(branch_type: Option<&str>, date: &str, r: &str, title: &str) -> BranchName {
    let branch_type = branch_type.map(|t| BranchType(t.into()));
    BranchName::new(branch_type.as_ref(), date, &ticket(r, title))
}

fn settings_with(entries: &[(&str, &str)]) -> Settings {
    Settings {
        branch_templates: entries
            .iter()
            .map(|(path, text)| (PathBuf::from(path), template(text)))
            .collect(),
        ..settings()
    }
}

fn request(ticket: &str, branch_type: Option<&str>) -> OpenRequest {
    request_in(REPO, ticket, branch_type)
}

fn request_in(repo: &str, ticket: &str, branch_type: Option<&str>) -> OpenRequest {
    OpenRequest {
        ticket: tref(ticket),
        repo: PathBuf::from(repo),
        branch_type: branch_type.map(|t| BranchType(t.to_string())),
        base: None,
    }
}

/// A completed open of T in `REPO` as SLICE-2-1 has it, on `branch` at
/// `base`: T's timer running alone, T in progress, the branch checked out in
/// its worktree, and the manifest and active slice recording it.
fn assert_opened(fake: &Fake, outcome: OpenOutcome, branch: &str, base: &str) -> SliceRepo {
    let OpenOutcome::Opened(slice) = outcome else {
        panic!("want Opened, got {outcome:?}")
    };
    let w = fake.snapshot();
    let running = w.running();
    assert_eq!(running.len(), 1, "exactly one running timer");
    assert_eq!(running[0].ticket, Some(tref(T)));
    assert_eq!(w.tickets[T].status, "development");
    let [repo] = slice.repos.as_slice() else {
        panic!("want one repository, got {:?}", slice.repos)
    };
    assert_eq!(repo.repo, PathBuf::from(REPO));
    assert_eq!(repo.branch, branch);
    assert_eq!(repo.base_commit, base);
    assert!(w.branches.contains_key(branch), "branch {branch} exists");
    assert_eq!(w.worktrees.get(branch), Some(&repo.worktree), "worktree checked out");
    let manifest = w.manifests.get(&slice.id).expect("manifest written");
    assert_eq!(manifest.repos, slice.repos);
    assert_eq!(w.active.as_ref(), Some(&slice));
    repo.clone()
}

/// `repo` added to the active slice on `branch`, created at `base` and
/// checked out in its worktree.
fn assert_added(fake: &Fake, outcome: OpenOutcome, repo: &str, branch: &str, base: &str) {
    let OpenOutcome::Added(slice) = outcome else {
        panic!("want Added, got {outcome:?}")
    };
    let added = slice.repos.last().unwrap();
    assert_eq!(added.repo, PathBuf::from(repo));
    assert_eq!(added.branch, branch);
    assert_eq!(added.base_commit, base);
    assert_eq!(added.worktree, worktree_in(repo, branch));
    let w = fake.snapshot();
    let r = &w.repos[Path::new(repo)];
    assert_eq!(
        r.branches.get(branch).map(String::as_str),
        Some(base),
        "{branch} created"
    );
    assert_eq!(r.worktrees.get(branch), Some(&added.worktree), "worktree checked out");
    assert_eq!(w.active.as_ref(), Some(&slice));
    assert_eq!(w.manifests[&slice.id].repos, slice.repos);
}

/// A refusal: the error, a non-zero exit, and nothing changed.
fn assert_refused(fake: &Fake, before: &World, err: &SliceError, want: &SliceError) {
    assert_eq!(err, want);
    assert_ne!(err.exit_code(), 0);
    assert_eq!(&fake.snapshot(), before, "nothing changed");
    assert_eq!(fake.writes(), Vec::<String>::new());
}

// ---------------------------------------------------------------------------
// SLICE-11-1: default template
// ---------------------------------------------------------------------------

// SLICE-11-1
#[test]
fn default_template() {
    assert_eq!(BranchTemplate::DEFAULT, "{type}/{date}-{ticket}-{title}");
    assert_eq!(BranchTemplate::default(), template(BranchTemplate::DEFAULT));
}

// SLICE-11-1
#[test]
fn repository_without_entry_uses_the_default() {
    assert_eq!(settings().template_for(Path::new(REPO)), BranchTemplate::default());

    let s = settings_with(&[("/work/api", "{ticket}_{title}"), ("/elsewhere/*", "{ticket}_{title}")]);
    assert_eq!(s.template_for(Path::new(REPO)), BranchTemplate::default());
}

// SLICE-11-1
#[test]
fn config_without_repos_has_no_templates() {
    let s = Config::parse("").unwrap().settings().unwrap();
    assert_eq!(s.branch_templates, Vec::new());
    assert_eq!(s.template_for(Path::new(REPO)), BranchTemplate::default());
}

// SLICE-11-1
#[test]
fn open_without_entry_names_branch_as_before() {
    let mut fake = Fake::new();
    fake.with_template(API, "{ticket}_{title}");

    let outcome = open(&fake.deps(), &request(T, Some("feature"))).expect("open");

    let repo = assert_opened(&fake, outcome, T_BRANCH_NEW, BASE_COMMIT);
    assert_eq!(repo.worktree, worktree_for(T_BRANCH_NEW));
}

// SLICE-11-1
#[test]
fn open_without_entry_finds_branch_as_before() {
    let mut fake = Fake::new();
    fake.with_template(API, "{ticket}_{title}");
    fake.with_branch(T_BRANCH_OLD, T_BRANCH_OLD_HEAD, true);
    fake.with_branch("ENG-1_login", "0under0", false);

    let outcome = open(&fake.deps(), &request(T, None)).expect("open");

    assert_opened(&fake, outcome, T_BRANCH_OLD, T_BRANCH_OLD_HEAD);
    assert!(
        !fake.write_ops().contains(&"create_branch".to_string()),
        "no branch created: {:?}",
        fake.writes()
    );
}

// ---------------------------------------------------------------------------
// SLICE-11-2: configured template names a new branch
// ---------------------------------------------------------------------------

// SLICE-11-2
#[test]
fn configured_template_renders_its_literal_text() {
    let name = new_name(Some("feature"), "20261112", T, "Login");
    assert_eq!(template(UNDERSCORE).render(&name), "feature/20261112-ENG-1_login");
    let name = new_name(Some("bug"), "20261112", "TBOARD-405", "Dashboard: totals (v2)");
    assert_eq!(
        template(UNDERSCORE).render(&name),
        "bug/20261112-TBOARD-405_dashboard-totals-v2"
    );
}

// SLICE-11-2
#[test]
fn configured_template_names_new_branch() {
    let mut fake = Fake::new();
    fake.with_template(REPO, UNDERSCORE);
    let deps = Deps {
        today: "20261112",
        ..fake.deps()
    };

    let outcome = open(&deps, &request(T, Some("feature"))).expect("open");

    let branch = "feature/20261112-ENG-1_login";
    let repo = assert_opened(&fake, outcome, branch, BASE_COMMIT);
    assert_eq!(
        repo.worktree,
        PathBuf::from("/work/app.slices/feature-20261112-ENG-1_login")
    );
    assert!(
        fake.writes().contains(&format!(
            "create_branch {branch} {} {BASE_COMMIT}",
            repo.worktree.display()
        )),
        "branch created: {:?}",
        fake.writes()
    );
}

// ---------------------------------------------------------------------------
// SLICE-11-3: configured template finds the ticket's branch
// ---------------------------------------------------------------------------

// SLICE-11-3
#[test]
fn configured_template_recognizes_its_branches() {
    let t = template(UNDERSCORE);
    assert_eq!(
        t.recognize("feature/20261101-ENG-1_login"),
        Some(parts(Some("feature"), Some("20261101"), T, "login"))
    );
    assert_eq!(
        t.recognize("bug/20261020-TBOARD-405_login-crash"),
        Some(parts(Some("bug"), Some("20261020"), "TBOARD-405", "login-crash"))
    );
    assert_eq!(
        t.recognize("feature/20261101-ENG-12_other").map(|n| n.ticket),
        Some(tref("ENG-12"))
    );
}

// SLICE-11-3
#[test]
fn configured_template_does_not_recognize_default_names() {
    let t = template(UNDERSCORE);
    for n in [
        "feature/20261101-ENG-1-login",
        "feature/20261101-ENG-1-login_x",
        "main",
        "feature/20261101_ENG-1_login",
    ] {
        assert_eq!(t.recognize(n), None, "{n}");
    }
    assert_eq!(
        BranchTemplate::default().recognize("feature/20261101-ENG-1_login"),
        None,
        "nor the default the configured template's"
    );
}

// SLICE-11-3
#[test]
fn configured_template_finds_branch() {
    for with_worktree in [true, false] {
        let mut fake = Fake::new();
        fake.with_template(REPO, UNDERSCORE);
        let branch = "bug/20261101-ENG-1_login";
        fake.with_branch(branch, T_BRANCH_OLD_HEAD, with_worktree);
        fake.with_branch("feature/20261101-ENG-12_other", "0other0", false);

        let outcome = open(&fake.deps(), &request(T, None)).expect("open");

        let repo = assert_opened(&fake, outcome, branch, T_BRANCH_OLD_HEAD);
        assert_eq!(repo.worktree, worktree_for(branch), "with worktree: {with_worktree}");
        assert!(
            !fake.write_ops().contains(&"create_branch".to_string()),
            "no branch created: {:?}",
            fake.writes()
        );
    }
}

// SLICE-11-3
#[test]
fn configured_template_finds_branch_with_its_own_type() {
    let mut fake = Fake::new();
    fake.with_template(REPO, UNDERSCORE);
    let branch = "bug/20261101-ENG-1_login";
    fake.with_branch(branch, T_BRANCH_OLD_HEAD, true);

    let outcome = open(&fake.deps(), &request(T, Some("bug"))).expect("open");

    assert_opened(&fake, outcome, branch, T_BRANCH_OLD_HEAD);
}

// SLICE-11-3
#[test]
fn default_named_branch_is_not_the_tickets_under_a_configured_template() {
    let mut fake = Fake::new();
    fake.with_template(REPO, UNDERSCORE);
    fake.with_branch(T_BRANCH_OLD, T_BRANCH_OLD_HEAD, true);

    let outcome = open(&fake.deps(), &request(T, Some("feature"))).expect("open");

    let branch = "feature/20261102-ENG-1_login";
    assert_opened(&fake, outcome, branch, BASE_COMMIT);
    assert_eq!(
        fake.snapshot().branches.get(T_BRANCH_OLD).map(String::as_str),
        Some(T_BRANCH_OLD_HEAD),
        "the default-named branch is left as it was"
    );
}

// SLICE-11-3
#[test]
fn default_named_branch_does_not_spare_the_type() {
    let mut fake = Fake::new();
    fake.with_template(REPO, UNDERSCORE);
    fake.with_branch(T_BRANCH_OLD, T_BRANCH_OLD_HEAD, true);
    let before = fake.snapshot();

    let err = open(&fake.deps(), &request(T, None)).expect_err("open must fail");

    assert_refused(&fake, &before, &err, &SliceError::TypeRequired);
}

// SLICE-11-3
#[test]
fn configured_template_branch_options_conflict() {
    let branch = "feature/20261101-ENG-1_login";
    let mut with_base = request(T, None);
    with_base.base = Some("main".into());
    for req in [with_base, request(T, Some("bug"))] {
        let mut fake = Fake::new();
        fake.with_template(REPO, UNDERSCORE);
        fake.with_branch(branch, T_BRANCH_OLD_HEAD, true);
        let before = fake.snapshot();

        let err = open(&fake.deps(), &req).expect_err("open must fail");

        assert_refused(
            &fake,
            &before,
            &err,
            &SliceError::BranchOptionsConflict { branch: branch.into() },
        );
    }
}

// SLICE-11-3
#[test]
fn configured_template_ambiguous_branch() {
    let mut fake = Fake::new();
    fake.with_template(REPO, UNDERSCORE);
    fake.with_branch("feature/20261101-ENG-1_login", T_BRANCH_OLD_HEAD, true);
    fake.with_branch("bug/20261020-ENG-1_login-crash", "0bug000", false);
    fake.with_branch(T_BRANCH_OLD, "0dflt00", false);
    let before = fake.snapshot();

    let err = open(&fake.deps(), &request(T, None)).expect_err("open must fail");

    let SliceError::AmbiguousBranch(mut branches) = err.clone() else {
        panic!("want AmbiguousBranch, got {err:?}")
    };
    branches.sort();
    assert_eq!(
        branches,
        ["bug/20261020-ENG-1_login-crash", "feature/20261101-ENG-1_login"],
        "every branch the template recognizes, and only those"
    );
    assert_ne!(err.exit_code(), 0);
    assert_eq!(fake.snapshot(), before, "nothing changed");
}

// ---------------------------------------------------------------------------
// SLICE-11-4: placeholders
// ---------------------------------------------------------------------------

// SLICE-11-4
#[test]
fn date_parts_render() {
    let t = template("{yyyy}/{mm}/{dd}/{ticket}_{title}");
    let name = new_name(None, "20261102", T, "Login");
    assert_eq!(t.render(&name), "2026/11/02/ENG-1_login");
}

// SLICE-11-4
#[test]
fn open_names_branch_from_date_parts() {
    let mut fake = Fake::new();
    fake.with_template(REPO, "{yyyy}/{mm}/{dd}/{ticket}_{title}");

    let outcome = open(&fake.deps(), &request(T, None)).expect("open");

    let branch = "2026/11/02/ENG-1_login";
    let repo = assert_opened(&fake, outcome, branch, BASE_COMMIT);
    assert_eq!(repo.worktree, worktree_for(branch));
}

// SLICE-11-4
#[test]
fn date_parts_recognized_as_a_full_date() {
    let t = template("{yyyy}/{mm}/{dd}/{ticket}_{title}");
    assert_eq!(
        t.recognize("2026/11/02/ENG-1_login"),
        Some(parts(None, Some("20261102"), T, "login"))
    );
    for n in [
        "2026/11/2/ENG-1_login",
        "26/11/02/ENG-1_login",
        "2026/11/02/x/ENG-1_login",
    ] {
        assert_eq!(t.recognize(n), None, "{n}");
    }
}

// SLICE-11-4
#[test]
fn template_without_full_date_recognizes_no_date() {
    assert_eq!(
        template("{yyyy}/{ticket}_{title}").recognize("2026/ENG-1_login"),
        Some(parts(None, None, T, "login"))
    );
    assert_eq!(
        template("{ticket}_{title}").recognize("ENG-1_login"),
        Some(parts(None, None, T, "login"))
    );
    assert_eq!(
        template("{type}/{ticket}_{title}").recognize("feature/ENG-1_login"),
        Some(parts(Some("feature"), None, T, "login"))
    );
}

// SLICE-11-4
#[test]
fn type_matches_lowercase_ascii_letters() {
    let t = template("{type}/{ticket}_{title}");
    assert!(t.recognize("hotfix/ENG-1_login").is_some());
    for n in [
        "Feature/ENG-1_login",
        "feat2/ENG-1_login",
        "/ENG-1_login",
        "fé/ENG-1_login",
    ] {
        assert_eq!(t.recognize(n), None, "{n}");
    }
}

// SLICE-11-4
#[test]
fn date_matches_eight_digits() {
    let t = template("{date}-{ticket}_{title}");
    assert_eq!(
        t.recognize("20261102-ENG-1_login"),
        Some(parts(None, Some("20261102"), T, "login"))
    );
    for n in ["2026110-ENG-1_login", "202611021-ENG-1_login", "2026-11-02-ENG-1_login"] {
        assert_eq!(t.recognize(n), None, "{n}");
    }
}

// SLICE-11-4
#[test]
fn ticket_matches_a_ticket_reference() {
    let t = template("{ticket}_{title}");
    for (n, r) in [
        ("ENG-1_x", "ENG-1"),
        ("TBOARD-405_x", "TBOARD-405"),
        ("A1B-77_x", "A1B-77"),
    ] {
        assert_eq!(t.recognize(n).map(|b| b.ticket), Some(tref(r)), "{n}");
    }
    for n in ["eng-1_x", "1ENG-1_x", "ENG_x", "ENG-_x", "ENG-1a_x", "ENG-1-2_x"] {
        assert_eq!(t.recognize(n), None, "{n}");
    }
}

// SLICE-11-4
#[test]
fn title_matches_any_characters() {
    let t = template("{ticket}_{title}");
    assert_eq!(
        t.recognize("ENG-1_Mixed_Case/and.dots"),
        Some(parts(None, None, T, "Mixed_Case/and.dots"))
    );
}

// SLICE-11-4
#[test]
fn literal_text_matches_only_itself() {
    let t = template("wip.{ticket}+{title}");
    assert_eq!(t.recognize("wip.ENG-1+login"), Some(parts(None, None, T, "login")));
    for n in [
        "wipxENG-1+login",
        "wip.ENG-1login",
        "wip.ENG-1-login",
        "xwip.ENG-1+login",
    ] {
        assert_eq!(t.recognize(n), None, "{n}");
    }
}

// SLICE-11-4
#[test]
fn whole_name_must_match() {
    let t = template("{ticket}_{title}");
    for n in ["x/ENG-1_login", "feature/20261102-ENG-1_login", " ENG-1_login"] {
        assert_eq!(t.recognize(n), None, "{n}");
    }
}

// SLICE-11-4
#[test]
fn adjacent_fixed_width_placeholders_recognized() {
    let t = template("{date}{ticket}_{title}");
    assert_eq!(
        t.recognize("20261102ENG-1_login"),
        Some(parts(None, Some("20261102"), T, "login"))
    );
    let t = template("{ticket}{yyyy}{mm}{dd}-{title}");
    assert_eq!(
        t.recognize("ENG-1220261102-login"),
        Some(parts(None, Some("20261102"), "ENG-12", "login"))
    );
}

/// A real calendar day as `yyyymmdd`.
fn date() -> impl Strategy<Value = String> {
    (1970u32..2200, 1u32..=12, 1u32..=28).prop_map(|(y, m, d)| format!("{y:04}{m:02}{d:02}"))
}

proptest! {
    // SLICE-11-4
    #[test]
    fn default_template_round_trips(
        branch_type in "[a-z]{1,12}",
        date in date(),
        reference in "[A-Z][A-Z0-9]{0,7}-[0-9]{1,6}",
        title in "\\PC{0,80}",
    ) {
        let name = new_name(Some(&branch_type), &date, &reference, &title);
        let t = BranchTemplate::default();
        let branch = t.render(&name);
        prop_assert_eq!(t.recognize(&branch), Some(name), "{}", branch);
    }
}

// ---------------------------------------------------------------------------
// SLICE-11-5: entry matching
// ---------------------------------------------------------------------------

// SLICE-11-5
#[test]
fn entry_matches_its_path_exactly() {
    let s = settings_with(&[("/work/app", "{ticket}_{title}")]);
    assert_eq!(s.template_for(Path::new("/work/app")), template("{ticket}_{title}"));
    for repo in ["/work/app2", "/work/ap", "/work", "/work/app/sub", "/other/work/app"] {
        assert_eq!(s.template_for(Path::new(repo)), BranchTemplate::default(), "{repo}");
    }
}

// SLICE-11-5
#[test]
fn star_matches_one_path_segment() {
    let s = settings_with(&[("/repos/provision/*", UNDERSCORE)]);
    for repo in ["/repos/provision/infra", "/repos/provision/k8s-cluster"] {
        assert_eq!(s.template_for(Path::new(repo)), template(UNDERSCORE), "{repo}");
    }
    for repo in [
        "/repos/provision",
        "/repos/provision/infra/modules",
        "/repos/other/infra",
        "/repos",
    ] {
        assert_eq!(s.template_for(Path::new(repo)), BranchTemplate::default(), "{repo}");
    }
}

// SLICE-11-5
#[test]
fn star_matches_within_a_segment() {
    let s = settings_with(&[("/repos/*-api", UNDERSCORE), ("/x/*/app", "{ticket}_{title}")]);
    assert_eq!(s.template_for(Path::new("/repos/acme-api")), template(UNDERSCORE));
    assert_eq!(s.template_for(Path::new("/x/team/app")), template("{ticket}_{title}"));
    for repo in ["/repos/acme-web", "/repos/acme/x-api", "/x/a/b/app", "/x/team/app2"] {
        assert_eq!(s.template_for(Path::new(repo)), BranchTemplate::default(), "{repo}");
    }
}

// SLICE-11-5
#[test]
fn first_matching_entry_wins() {
    let broad = ("/repos/*", "{ticket}_{title}");
    let narrow = ("/repos/app", UNDERSCORE);
    let s = settings_with(&[broad, narrow]);
    assert_eq!(s.template_for(Path::new("/repos/app")), template("{ticket}_{title}"));
    let s = settings_with(&[narrow, broad]);
    assert_eq!(s.template_for(Path::new("/repos/app")), template(UNDERSCORE));
    assert_eq!(s.template_for(Path::new("/repos/web")), template("{ticket}_{title}"));
}

// SLICE-11-5
#[test]
fn config_entries_in_order_with_home_expanded() {
    let cfg = Config::parse(
        "[[repos]]\npath = \"~/repos/provision/*\"\nbranch = \"{type}/{date}-{ticket}_{title}\"\n\n\
         [[repos]]\npath = \"/srv/app\"\nbranch = \"{ticket}_{title}\"\n",
    )
    .unwrap();

    let s = cfg.settings().unwrap();

    assert_eq!(
        s.branch_templates,
        vec![
            (home().join("repos/provision/*"), template(UNDERSCORE)),
            (PathBuf::from("/srv/app"), template("{ticket}_{title}")),
        ]
    );
    assert_eq!(
        s.template_for(&home().join("repos/provision/infra")),
        template(UNDERSCORE)
    );
    assert_eq!(s.template_for(Path::new("/srv/app")), template("{ticket}_{title}"));
    assert_eq!(
        s.template_for(&home().join("repos/provision")),
        BranchTemplate::default()
    );
}

// SLICE-11-5
#[test]
fn open_uses_the_matching_entry() {
    let mut fake = Fake::new();
    fake.with_template("/work/ap*/sub", "{ticket}-{title}");
    fake.with_template("/work/*", UNDERSCORE);
    fake.with_template(REPO, "{ticket}-{title}");

    let outcome = open(&fake.deps(), &request(T, Some("feature"))).expect("open");

    assert_opened(&fake, outcome, "feature/20261102-ENG-1_login", BASE_COMMIT);
}

// ---------------------------------------------------------------------------
// SLICE-11-6: template without type
// ---------------------------------------------------------------------------

// SLICE-11-6
#[test]
fn has_type() {
    assert!(BranchTemplate::default().has_type());
    assert!(template(UNDERSCORE).has_type());
    assert!(template("{ticket}/{type}-{title}").has_type());
    assert!(!template("{ticket}_{title}").has_type());
    assert!(!template("{yyyy}/{mm}/{dd}/{ticket}_{title}").has_type());
}

// SLICE-11-6
#[test]
fn template_without_type_needs_no_type() {
    for branch_type in [None, Some("feature")] {
        let mut fake = Fake::new();
        fake.with_template(REPO, "{date}-{ticket}_{title}");

        let outcome = open(&fake.deps(), &request(T, branch_type)).expect("open");

        let branch = "20261102-ENG-1_login";
        let repo = assert_opened(&fake, outcome, branch, BASE_COMMIT);
        assert_eq!(repo.worktree, worktree_for(branch), "--type {branch_type:?}");
        assert_eq!(fake.snapshot().entries.len(), 1, "one new entry");
    }
}

// SLICE-11-6
#[test]
fn template_without_type_ignores_type_for_existing_branch() {
    for branch_type in [None, Some("feature"), Some("bug")] {
        let mut fake = Fake::new();
        fake.with_template(REPO, "{date}-{ticket}_{title}");
        let branch = "20261101-ENG-1_login";
        fake.with_branch(branch, T_BRANCH_OLD_HEAD, false);

        let outcome = open(&fake.deps(), &request(T, branch_type)).expect("open");

        let repo = assert_opened(&fake, outcome, branch, T_BRANCH_OLD_HEAD);
        assert_eq!(repo.worktree, worktree_for(branch), "--type {branch_type:?}");
        assert!(
            !fake.write_ops().contains(&"create_branch".to_string()),
            "no branch created: {:?}",
            fake.writes()
        );
    }
}

// ---------------------------------------------------------------------------
// SLICE-11-7: invalid template refused
// ---------------------------------------------------------------------------

/// Templates SLICE-11-7 refuses, each with words of which its error must use
/// one to name the problem; none are required where the problem can be put
/// in words that name no placeholder.
fn invalid_templates() -> Vec<(&'static str, &'static [&'static str])> {
    vec![
        ("{type}/{date}-{title}", &["ticket"]),
        ("{type}/{date}-{ticket}", &["title"]),
        ("{ticket}_{title}-{ticket}", &[]),
        ("{type}/{ticket}-{title}-{type}", &[]),
        ("{date}-{ticket}_{title}-{date}", &[]),
        ("{type}/{ticket}-{title}-{sprint}", &["sprint"]),
        ("{type}/{ticket}-{title}-{Type}", &["Type"]),
        ("{type}/{ticket}-{title}-{type", &[]),
        ("{type}/{ticket}-}{title}", &[]),
        ("{type}{ticket}-{title}", &[]),
        ("{ticket}{title}", &[]),
        ("{type}/{title}{ticket}", &[]),
    ]
}

/// The error names the problem with one of `words`, if any are given.
fn assert_names_problem(err: &str, template: &str, words: &[&str]) {
    assert!(
        words.is_empty() || words.iter().any(|w| err.contains(w)),
        "{template}: the error names the problem with one of {words:?}: {err}"
    );
}

// SLICE-11-7
#[test]
fn valid_templates_parse() {
    for t in [
        BranchTemplate::DEFAULT,
        UNDERSCORE,
        "{ticket}_{title}",
        "{type}/{ticket}_{title}",
        "{yyyy}/{mm}/{dd}/{ticket}_{title}",
        "{date}{ticket}_{title}",
        "{ticket}{date}-{title}",
        "{yyyy}{mm}{dd}/{ticket}-{title}",
        "{ticket}-{title}{dd}",
    ] {
        assert!(BranchTemplate::parse(t).is_ok(), "{t}: {:?}", BranchTemplate::parse(t));
    }
}

// SLICE-11-7
#[test]
fn invalid_templates_refused() {
    for (t, words) in invalid_templates() {
        let err = BranchTemplate::parse(t).expect_err(t);
        assert_names_problem(&err, t, words);
    }
}

/// Settings from a config whose second `[[repos]]` entry is `bad`.
fn settings_with_entry(bad: &str) -> Result<Settings, String> {
    let text = format!(
        "[[repos]]\npath = \"/srv/good\"\nbranch = \"{{ticket}}_{{title}}\"\n\n[[repos]]\n{bad}\n\n\
         [[repos]]\npath = \"/srv/later\"\nbranch = \"{{ticket}}-{{title}}\"\n"
    );
    Config::parse(&text)
        .unwrap_or_else(|e| panic!("{text}: {e}"))
        .settings()
}

// SLICE-11-7
#[test]
fn config_entry_with_invalid_template_refused() {
    for (t, words) in invalid_templates() {
        let err = settings_with_entry(&format!("path = \"/srv/bad\"\nbranch = \"{t}\"")).expect_err(t);
        assert!(err.contains("/srv/bad"), "{t}: the error names the entry: {err}");
        assert_names_problem(&err, t, words);
    }
}

// SLICE-11-7
#[test]
fn config_entry_without_template_refused() {
    let err = settings_with_entry("path = \"/srv/bad\"").expect_err("no template");
    assert!(err.contains("/srv/bad"), "the error names the entry: {err}");
    assert_names_problem(&err, "no branch", &["branch", "template"]);
}

// SLICE-11-7
#[test]
fn config_entry_without_path_refused() {
    let err = settings_with_entry("branch = \"{ticket}_{title}\"").expect_err("no path");
    assert!(
        err.contains('2') || err.contains("second"),
        "the error names the entry by its position, the second: {err}"
    );
    assert!(!err.contains("/srv/good"), "not the entry before it: {err}");
    assert!(!err.contains("/srv/later"), "not the entry after it: {err}");
    assert_names_problem(&err, "no path", &["path"]);
}

// ---------------------------------------------------------------------------
// SLICE-11-8: empty title
// ---------------------------------------------------------------------------

// SLICE-11-8
#[test]
fn empty_title_drops_title_and_the_text_before_it() {
    for (t, branch_type, want) in [
        (BranchTemplate::DEFAULT, Some("feature"), "feature/20261102-ENG-1"),
        (UNDERSCORE, Some("feature"), "feature/20261102-ENG-1"),
        ("{yyyy}/{mm}/{dd}/{ticket}__{title}", None, "2026/11/02/ENG-1"),
        ("{ticket}/{title}-wip", None, "ENG-1-wip"),
        ("{ticket}{date}{title}", None, "ENG-120261102"),
    ] {
        let name = new_name(branch_type, "20261102", T, "!!!");
        assert_eq!(name.short_title, "");
        assert_eq!(template(t).render(&name), want, "{t}");
    }
}

// SLICE-11-8
#[test]
fn open_with_empty_title() {
    let mut fake = Fake::new();
    fake.with_template(REPO, UNDERSCORE);
    fake.world.borrow_mut().tickets.get_mut(T).unwrap().title = "!!!".into();

    let outcome = open(&fake.deps(), &request(T, Some("feature"))).expect("open");

    let branch = "feature/20261102-ENG-1";
    let repo = assert_opened(&fake, outcome, branch, BASE_COMMIT);
    assert_eq!(repo.worktree, worktree_for(branch));
}

// ---------------------------------------------------------------------------
// SLICE-11-9: added repository uses its own template
// ---------------------------------------------------------------------------

/// An active slice for T in `REPO` on `branch`, with `REPO`'s template
/// `a` (the default when `None`), its timer running, and `API`, with no
/// branch for T and template `r` (the default when `None`).
fn slice_on(branch: &str, a: Option<&str>, r: Option<&str>) -> Fake {
    let mut fake = Fake::new();
    if let Some(a) = a {
        fake.with_template(REPO, a);
    }
    if let Some(r) = r {
        fake.with_template(API, r);
    }
    fake.with_active_slice_on(T, branch, true);
    fake.with_repo(API, API_HEAD);
    fake
}

// SLICE-11-9
#[test]
fn added_repository_writes_first_branchs_parts_with_its_own_template() {
    // The title is the one the first branch was named with, not the
    // ticket's current title.
    let fake = slice_on("feature/20261101-ENG-1-sign-in", None, Some(UNDERSCORE));

    let outcome = open(&fake.deps(), &request_in(API, T, None)).expect("open");

    assert_added(&fake, outcome, API, "feature/20261101-ENG-1_sign-in", API_HEAD);
}

// SLICE-11-9
#[test]
fn added_repository_takes_date_parts_as_a_full_date() {
    let fake = slice_on(
        "2026/10/30/ENG-1_login",
        Some("{yyyy}/{mm}/{dd}/{ticket}_{title}"),
        Some(UNDERSCORE),
    );

    let outcome = open(&fake.deps(), &request_in(API, T, Some("bug"))).expect("open");

    assert_added(&fake, outcome, API, "bug/20261030-ENG-1_login", API_HEAD);
}

// SLICE-11-9
#[test]
fn added_repository_without_type_drops_it() {
    let fake = slice_on("feature/20261101-ENG-1-login", None, Some("{ticket}_{title}"));

    let outcome = open(&fake.deps(), &request_in(API, T, None)).expect("open");

    assert_added(&fake, outcome, API, "ENG-1_login", API_HEAD);
}

// SLICE-11-9
#[test]
fn type_for_added_repository_from_type_option() {
    let fake = slice_on("ENG-1_login", Some("{ticket}_{title}"), None);

    let outcome = open(&fake.deps(), &request_in(API, T, Some("bug"))).expect("open");

    assert_added(&fake, outcome, API, "bug/20261102-ENG-1-login", API_HEAD);
}

// SLICE-11-9
#[test]
fn type_required_for_added_repository() {
    let fake = slice_on("20261030-ENG-1_login", Some("{date}-{ticket}_{title}"), None);
    let before = fake.snapshot();

    let err = open(&fake.deps(), &request_in(API, T, None)).expect_err("open must fail");

    assert_refused(&fake, &before, &err, &SliceError::TypeRequired);
}

// SLICE-11-9
#[test]
fn date_for_added_repository_is_today_without_a_full_date() {
    for (a, branch) in [
        ("{type}/{ticket}_{title}", "feature/ENG-1_login"),
        ("{type}/{yyyy}{mm}-{ticket}_{title}", "feature/202610-ENG-1_login"),
    ] {
        let fake = slice_on(branch, Some(a), None);

        let outcome = open(&fake.deps(), &request_in(API, T, None)).expect(a);

        assert_added(&fake, outcome, API, "feature/20261102-ENG-1-login", API_HEAD);
    }
}
