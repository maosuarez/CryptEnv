//! Caller identification and the per-route MCP policy.
//!
//! `verify_token` classifies every authenticated request as a *session*
//! principal (a password-derived CLI/GUI session token) or the *MCP* principal
//! (the static MCP token). Handlers receive the result through the
//! [`AuthedPrincipal`] extractor, so a route cannot forget authentication.
//!
//! Every route is registered with an [`McpPolicy`] (see `build_router`). The
//! extractor enforces `Deny` centrally; a route registered without a policy
//! defaults to `Deny` for the MCP principal. `Approve` routes are enforced by
//! the handler itself, which creates a pending approval instead of acting.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use std::sync::Arc;
use std::time::Instant;
use subtle::ConstantTimeEq;

use super::{err_json, ApiState};

/// Who is calling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Principal {
    /// A password-derived CLI or GUI session token.
    Session,
    /// The static MCP token.
    Mcp,
}

/// What the MCP principal may do on a route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum McpPolicy {
    /// Same as a session caller (the handler may still narrow it, e.g. path confinement).
    Allow,
    /// 403 `MCP_FORBIDDEN`, no side effect.
    Deny,
    /// The handler creates a pending approval (202) instead of acting.
    Approve,
}

/// Verifies the `X-Vault-Token` header. Accepts a live session token (sliding
/// expiry) or the static MCP token (no expiry).
pub(super) async fn verify_token(headers: &HeaderMap, state: &ApiState) -> Result<Principal, StatusCode> {
    let provided = headers
        .get("x-vault-token")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Lock state first: a locked vault must never renew (or drop) a session.
    // The vault lock is released before `sessions` is taken (never nested).
    let epoch = {
        let vault = state.vault.lock().await;
        if vault.key.is_none() {
            return Err(StatusCode::FORBIDDEN);
        }
        vault.epoch
    };

    // --- Session token (sliding expiry, same epoch) ---
    if state.sessions.lock().await.touch(provided, Instant::now(), epoch) {
        return Ok(Principal::Session);
    }

    // --- Fallback: static MCP token (no expiry) ---
    let vault = state.vault.lock().await;
    if vault.key.is_none() {
        return Err(StatusCode::FORBIDDEN);
    }
    let mcp_token = vault
        .db
        .get_setting("mcp_token")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match mcp_token {
        Some(t) if bool::from(t.as_bytes().ct_eq(provided.as_bytes())) => Ok(Principal::Mcp),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

/// 403 `MCP_FORBIDDEN` response. `what` names the operation, never a value.
pub(super) fn mcp_forbidden(what: &str) -> Response {
    err_json(
        StatusCode::FORBIDDEN,
        &format!("not available to the MCP token: {what}"),
        "MCP_FORBIDDEN",
    )
    .into_response()
}

/// Axum extractor: authenticates the request and applies the route's MCP
/// policy. Put it before any body extractor.
pub(crate) struct AuthedPrincipal(pub Principal);

#[axum::async_trait]
impl FromRequestParts<Arc<ApiState>> for AuthedPrincipal {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<ApiState>) -> Result<Self, Self::Rejection> {
        let principal = verify_token(&parts.headers, state).await.map_err(|code| {
            let (msg, err_code) = match code {
                StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
                StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
                _ => ("internal error", "INTERNAL_ERROR"),
            };
            err_json(code, msg, err_code).into_response()
        })?;

        if principal == Principal::Mcp {
            let policy = parts.extensions.get::<McpPolicy>().copied().unwrap_or(McpPolicy::Deny);
            if policy == McpPolicy::Deny {
                return Err(mcp_forbidden("this operation is reserved for the user"));
            }
        }
        Ok(AuthedPrincipal(principal))
    }
}
