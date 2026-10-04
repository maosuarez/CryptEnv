//! MCP principal: per-route policy, output confinement, human approvals and
//! backend-side command execution
//! (`mcp-token-capabilities`, `mcp-command-execution-hardening`).

use axum::http::StatusCode;
use serde_json::json;

use crate::api::approvals::ApprovalSecret;
use crate::test_support::{req, router, unlocked_vault, TestVault};

fn code(json: &serde_json::Value) -> Option<&str> {
    json.get("code").and_then(|c| c.as_str())
}

/// Registers `dir` as the root of the seeded "demo" project.
async fn set_root(v: &TestVault, dir: &std::path::Path) {
    let s = v.state.lock().await;
    s.db.set_project_root(v.project_id, Some(dir.to_str().unwrap())).await.unwrap();
}

// ─── Policy table ─────────────────────────────────────────────────────────────

#[test]
fn every_route_has_an_explicit_mcp_policy() {
    for (method, path, _) in crate::api::api_routes() {
        assert!(
            crate::api::explicit_mcp_policy(method, path).is_some(),
            "{method} {path} has no explicit MCP policy"
        );
    }
    for (method, path, _) in crate::api::MCP_POLICIES {
        assert!(
            crate::api::api_routes().iter().any(|(m, p, _)| m == method && p == path),
            "stale policy entry {method} {path}"
        );
    }
}

