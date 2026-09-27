//! Task 2.2 (change `cli-remote-endpoint-config`): the CLI/MCP HTTP client pins
//! exactly one certificate and has no trust-bypass path. This spins a real
//! HTTPS server with cert A and confirms a `reqwest` blocking client that pins a
//! *different* self-signed cert (built the same way `client.rs` builds it —
//! `ClientBuilder::new().add_root_certificate(..)`, no `danger_*`) fails the
//! handshake rather than completing the request.

use std::net::TcpListener;

use axum::{routing::get, Router};
use axum_server::tls_rustls::RustlsConfig;
use rcgen::{CertificateParams, KeyPair};

/// Self-signed cert for `127.0.0.1` / `localhost`, mirroring `src/tls/mod.rs`.
fn make_self_signed() -> (String, String) {
    let params = CertificateParams::new(vec!["127.0.0.1".to_string(), "localhost".to_string()])
        .expect("cert params");
    let key_pair = KeyPair::generate().expect("keypair");
    let cert = params.self_signed(&key_pair).expect("self-sign");
    (cert.pem(), key_pair.serialize_pem())
}

#[tokio::test]
async fn pinned_client_rejects_a_mismatched_server_certificate() {
    // Some environments have no process-wide rustls crypto provider installed.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Server presents cert A.
    let (server_cert_pem, server_key_pem) = make_self_signed();
    // Client will pin cert B — a different, unrelated self-signed cert.
    let (other_cert_pem, _other_key_pem) = make_self_signed();

    let tls_config = RustlsConfig::from_pem(
        server_cert_pem.clone().into_bytes(),
        server_key_pem.into_bytes(),
    )
    .await
    .expect("rustls config from cert A");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");

    let app = Router::new().route("/health", get(|| async { "ok" }));
    tokio::spawn(async move {
        let _ = axum_server::from_tcp_rustls(listener, tls_config)
            .serve(app.into_make_service())
            .await;
    });

    let url = format!("https://{addr}/health");

    // Build the client exactly like `client.rs::build_pinned_or_plain`: pin a
    // certificate, nothing else. No `danger_accept_invalid_certs`.
    let result = tokio::task::spawn_blocking(move || {
        let cert = reqwest::Certificate::from_pem(other_cert_pem.as_bytes())
            .expect("parse pinned cert B");
        let client = reqwest::blocking::ClientBuilder::new()
            .add_root_certificate(cert)
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("build pinned client");
        client.get(&url).send()
    })
    .await
    .expect("join blocking task");

    assert!(
        result.is_err(),
        "a client pinning an unrelated cert must NOT complete the request against cert A"
    );

    // Sanity: pinning the *correct* cert A lets the same request through, proving
    // the failure above is cert mismatch and not a dead server.
    let good_url = format!("https://{addr}/health");
    let ok = tokio::task::spawn_blocking(move || {
        let cert = reqwest::Certificate::from_pem(server_cert_pem.as_bytes())
            .expect("parse cert A");
        let client = reqwest::blocking::ClientBuilder::new()
            .add_root_certificate(cert)
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("build client");
        client.get(&good_url).send().and_then(|r| r.error_for_status())
    })
    .await
    .expect("join blocking task");

    assert!(
        ok.is_ok(),
        "pinning the server's real cert A must succeed: {ok:?}"
    );
}
