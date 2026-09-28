// Integration tests for issue #7 — path traversal via environment name
// escapes `output_dir` on `/fill`, `/environments/{}/example` and inject.
// See docs/plans/issue-7-path-traversal-environment-name.md.
//
// Issue #11's HTTP test harness (`crate::test_support`, `TestVault`) has not
// landed on this branch, so the HTTP-level cases from the plan's §5 (T3's
// `/fill` secret non-exfiltration scan and T8's error-shape assertions) are
// not implemented here. Per the plan's own stated fallback: the non-HTTP
// cases below still cover objectives 1, 2, 4 and 6. `project::inject_environment`
// stands in as the one non-HTTP-reachable sink (`api::handle_fill` and
// `api::handle_environment_example` are private to the `api` module and only
// reachable through the axum router); all three sinks share the exact same
// `fsguard::resolve_within` call, which is exercised directly below.
//
// Run with: cargo test --test path_containment

use crypt_env_lib::db::VaultDb;
use crypt_env_lib::fsguard;
use crypt_env_lib::project::{self, EnvironmentInput, EnvironmentVar};
use std::fs;
use tempfile::tempdir;

/// At least 17 malicious names (plan §5/T1). Covers `../` runs, `..\` runs,
/// bare `..`/`.`, POSIX/Windows absolute, UNC, verbatim prefix, drive-
/// relative, NUL byte, percent-encoded and overlong-UTF-8-looking, Unicode
/// look-alikes, Windows reserved device names, trailing dot/space, and NTFS
/// alternate-data-stream. Over-length and empty/whitespace are covered
/// separately below (not naturally `&'static str` table entries).
const MALICIOUS: &[&str] = &[
    "../../../tmp/pwned",                        // POSIX traversal (the issue's repro)
    "..\\..\\..\\Windows\\Temp\\pwned",           // Windows traversal
    "..",                                         // bare parent
    ".",                                          // bare current
    "/etc/cron.d/pwned",                          // POSIX absolute
    "C:\\Windows\\Temp\\pwned",                   // Windows absolute
    "C:pwned",                                    // Windows drive-relative
    "\\\\wsl.localhost\\Ubuntu\\home\\u\\pwned",  // UNC
    "\\\\?\\C:\\pwned",                           // verbatim prefix
    "prod\0evil",                                 // NUL byte
    "%2e%2e%2fpwned",                             // percent-encoded
    "..%c0%af..%c0%afpwned",                      // overlong-UTF-8-looking
    "‥/pwned",                                    // Unicode look-alike (U+2025)
    "．．／pwned",                                 // Unicode look-alike (fullwidth)
    "CON",                                        // reserved device name
    "NUL",                                        // reserved device name
    "COM1",                                       // reserved device name
    "LPT1",                                       // reserved device name
    "prod.",                                      // trailing dot
    "prod ",                                      // trailing space
    "env:stream",                                 // NTFS ADS
];

fn overlength_name() -> String {
    "a".repeat(65)
}

// ─── T1 — the malicious table (objectives 1 and 2) ────────────────────────────

#[test]
fn t1_malicious_table_rejected_by_both_layers() {
    let dir = tempdir().unwrap();
    let base = dir.path().to_str().unwrap();

    for name in MALICIOUS {
        assert!(
            project::validate_environment_name(name).is_err(),
            "layer 1 (validate_environment_name) should reject {name:?}"
        );

        // Note on the percent-encoded / overlong-UTF-8-looking / Unicode
        // look-alike entries: these must be treated as literal filenames,
        // never decoded. Both outcomes below are acceptable per the plan —
        // rejection, or a path that resolves but stays a direct child of
        // the canonicalized base. Silently *decoding* them into a real `..`
        // is the only forbidden outcome, and that would surface as an
        // escaped/non-child path, which the assertions below would catch.
        match fsguard::resolve_within(base, &format!(".env.{name}")) {
            Err(_) => {}
            Ok(p) => {
                let canon = fs::canonicalize(base).unwrap();
                assert!(p.starts_with(&canon), "escaped base for {name:?}: {p:?}");
                assert_eq!(
                    p.parent(),
                    Some(canon.as_path()),
                    "not a direct child of base for {name:?}: {p:?}"
                );
            }
        }
    }

    // Over-length (>64), tested separately since `String` can't live in a
    // `&'static [&str]` table.
    let long = overlength_name();
    assert!(project::validate_environment_name(&long).is_err(), "layer 1 should reject over-length names");
    assert!(
        fsguard::resolve_within(base, &format!(".env.{long}")).is_err()
            || fsguard::resolve_within(base, &format!(".env.{long}"))
                .unwrap()
                .starts_with(fs::canonicalize(base).unwrap()),
        "layer 2 must at least contain an over-length name if it doesn't reject it"
    );

    // Whitespace-only is rejected by both layers.
    assert!(project::validate_environment_name("   ").is_err(), "layer 1 should reject whitespace-only");
    assert!(fsguard::resolve_within(base, "   ").is_err(), "layer 2 should reject whitespace-only");

    // The empty name is the unnamed root environment: valid at layer 1, but
    // it never reaches a sink raw — it maps to `.env`, a direct child of base.
    assert!(project::validate_environment_name("").is_ok(), "empty name is the root .env environment");
    assert!(fsguard::resolve_within(base, "").is_err(), "layer 2 should still reject a raw empty filename");
    let root = fsguard::resolve_within(base, &project::environment_filename("")).unwrap();
    assert_eq!(root.parent(), Some(fs::canonicalize(base).unwrap().as_path()));
    assert_eq!(root.file_name().and_then(|f| f.to_str()), Some(".env"));
}