#[tokio::test]
async fn route_without_a_policy_is_denied_to_mcp_but_open_to_the_session() {
    use axum::routing::get;
    let v = unlocked_vault().await;
    let app = axum::Router::new()
        .route(
            "/unpoliced",
            get(|_auth: crate::api::auth::AuthedPrincipal| async { "ok" }),
        )
        .with_state(v.api.clone());
    let (status, body) = req(&app, "GET", "/unpoliced", Some(&v.mcp_token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(code(&body), Some("MCP_FORBIDDEN"));
    let (status, _) = req(&app, "GET", "/unpoliced", Some(&v.token), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn every_deny_route_rejects_the_mcp_token_with_no_side_effect() {
    let v = unlocked_vault().await;
    let app = router(&v);
    for (method, path, policy) in crate::api::MCP_POLICIES {
        if *policy != crate::api::McpPolicy::Deny {
            continue;
        }
        let uri = path.replace(":id", &v.item_ids[1].to_string());
        let body = if *method == "GET" || *method == "DELETE" { None } else { Some(json!({})) };
        let (status, json) = req(&app, method, &uri, Some(&v.mcp_token), body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
        assert_eq!(code(&json), Some("MCP_FORBIDDEN"), "{method} {path}");
    }
}

// ─── Denials ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn mcp_reveal_is_forbidden_and_leaks_nothing_while_the_session_still_reveals() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let uri = format!("/items/{}/reveal", v.item_ids[1]);

    let (status, body) = req(&app, "POST", &uri, Some(&v.mcp_token), Some(json!({"confirm": true}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(code(&body), Some("MCP_FORBIDDEN"));
    assert!(!body.to_string().contains("hunter2"));

    let (status, body) = req(&app, "POST", &uri, Some(&v.token), Some(json!({"confirm": true}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.get("value").and_then(|x| x.as_str()), Some("hunter2"));
}

#[tokio::test]
async fn mcp_cannot_change_auto_lock_but_can_set_the_hotkey() {
    let v = unlocked_vault().await;
    let app = router(&v);

    let (status, body) =
        req(&app, "PUT", "/settings", Some(&v.mcp_token), Some(json!({"auto_lock_timeout": 0}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(code(&body), Some("MCP_FORBIDDEN"));
    let stored = v.state.lock().await.db.get_setting("auto_lock_timeout").await.unwrap();
    assert_eq!(stored, None, "setting must be unchanged");

    let (status, _) = req(&app, "PUT", "/settings", Some(&v.mcp_token), Some(json!({"hotkey": "Ctrl+Alt+K"}))).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = req(&app, "PUT", "/settings", Some(&v.token), Some(json!({"auto_lock_timeout": 0}))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn mcp_fill_is_confined_to_project_roots() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let root = tempfile::tempdir().unwrap();
    set_root(&v, root.path()).await;
    let uri = format!("/fill?environment_id={}", v.env_id);

    // Outside every root (the ~/.bashrc scenario): refused, nothing written.
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join(".bashrc");
    std::fs::write(&victim, "export A=1\n").unwrap();
    let body = json!({"template": "DB_HOST=\n", "output_path": victim, "overwrite": true});
    let (status, json) = req(&app, "POST", &uri, Some(&v.mcp_token), Some(body)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(code(&json), Some("MCP_FORBIDDEN"));
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "export A=1\n");

    // Inside the root: written.
    let inside = root.path().join("app/.env");
    let body = json!({"template": "DB_HOST=\n", "output_path": inside});
    let (status, json) = req(&app, "POST", &uri, Some(&v.mcp_token), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{json:?}");
    assert!(std::fs::read_to_string(&inside).unwrap().contains("DB_HOST=localhost"));
    assert!(json.get("content").is_none(), "no plaintext in the response");

    // Foreign file inside the root: overwrite is forbidden for MCP.
    let foreign = root.path().join("hand-written.env");
    std::fs::write(&foreign, "MINE=1\n").unwrap();
    let body = json!({"template": "DB_HOST=\n", "output_path": foreign, "overwrite": true});
    let (status, json) = req(&app, "POST", &uri, Some(&v.mcp_token), Some(body)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(code(&json), Some("MCP_FORBIDDEN"));
    assert_eq!(std::fs::read_to_string(&foreign).unwrap(), "MINE=1\n");

    // The user may overwrite it.
    let body = json!({"template": "DB_HOST=\n", "output_path": foreign, "overwrite": true});
    let (status, _) = req(&app, "POST", &uri, Some(&v.token), Some(body)).await;
    assert_eq!(status, StatusCode::OK);

    // Inline content would be a plaintext secret in an MCP-visible body.
    let (status, json) =
        req(&app, "POST", &uri, Some(&v.mcp_token), Some(json!({"template": "DB_HOST=\n"}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!json.to_string().contains("localhost"));
}

#[tokio::test]
async fn mcp_output_dir_outside_a_root_is_refused() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let root = tempfile::tempdir().unwrap();
    set_root(&v, root.path()).await;
    let outside = tempfile::tempdir().unwrap();
    let uri = format!("/fill?environment_id={}", v.env_id);

    let body = json!({"template": "DB_HOST=\n", "output_dir": outside.path()});
    let (status, _) = req(&app, "POST", &uri, Some(&v.mcp_token), Some(body)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let body = json!({"template": "DB_HOST=\n", "output_dir": root.path()});
    let (status, json) = req(&app, "POST", &uri, Some(&v.mcp_token), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{json:?}");

    let ex = format!("/environments/{}/example", v.env_id);
    let (status, _) = req(&app, "POST", &ex, Some(&v.mcp_token), Some(json!({"output_path": "/etc/x.example"}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ─── Approvals ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn mcp_share_export_waits_for_approval_and_never_sees_the_passphrase() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let out = v.dir.path().join("pkg.cenv");
    let body = json!({"items": [v.item_ids[1]], "output_path": out});

    let (status, accepted) = req(&app, "POST", "/share/export", Some(&v.mcp_token), Some(body)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted:?}");
    assert_eq!(accepted.get("status").and_then(|s| s.as_str()), Some("pending"));
    assert!(accepted.get("passphrase").is_none());
    assert!(!out.exists(), "nothing is written before approval");
    let id = accepted.get("approvalId").and_then(|s| s.as_str()).unwrap().to_string();

    let (status, poll) = req(&app, "GET", &format!("/approvals/{id}"), Some(&v.mcp_token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(poll.get("status").and_then(|s| s.as_str()), Some("pending"));
    assert_eq!(poll["summary"]["item_count"], 1);
    assert_eq!(poll["summary"]["items"][0], "DB_PASSWORD");

    // The user approves in the desktop app (the Tauri command calls this).
    let resolution = v.api.resolve_approval(&id, true).await.unwrap();
    let passphrase = match resolution.secret {
        Some(ApprovalSecret::Export { passphrase, .. }) => passphrase,
        _ => panic!("the GUI must receive the export passphrase"),
    };
    assert!(!passphrase.is_empty());
    assert!(out.exists(), "approved export is written");

    let (_, poll) = req(&app, "GET", &format!("/approvals/{id}"), Some(&v.mcp_token), None).await;
    assert_eq!(poll.get("status").and_then(|s| s.as_str()), Some("approved"));
    assert_eq!(poll["meta"]["item_count"], 1);
    assert!(!poll.to_string().contains(&passphrase), "polling must not reveal the passphrase");
    assert!(!accepted.to_string().contains(&passphrase));
}

#[tokio::test]
async fn mcp_relay_send_is_pending_denied_and_never_uploads() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let body = json!({"item_ids": [v.item_ids[0], v.item_ids[1]]});
    let (status, accepted) = req(&app, "POST", "/relay/send", Some(&v.mcp_token), Some(body)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(accepted.get("code").is_none() && accepted.get("passphrase").is_none());
    let id = accepted.get("approvalId").and_then(|s| s.as_str()).unwrap().to_string();

    let resolution = v.api.resolve_approval(&id, false).await.unwrap();
    assert!(resolution.secret.is_none());
    let (_, poll) = req(&app, "GET", &format!("/approvals/{id}"), Some(&v.mcp_token), None).await;
    assert_eq!(poll.get("status").and_then(|s| s.as_str()), Some("denied"));
    assert!(v.api.resolve_approval(&id, true).await.is_err(), "a denied request cannot be approved later");
}

#[tokio::test]
async fn approved_relay_send_error_carries_no_secret_material() {
    // The relay is not configured in the test vault, so execution fails; the
    // MCP-visible outcome must still be free of code/passphrase.
    let v = unlocked_vault().await;
    let app = router(&v);
    let (_, accepted) = req(&app, "POST", "/relay/send", Some(&v.mcp_token), Some(json!({"item_ids": [v.item_ids[1]]}))).await;
    let id = accepted.get("approvalId").and_then(|s| s.as_str()).unwrap().to_string();
    let resolution = v.api.resolve_approval(&id, true).await.unwrap();
    assert!(resolution.secret.is_none());
    let (_, poll) = req(&app, "GET", &format!("/approvals/{id}"), Some(&v.mcp_token), None).await;
    assert_eq!(poll["meta"]["ok"], false);
    for needle in ["passphrase", "hunter2"] {
        let text = poll["meta"].to_string();
        assert!(!text.contains(&format!("\"{needle}\"")), "{text}");
    }
}

#[tokio::test]
async fn ninth_pending_approval_is_rejected_with_429() {
    let v = unlocked_vault().await;
    let app = router(&v);
    for _ in 0..8 {
        let (status, _) = req(&app, "POST", "/relay/send", Some(&v.mcp_token), Some(json!({"item_ids": [v.item_ids[0]]}))).await;
        assert_eq!(status, StatusCode::ACCEPTED);
    }
    let (status, body) =
        req(&app, "POST", "/relay/send", Some(&v.mcp_token), Some(json!({"item_ids": [v.item_ids[0]]}))).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(code(&body), Some("APPROVALS_FULL"));
}

#[tokio::test]
async fn lock_discards_pending_approvals() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let (_, accepted) = req(&app, "POST", "/relay/send", Some(&v.mcp_token), Some(json!({"item_ids": [v.item_ids[0]]}))).await;
    let id = accepted.get("approvalId").and_then(|s| s.as_str()).unwrap().to_string();

    crate::vault::lock_vault(&v.state).await;
    // Unlock again (new lock epoch): the old approval is gone and cannot be approved.
    {
        let mut s = v.state.lock().await;
        let key = crate::crypto::unlock_vault_crypto(
            v.master_password.as_bytes(),
            &s.db.get_meta().await.unwrap().unwrap().0,
            &s.db.get_meta().await.unwrap().unwrap().1,
        )
        .unwrap();
        s.set_key(Some(zeroize::Zeroizing::new(key)));
    }
    assert!(v.api.approval_view(&id).await.is_none());
    assert!(v.api.resolve_approval(&id, true).await.is_err());
}

#[tokio::test]
async fn mcp_server_writes_need_approval_for_mcp_and_are_direct_for_the_session() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let cwd = tempfile::tempdir().unwrap();
    let config = cwd.path().join(".claude").join("mcp_servers.json");
    let body = json!({"name": "demo", "command": "node", "args": ["a.js"], "scope": "project", "cwd": cwd.path()});

    let (status, json) = req(&app, "POST", "/mcp-servers", Some(&v.mcp_token), Some(body.clone())).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{json:?}");
    assert!(!config.exists(), "no host config written before approval");
    let id = json.get("approvalId").and_then(|s| s.as_str()).unwrap().to_string();

    v.api.resolve_approval(&id, true).await.unwrap();
    assert!(std::fs::read_to_string(&config).unwrap().contains("\"demo\""));

    let del = json!({"name": "demo", "scope": "project", "cwd": cwd.path()});
    let uri = format!("/mcp-servers?name=demo&scope=project&cwd={}", cwd.path().to_str().unwrap());
    let (status, _) = req(&app, "DELETE", &uri, Some(&v.token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!std::fs::read_to_string(&config).unwrap().contains("\"demo\""));
    let _ = del;

    let (status, _) = req(&app, "POST", "/mcp-servers", Some(&v.token), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "session writes succeed immediately");
}

// ─── generate-env ─────────────────────────────────────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn generated_env_is_private_gated_and_deleted_on_lock() {
    use std::os::unix::fs::PermissionsExt;
    let v = unlocked_vault().await;
    let app = router(&v);
    let body = json!({"keys": ["DB_PASSWORD", "NOPE"], "environment_id": v.env_id});

    let (status, accepted) = req(&app, "POST", "/generate-env", Some(&v.mcp_token), Some(body)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted:?}");
    let id = accepted.get("approvalId").and_then(|s| s.as_str()).unwrap().to_string();
    v.api.resolve_approval(&id, true).await.unwrap();

    let (_, poll) = req(&app, "GET", &format!("/approvals/{id}"), Some(&v.mcp_token), None).await;
    let path = std::path::PathBuf::from(poll["meta"]["path"].as_str().unwrap());
    assert!(!poll.to_string().contains("hunter2"), "values never reach the MCP caller");
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(path.starts_with(v.dir.path().join("mcp-tmp")), "private directory, not a shared /tmp");
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("DB_PASSWORD=hunter2"));
    assert!(content.contains("# NOPE: not found in vault"));

    crate::vault::lock_vault(&v.state).await;
    assert!(!path.exists(), "lock deletes generated files");
}

#[cfg(unix)]
#[tokio::test]
async fn generated_env_quotes_multiline_and_special_values() {
    let v = unlocked_vault().await;
    let app = router(&v);
    {
        let s = v.state.lock().await;
        let key = s.key.clone().unwrap();
        for (name, value) in [("PEM_KEY", "line1\nline2"), ("WITH_SPACE", "a b #c")] {
            let item = crate::vault::VaultItem {
                id: 0,
                item_type: "secret".to_string(),
                name: Some(name.to_string()),
                value: Some(value.to_string()),
                url: None,
                username: None,
                password: None,
                title: None,
                description: None,
                command: None,
                shell: None,
                categories: None,
                notes: None,
                content: None,
                created: "0".to_string(),
                is_global: Some(true),
            };
            let enc = crate::vault::encrypt_item(&key, &item).unwrap();
            s.db.upsert_item(0, "secret", &enc, "0", true).await.unwrap();
        }
    }
    let body = json!({"keys": ["PEM_KEY", "WITH_SPACE"], "environment_id": v.env_id});
    let (_, accepted) = req(&app, "POST", "/generate-env", Some(&v.mcp_token), Some(body)).await;
    let id = accepted.get("approvalId").and_then(|s| s.as_str()).unwrap().to_string();
    v.api.resolve_approval(&id, true).await.unwrap();
    let (_, poll) = req(&app, "GET", &format!("/approvals/{id}"), Some(&v.mcp_token), None).await;
    let content = std::fs::read_to_string(poll["meta"]["path"].as_str().unwrap()).unwrap();

    assert_eq!(poll["meta"]["count"], 2, "{poll:?}");
    let parsed = crate::envfile::parse_dotenv(&content);
    assert!(parsed.contains(&("PEM_KEY".to_string(), "line1\nline2".to_string())), "{content}");
    assert!(parsed.contains(&("WITH_SPACE".to_string(), "a b #c".to_string())), "{content}");
    assert_eq!(parsed.len(), 2, "a multi-line value must not inject variables");
}

// ─── POST /exec ───────────────────────────────────────────────────────────────

/// Seeds a global command item and returns its id.
async fn seed_command(v: &TestVault, name: &str, command: &str) -> i64 {
    let s = v.state.lock().await;
    let key = s.key.clone().unwrap();
    let mut item = crate::vault::VaultItem {
        id: 0,
        item_type: "command".to_string(),
        name: Some(name.to_string()),
        value: None,
        url: None,
        username: None,
        password: None,
        title: None,
        description: None,
        command: Some(command.to_string()),
        shell: None,
        categories: None,
        notes: None,
        content: None,
        created: "0".to_string(),
        is_global: Some(true),
    };
    item.id = 0;
    let enc = crate::vault::encrypt_item(&key, &item).unwrap();
    s.db.upsert_item(0, "command", &enc, "0", true).await.unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn exec_injects_secrets_and_redacts_them_from_the_mcp_response() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let id = seed_command(&v, "show", "printenv DB_PASSWORD; echo host={{host}}").await;
    let uri = format!("/exec?environment_id={}", v.env_id);
    let body = json!({"commandId": id, "params": {"host": "db.internal"}, "injectKeys": ["DB_PASSWORD"]});

    let (status, json) = req(&app, "POST", &uri, Some(&v.mcp_token), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{json:?}");
    let out = json["stdout"].as_str().unwrap();
    assert!(out.contains("[REDACTED:DB_PASSWORD]"), "{out}");
    assert!(out.contains("host=db.internal"));
    assert!(!json.to_string().contains("hunter2"));
    assert_eq!(json["exitCode"], 0);
    assert_eq!(json["timedOut"], false);
}

#[cfg(unix)]
#[tokio::test]
async fn exec_rejects_shell_metacharacters_before_any_process_starts() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let marker = v.dir.path().join("should-not-exist");
    let id = seed_command(&v, "touchy", "echo {{p}}").await;
    let uri = format!("/exec?environment_id={}", v.env_id);
    let body = json!({
        "commandId": id,
        "params": {"p": format!("; touch {}", marker.display())},
        "injectKeys": ["DB_PASSWORD"],
    });
    let (status, json) = req(&app, "POST", &uri, Some(&v.mcp_token), Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{json:?}");
    assert!(json["error"].as_str().unwrap().contains("'p'"));
    assert!(!marker.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn exec_unknown_command_key_and_blocked_key() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let id = seed_command(&v, "c", "true").await;
    let uri = format!("/exec?environment_id={}", v.env_id);

    let (status, _) = req(&app, "POST", &uri, Some(&v.mcp_token), Some(json!({"commandId": 999999}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = req(&app, "POST", &uri, Some(&v.mcp_token), Some(json!({"commandId": id, "injectKeys": ["NOPE_KEY"]}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = req(&app, "POST", &uri, Some(&v.mcp_token), Some(json!({"commandId": id, "injectKeys": ["PATH"]}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[cfg(unix)]
#[tokio::test]
async fn locking_the_vault_kills_a_running_command() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let id = seed_command(&v, "slow", "sleep 60").await;
    let uri = format!("/exec?environment_id={}", v.env_id);

    let state = v.state.clone();
    let locker = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        crate::vault::lock_vault(&state).await;
    });
    let started = std::time::Instant::now();
    let (status, json) = req(&app, "POST", &uri, Some(&v.mcp_token), Some(json!({"commandId": id}))).await;
    locker.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{json:?}");
    assert_eq!(json["cancelled"], true);
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "the run was killed, not waited out");
}

#[cfg(unix)]
#[tokio::test]
async fn delete_exec_cancels_a_run_by_id() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let id = seed_command(&v, "slow", "sleep 60").await;
    let uri = format!("/exec?environment_id={}", v.env_id);

    let app2 = app.clone();
    let token = v.mcp_token.clone();
    let canceller = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        req(&app2, "DELETE", "/exec/run-abcdef12", Some(&token), None).await
    });
    let body = json!({"commandId": id, "runId": "run-abcdef12"});
    let (status, json) = req(&app, "POST", &uri, Some(&v.mcp_token), Some(body)).await;
    let (cancel_status, _) = canceller.await.unwrap();
    assert_eq!(cancel_status, StatusCode::OK);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["cancelled"], true);
}

#[cfg(unix)]
#[tokio::test]
async fn a_command_over_the_time_limit_reports_timed_out_not_a_failure() {
    let v = unlocked_vault().await;
    v.api.set_exec_timeout_ms(300);
    let app = router(&v);
    let id = seed_command(&v, "slow", "sleep 60 & sleep 60").await;
    let uri = format!("/exec?environment_id={}", v.env_id);
    let started = std::time::Instant::now();
    let (status, json) = req(&app, "POST", &uri, Some(&v.mcp_token), Some(json!({"commandId": id}))).await;
    assert_eq!(status, StatusCode::OK, "{json:?}");
    assert_eq!(json["timedOut"], true);
    assert!(json["exitCode"].is_null());
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}
