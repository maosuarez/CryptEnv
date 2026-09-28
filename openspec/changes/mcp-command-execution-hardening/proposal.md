## Why

The MCP server's command tools are the most direct way secrets leak and arbitrary code runs:
- **MCP-1:** `inject_env` calls `std::env::set_var` on the MCP process (`crypt-env-mcp.rs:1249`). `run_command` then spawns `sh -c` / `cmd /C` with the full inherited environment, pastes caller `params` raw into the shell string, and returns stdout. So `params: {p: "; printenv DB_PASSWORD"}` returns the plaintext secret, and any other command runs as well.
- **MCP-3:** output is truncated at a fixed byte offset (`&s[..2000]`, `:1617`). A multi-byte character at that boundary panics and kills the MCP server.
- **MCP-4:** there is no timeout, output is buffered without limit, and stdin is not nulled. A long-running command blocks the single-threaded MCP loop forever, and a child that reads stdin eats the JSON-RPC stream. Grandchildren are orphaned.
- **MCP-9:** `generate_env` writes plaintext secrets into shared `/tmp` at mode 0644 (`:1146`). The files outlive the process.

All of these tools read values through `/items/:id/reveal`, which `mcp-token-capabilities` closes to the MCP token.

## What Changes

- **Execution moves into the backend.** A new REST endpoint `POST /exec` (MCP-allowed) takes `{commandId, params, injectKeys, cwd?}`. The backend resolves secret values itself and spawns the stored command with a cleared environment plus a minimal baseline and the injected secrets. It returns only redacted, bounded output. The MCP binary never receives secret values.
- **`inject_env` becomes stateful key selection.** It records *key names* in the MCP session's injection set, which later `run_command` calls pass as `injectKeys`. It no longer mutates the MCP process environment.
- **Parameters are strictly validated.** Every `{{param}}` value must match `^[A-Za-z0-9._/:@=+,-]{0,256}$`; anything else is rejected. Shell metacharacters can never reach the shell.
- **Output is redacted and bounded.** Every injected secret value, plus its base64 and hex forms, is replaced by `[REDACTED:KEY]` before it is returned. Each stream is capped at 64 KiB captured and 2000 characters returned, truncated on a character boundary.
- **Processes are bounded:**
  - stdin is `null`;
  - there is a 120-second wall-clock timeout;
  - the whole process tree is killed on timeout or on MCP disconnect (a Unix process group, a Windows Job Object with kill-on-close).
- **WSL clients.** When the MCP client runs inside WSL and the backend runs on Windows, the backend runs the command in the caller's distribution through `wsl.exe -d <distro> --cd <dir> --exec`, passing the secrets over `WSLENV`-free stdin-fed env injection (see design D4).
- **`generate_env` requires GUI approval** (via `mcp-token-capabilities`). The file is written by the backend at mode 0600 into a private per-user directory, and is deleted after 10 minutes, at vault lock, and at app exit, with a startup sweep for leftovers.
- **BREAKING:** `run_command` rejects parameters containing shell metacharacters, and `inject_env` no longer affects commands run outside `run_command`.

## Capabilities

### New Capabilities
- `mcp-command-execution`: how stored commands run with injected secrets. Covers isolation, parameter rules, output redaction, resource bounds and temporary-file handling.

### Modified Capabilities
<!-- none -->

## Impact

- **Backend:** new `exec/` module (spawn, redaction, process-tree kill, WSL bridge); `api/mod.rs` gets the `/exec` route and `generate_env` moves backend-side.
- **MCP server:** `bin/crypt-env-mcp.rs` (`inject_env`, `run_command`, `generate_env`, `inject_env_by_name`).
- **Dependencies:** none new (`libc` and `windows` are already present; Job Object APIs come from the existing `windows` crate features, possibly with one feature flag added).
- **Docs:** the MCP section of `docs/index.html` (parameter rules, redaction, timeout).
- **Security:** removes the secret-exfiltration and RCE path through the MCP tools. It must ship together with `mcp-token-capabilities`.

## Non-Goals

- Sandboxing the stored command itself (network or filesystem). Commands are user-authored and trusted.
- Streaming or long-running command support (dev servers). Those are out of scope; the timeout applies.
- Detecting secrets that a command transforms beyond base64 or hex (see design Risks).