// ─── T2 analog — filesystem invariant (objectives 1 and 4) ────────────────────
//
// The plan's T2 runs the table through all three HTTP-reachable sinks; here
// we run it through `fsguard::resolve_within` directly (the shared primitive
// behind all three) plus the one non-HTTP sink, `project::inject_environment`.

#[tokio::test]
async fn t2_filesystem_invariant_no_amplification() {
    let root = tempdir().unwrap();
    let base_dir = root.path().join("base");
    let sibling_dir = root.path().join("sibling");
    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&sibling_dir).unwrap();
    let canary_path = sibling_dir.join("canary.txt");
    fs::write(&canary_path, b"canary-untouched").unwrap();

    let base = base_dir.to_str().unwrap();
    for name in MALICIOUS {
        let _ = fsguard::resolve_within(base, &format!(".env.{name}"));
    }

    // Sibling directory (this issue's `/tmp/pwned` stand-in) byte-identical.
    assert_eq!(
        fs::read(&canary_path).unwrap(),
        b"canary-untouched",
        "a sibling of the base must never be touched"
    );

    // No directory named `.env...` (or any other attacker-named directory)
    // was created under base — kills the `create_dir_all` amplification
    // specifically (objective 4). Only the base itself may exist;
    // `fsguard::resolve_within` never creates anything beyond it.
    let created_dirs: Vec<String> = fs::read_dir(&base_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        created_dirs.is_empty(),
        "no directories should ever be created under base by a rejected name, found: {created_dirs:?}"
    );
}

