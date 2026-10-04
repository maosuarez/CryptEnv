## Context

The MCP binary (`crypt-env-mcp`) runs as a stdio child of an AI host. It may run on Windows next to the backend, or inside WSL talking to the Windows backend over `CRYPTENV_API_URL` (see `docs/wsl-windows.md`). Stored commands are vault items of type `command`, with `{{param}}` placeholders. The token is readable by the agent, so anything the MCP binary can fetch, the agent can too. That is why values must never cross the API boundary toward MCP.

## Goals / Non-Goals

**Goals:** secrets are never visible to the MCP principal; no shell injection through params; bounded resources; no orphaned processes.

**Non-Goals:** sandboxing user-authored commands.

## Decisions

### D1. `POST /exec` in the backend
The request is `{commandId, params, injectKeys[], cwd?, clientContext}`, and the response is `{exitCode|null, timedOut, stdout, stderr}`, redacted.

The backend flow:
1. load the command item;
2. validate params;
3. substitute;
4. resolve `injectKeys` against the scope rules the MCP tools already use (project/environment params);
5. decrypt values into a `Zeroizing<String>` map;
6. spawn;
7. collect output;
8. redact;
9. zeroize.

Execution runs in `spawn_blocking` with a semaphore of 4, and never under the vault lock (values are copied out first).

*Rejected:*
- `/mcp/materialize` returning values to MCP: the token holder could read them.
- Removing exec entirely: loses the feature, and the user chose backend execution.

### D2. Parameter allowlist rather than quoting
POSIX single-quote escaping is sound, but `cmd.exe` quoting is not reliably escapable (`%VAR%` expansion, `^`, and `!` with delayed expansion). One conservative allowlist on all platforms avoids per-shell escaping bugs. The rejected characters include space, which is **BREAKING** for some commands. Commands needing spaces should use separate params.

### D3. Redaction
For each injected value `v` with `len >= 4`, redact `v`, `base64(v)` (standard and URL-safe, unpadded), and `hex(v)`. This is done with aho-corasick over the captured bytes before UTF-8 lossy decoding. `aho-corasick` is already in the tree through `regex`, so it is added as a direct dependency with no new crate. Values shorter than 4 characters are not redacted, because the false-positive rate would be too high; this is documented.

### D4. Process tree control and the WSL bridge
- **Unix:** `Command::process_group(0)`; on timeout `killpg(pgid, SIGKILL)`.
- **Windows:** the child is created suspended and assigned to a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, then resumed. Dropping the job handle kills the tree.
- **WSL callers:** the MCP binary sends `clientContext {os:"linux", wslDistro, cwd}`, detected from `WSL_DISTRO_NAME`. The backend spawns `wsl.exe -d <distro> --cd <cwd> --exec /usr/bin/env -i <baseline> sh -c <cmd>`. Secrets must not appear in argv (visible in `ps`) or in `WSLENV`, which needs the values in the Windows environment of `wsl.exe`. That is acceptable only because that env belongs to our own child. Instead, the backend writes `KEY=value\0...` to the child's stdin, and a fixed POSIX prelude reads it and exports the values before `exec`-ing the command. stdin is therefore used for secret delivery and closed afterwards; the command itself still sees `/dev/null`.
- The MCP disconnect is detected when the stdio loop ends. The MCP binary then sends `DELETE /exec/:runId`, and the backend kills the run. Runs are also bound to the lock epoch, so a lock kills them.

### D5. `generate_env`
The backend writes into `<app_data>/mcp-tmp/` (0700), with `create_new` at mode 0600. On Windows the file inherits the per-user app-data ACL. The backend tracks the files and deletes them after 10 minutes via the auto-lock tick, on lock (epoch change) and on exit, and sweeps the directory at startup. The approval requirement comes from `mcp-token-capabilities`.

## Security & Threat Model

- **Adversary:** a prompt-injected agent holding the MCP token.
- **After this change, it can:**
  - run user-stored commands with allowlisted params;
  - see their redacted output;
  - request `generate_env` (which needs human approval).
- **It cannot:**
  - read values through the API;
  - inject shell syntax;
  - hang the MCP server;
  - leave orphaned processes.
- **Residual:** a stored command that transforms a secret (for example `rev`) and prints it defeats redaction. Commands are user-authored, so this is documented in the tool description and the docs.

## Risks / Trade-offs

- [Allowlist breaks params containing spaces] → Documented as breaking; the error message explains the allowed charset.
- [WSL bridge complexity] → Covered by an integration test on Windows (manual task); Linux-native backends take the direct path.
- [Semaphore of 4 queues bursts] → Returns 429 when saturated beyond a queue of 8.

## Migration Plan

Ships in the same release as `mcp-token-capabilities`. There is no persistent state; leftover `/tmp/crypt_env_*.env` files from older versions are swept by the MCP binary on startup (best effort, owned files only).
