# SPOOL.md

Spool is a requirements traceability tool. It links structured requirements (defined in `.req` YAML files) to test annotations in source code, then reports which acceptance criteria have tests and which don't.

## .req File Format

Each `.req` file defines one feature and its requirements. Files live in the `.spool/` directory.

```yaml
feature: AUTH                          # Uppercase name, must match ID prefix

requirements:
  - id: AUTH-1                         # PREFIX-N format
    title: User Login
    description: >
      The system must authenticate users via email and password.
    status: active                     # active | draft | deprecated
    deciders:                          # Who decided on this requirement
      - Jane Smith
    consulted: []                      # Who was consulted
    date: 2026-03-28                   # When the requirement was created
    rationale: >                       # Why this requirement exists
      Email/password is the simplest auth method for MVP.
    superseded_by:                     # If deprecated, what replaced it
    acceptance_criteria:

      - id: AUTH-1-1                   # PREFIX-N-N format
        title: Valid Credentials
        description: >
          Given a registered user with valid credentials,
          when login is attempted,
          then authentication succeeds and a session is created.

      - id: AUTH-1-2
        title: Invalid Password
        description: >
          Given a registered user with an incorrect password,
          when login is attempted,
          then authentication fails with an error message.
```

## Key Rules

- **One feature per file.** The filename is typically `featurename.req`.
- **IDs must match the feature prefix.** Feature `AUTH` → requirements `AUTH-1`, `AUTH-2` → ACs `AUTH-1-1`, `AUTH-1-2`.
- **IDs are uppercase with no leading zeros.** `AUTH-1-1`, not `auth-01-01`.
- **Only `status: active` requirements are tracked** for coverage. Use `draft` for work-in-progress and `deprecated` for retired requirements.
- **Descriptions use Given/When/Then format** for acceptance criteria (recommended, not enforced).

## Test Annotations

Link a test to an AC by placing a comment with the AC's Spool ID above the test function:

```go
// AUTH-1-1
func TestValidLogin(t *testing.T) {
    // test implementation
}
```

```typescript
// AUTH-1-2
it('rejects invalid password', () => {
    // test implementation
});
```

```rust
// AUTH-1-1
#[test]
fn valid_login() {
    // test implementation
}
```

Rust tests are functions whose attributes include one ending in `test` (or `#[test_case]`); put the ID above the attributes. Supported comment styles: `//` and `#`. Multiple annotations can be stacked above a single test function.

## Configuration

`.spool.yaml` at the project root. `spool init` writes one based on the project types it detects:

```yaml
test_patterns:                 # files to scan for annotated tests
  - "**/*_test.go"             # Go
  # - "**/*.rs"                # Rust (#[test], #[tokio::test], #[rstest], ...)
  # - "**/*.test.*"            # Jest/Vitest
  # - "**/*.spec.*"            # Playwright
  # - "**/src/test/**/*.java"  # JUnit
exclude_patterns:              # directories and files never scanned
  - "**/vendor"
  # - "**/target"
  # - "**/node_modules"
```

Patterns are Go globs matched against the path from the project root; `**` matches any number of directories. When `exclude_patterns` is absent it defaults to `**/node_modules` and `**/vendor`; setting it replaces those defaults. `.git` is always skipped.

## Running

```bash
spool init           # Set up .spool.yaml, .spool/ and SPOOL.md
spool                # Print traceability report
spool lsp            # Start LSP server (used by editor extensions)
```