#[tokio::test]
async fn t2_inject_environment_sink_no_amplification() {
    let db_dir = tempdir().unwrap();
    let db_path = db_dir.path().join("vault.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();
    let project_id = db.upsert_project(0, "t2-project", None, "generic").await.unwrap();

    let root = tempdir().unwrap();
    let base_dir = root.path().join("base");
    fs::create_dir_all(&base_dir).unwrap();
    let vault_key = [0u8; 32];

    for name in MALICIOUS {
        // Bypass layer 1 (`db::upsert_environment` directly) to exercise
        // layer 2 in isolation, same as a legacy row would. `inject_environment`
        // builds the filename as `.env.<env.name>` (same convention as
        // `/fill` and `/example`), so the stored name is used as-is here —
        // matching how T1 exercises `fsguard::resolve_within` directly.
        let env_id = match db.upsert_environment(0, project_id, name, false).await {
            Ok(id) => id,
            Err(_) => continue, // e.g. embedded NUL byte rejected by the DB driver itself
        };
        let _ = project::inject_environment(
            &db,
            &vault_key,
            env_id,
            None,
            Some(base_dir.to_str().unwrap().to_string()),
            false,
            None,
        )
        .await;
    }

    let created_dirs: Vec<String> = fs::read_dir(&base_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        created_dirs.is_empty(),
        "inject_environment must never create a directory under base via a hostile name, found: {created_dirs:?}"
    );
}

// ─── T4 — choke-point coverage (objective 6) ───────────────────────────────────
//
// Calls `project::save_environment` directly with no HTTP involved — this is
// the test that would have caught the original bug, where the HTTP handler
// was the only gate and the Tauri command path had none.

#[tokio::test]
async fn t4_choke_point_rejects_at_save_environment_directly() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("vault.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();
    let project_id = db.upsert_project(0, "t4-project", None, "generic").await.unwrap();

    let input = EnvironmentInput {
        id: 0,
        project_id,
        name: "../../../tmp/pwned".to_string(),
        is_default: false,
        paths: vec![],
        vars: Vec::<EnvironmentVar>::new(),
    };

    let result = project::save_environment(&db, input).await;
    assert!(result.is_err(), "save_environment must reject a traversal name with no HTTP involved");
    assert!(
        result.unwrap_err().starts_with("name: "),
        "rejection must be a layer-1 validation error, not a downstream failure"
    );
}

// ─── T5 — layer independence (objective 2) ─────────────────────────────────────
//
// Disabling either layer must leave the other's tests green. Implemented as
// two test fns that call each layer directly and in isolation, rather than
// by mutating production code.

#[test]
fn t5_layer2_alone_still_contains_every_malicious_name() {
    // Simulates "layer 1 disabled": call `fsguard::resolve_within` directly
    // (as a legacy row that bypassed `validate_environment_name` would),
    // without ever consulting layer 1.
    let dir = tempdir().unwrap();
    let base = dir.path().to_str().unwrap();
    for name in MALICIOUS {
        match fsguard::resolve_within(base, &format!(".env.{name}")) {
            Err(_) => {}
            Ok(p) => {
                let canon = fs::canonicalize(base).unwrap();
                assert!(p.starts_with(&canon), "layer 2 alone failed to contain {name:?}: {p:?}");
            }
        }
    }
}

#[test]
fn t5_layer1_alone_still_rejects_every_malicious_name() {
    // Simulates "layer 2 disabled": call `validate_environment_name` in
    // isolation, without ever consulting `fsguard`.
    for name in MALICIOUS {
        assert!(
            project::validate_environment_name(name).is_err(),
            "layer 1 alone failed to reject {name:?}"
        );
    }
}

// ─── T6 — positive path, no over-rejection (objective 5) ──────────────────────

#[test]
fn t6_valid_names_are_accepted_and_resolve_as_expected() {
    for name in ["production", "local", "staging-2", "v1.2", "a"] {
        assert!(project::validate_environment_name(name).is_ok(), "{name:?} should be a valid environment name");
    }
    let sixty_four = "a".repeat(64);
    assert!(project::validate_environment_name(&sixty_four).is_ok(), "64 chars should be accepted");

    let dir = tempdir().unwrap();
    let base = dir.path().to_str().unwrap();
    let resolved = fsguard::resolve_within(base, ".env.production").unwrap();
    let expected = fs::canonicalize(base).unwrap().join(".env.production");
    assert_eq!(resolved, expected, "objective 5: resolved path must be the canonicalized base joined with the filename");
}

// ─── T7 — legacy-name containment (D2) ─────────────────────────────────────────
//
// Inserts a row with a hostile name directly through `db::upsert_environment`,
// bypassing validation — simulating a pre-fix vault. Asserts `inject_environment`
// (the one non-HTTP sink) rejects it at layer 2, and that the environment
// remains listable and deletable — no vault bricking, per plan §4/D2.

#[tokio::test]
async fn t7_legacy_hostile_name_contained_and_vault_stays_usable() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("vault.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();
    let project_id = db.upsert_project(0, "legacy-project", None, "generic").await.unwrap();

    // Bypasses `project::validate_environment_name` entirely.
    let env_id = db
        .upsert_environment(0, project_id, "../../../tmp/pwned", false)
        .await
        .unwrap();

    let output_root = tempdir().unwrap();
    let base_dir = output_root.path().join("base");
    fs::create_dir_all(&base_dir).unwrap();
    let vault_key = [0u8; 32];

    let result = project::inject_environment(
        &db,
        &vault_key,
        env_id,
        None,
        Some(base_dir.to_str().unwrap().to_string()),
        false,
        None,
    )
    .await;
    assert!(result.is_err(), "a legacy hostile name must still be rejected at layer 2");

    // No amplification directory under base.
    let has_amplified_dir = fs::read_dir(&base_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| e.path().is_dir());
    assert!(!has_amplified_dir, "create_dir_all must never see the interpolated legacy name");

    // Vault stays usable: the environment is still listable and deletable.
    let envs = db.list_environments(project_id).await.unwrap();
    assert!(envs.iter().any(|e| e.id == env_id), "legacy environment must remain listable");
    db.delete_environment(env_id).await.unwrap();
    let envs_after = db.list_environments(project_id).await.unwrap();
    assert!(!envs_after.iter().any(|e| e.id == env_id), "legacy environment must remain deletable");
}

// ─── Project-name validation (D2) ──────────────────────────────────────────────
//
// Not one of the plan's numbered T-cases, but covers `validate_project_name`
// directly since `save_project` shares the same choke-point pattern.

#[test]
fn project_name_allows_human_labels_rejects_hostile_ones() {
    for name in ["My App", "Café Backend", "a", &"a".repeat(128)] {
        assert!(project::validate_project_name(name).is_ok(), "{name:?} should be a valid project name");
    }
    for name in ["../../../tmp/pwned", "CON", "a/b", "a\\b", "..", ".", "trailing.", "trailing ", &"a".repeat(129)] {
        assert!(project::validate_project_name(name).is_err(), "{name:?} should be rejected");
    }
}
