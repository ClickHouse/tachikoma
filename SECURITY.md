# Security Policy

## Reporting a Vulnerability

If you find a security issue in tachikoma, please report it privately rather than opening a public issue.

**Email:** security@clickhouse.com

Include:
- Description of the vulnerability
- Steps to reproduce
- Impact assessment (what an attacker could do)

We'll acknowledge your report within 3 business days and aim to release a fix within 14 days for confirmed issues.

## Scope

Tachikoma runs VMs on your local machine. The main security boundaries are:

- **VM isolation** — VMs should not be able to access host resources beyond what's explicitly mounted
- **Credential handling** — API keys and tokens should not leak into VMs when the credential proxy is enabled
- **Config injection** — repo-level config (`.tachikoma.toml`) should not be able to escape its sandbox (path traversal, shell injection)

## Out of Scope

- Docker socket forwarding (`examples/docker-bridge-start.sh`) is documented as granting host root access. That's by design, not a vulnerability.
- The credential proxy binds to the Tart vmnet bridge (`192.168.64.1`) by default. Other VMs on the same subnet can reach it. This is documented.
