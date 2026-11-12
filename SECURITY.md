# RUX security (experimental)

RUX is currently an **experimental language/runtime**. The generated CRUD is **not yet suitable for sensitive production workloads** or multi-user applications. In particular, it does not currently provide end-user login, sessions, per-record authorization, encryption in transit, or an independently audited security model.

## Secure runtime defaults

- The native CRUD binds to `127.0.0.1:3000` by default.
- Remote binds require an explicit opt-in, a write API token with at least 32 characters, and `RUX_ALLOWED_HOSTS`.
- All HTTP routes are rate-limited by **TCP peer IP**, defaulting to **120 requests/minute** (`RUX_RATE_LIMIT_PER_MINUTE`). At a reverse proxy, peer IP is usually the proxy's address, not the end user's address: enforce per-client limits at the gateway too.
- JSON bodies are capped at **32 KiB**.
- The API uses separate write (`RUX_API_TOKEN`) and optional read-only (`RUX_API_READ_TOKEN`) bearer tokens; when configured, read-only tokens cannot modify records. **These are shared application credentials, not identities for individual users.**
- `/metrics` is disabled by default. It requires `RUX_EXPOSE_METRICS=true`, a configured write token, and an authorized Bearer header.
- Host headers are checked against explicitly allowed names, and browser cross-origin writes are rejected. CORS remains closed; no external origin is allowed by default.
- Common response-hardening headers are set. HTTPS/TLS is **not implemented by RUX itself**; provide TLS at a trusted reverse proxy or ingress.

## Local development

```powershell
cargo run --release -- dev teste/main.rux
```

Local CRUD is deliberately unauthenticated for development and must not be exposed to untrusted users. Do not store sensitive information in development fixtures.

## Deployment configuration

Create a random secret on a trusted machine and store it securely; do not commit it, embed it in the frontend, or put it in a public log:

```powershell
cargo run --release -- secret generate
```

Example PowerShell configuration for an application behind an HTTPS-terminating reverse proxy:

```powershell
$env:RUX_ENV = "production"
$env:RUX_HOST = "0.0.0.0"
$env:RUX_ALLOW_UNAUTHENTICATED_REMOTE = "true"
$env:RUX_ALLOWED_HOSTS = "api.example.com"
$env:RUX_API_TOKEN = "<random-generated-secret>"
$env:RUX_RATE_LIMIT_PER_MINUTE = "600"
cargo run --release -- start teste/main.rux
```

The legacy variable `RUX_ALLOW_UNAUTHENTICATED_REMOTE` permits **remote binding**, not anonymous remote API access: the runtime still requires a write token. This switch will be renamed in a future release.

**The configuration above is illustrative, not a production-readiness guarantee.** Use TLS, hardened ingress rules, a secret manager, network segmentation, and tested backup/recovery procedures. The generated browser UI does not yet implement user login for token-protected APIs.

## Security limitations / planned work

1. Native user identity, sessions, strong credential storage, and recovery flows.
2. Declarative RBAC/ABAC and per-record access controls with deny-by-default semantics.
3. Secret providers/rotation and first-class TLS/deployment integration.
4. Dependency auditing, fuzzing, adversarial integration tests and external security audit.
5. Migration away from the legacy generated JavaScript server, which is local-only and does not share the native Rust authorization middleware.

## Reporting security issues

Do not post working exploits or sensitive data in public issues. Contact the repository maintainer privately to report suspected vulnerabilities.
