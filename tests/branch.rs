//! Branch names: `<type>/<yyyymmdd>-<REF>-<short title>` (SLICE-2's rationale).

use slice::ports::*;

fn ticket(r: &str, title: &str) -> Ticket {
    Ticket {
        reference: TicketRef(r.into()),
        system: TicketSystemKind::Monday,
        title: title.into(),
        status: String::new(),
    }
}

fn name(title: &str) -> String {
    BranchName::new(&BranchType("feature".into()), "20261102", &ticket("TBOARD-405", title)).to_string()
}

// SLICE-2-1
#[test]
fn branch_named_from_type_date_ref_and_title() {
    assert_eq!(name("Dashboard"), "feature/20261102-TBOARD-405-dashboard");
}

// SLICE-2-1
#[test]
fn short_title_is_lowercase_words_joined_by_hyphens() {
    assert_eq!(
        name("Add OAuth2 login & SSO (phase 1)!"),
        "feature/20261102-TBOARD-405-add-oauth2-login-sso-phase-1"
    );
    assert_eq!(
        name("  Spaces --  and   dashes "),
        "feature/20261102-TBOARD-405-spaces-and-dashes"
    );
}

// SLICE-2-1
#[test]
fn short_title_cut_at_a_word_near_40_characters() {
    let n = name("Shared access rules editor with conditional rules and delegated approvals");
    let short = n.strip_prefix("feature/20261102-TBOARD-405-").unwrap();
    assert!(short.len() <= 40, "{short} is {} chars", short.len());
    assert_eq!(short, "shared-access-rules-editor-with");
    assert!(!short.ends_with('-'));
}

// SLICE-2-1
#[test]
fn title_with_no_letters_or_digits_leaves_no_trailing_hyphen() {
    assert_eq!(name("!!!"), "feature/20261102-TBOARD-405");
}

// SLICE-2-9
#[test]
fn parse_takes_a_branch_name_apart() {
    let b = BranchName::parse("bug/20261020-ENG-14795-login-crash").expect("parses");
    assert_eq!(b.branch_type, BranchType("bug".into()));
    assert_eq!(b.date, "20261020");
    assert_eq!(b.ticket, TicketRef("ENG-14795".into()));
    assert_eq!(b.short_title, "login-crash");
    assert_eq!(b.to_string(), "bug/20261020-ENG-14795-login-crash");

    let bare = BranchName::parse("feature/20261102-TBOARD-405").expect("parses with no title");
    assert_eq!(bare.ticket, TicketRef("TBOARD-405".into()));
    assert_eq!(bare.short_title, "");
}

// SLICE-2-9
#[test]
fn parse_rejects_other_shapes() {
    for n in [
        "main",
        "dev",
        "slice/ENG-1-1",
        "feature/ENG-1-login",
        "feature/2026110-ENG-1-login",
        "Feature/20261102-ENG-1-login",
        "feature/20261102-eng-1-login",
        "feature/sub/20261102-ENG-1-login",
    ] {
        assert_eq!(BranchName::parse(n), None, "{n}");
    }
}

// SLICE-2-9
#[test]
fn parse_does_not_confuse_ticket_numbers() {
    let b = BranchName::parse("feature/20261102-ENG-12-other").unwrap();
    assert_eq!(b.ticket, TicketRef("ENG-12".into()));
    assert_ne!(b.ticket, TicketRef("ENG-1".into()));
}
