# Security

## Reporting a vulnerability

Please report security problems privately, not in a public issue: use **Report a vulnerability**
on the repository's Security tab (GitHub private vulnerability reporting). Describe what is
affected, how to reproduce it and what an attacker could do. You will get an answer as soon as
possible; fixes are released as a new version, which the app's updater offers.

## What TexCraft protects

- **AI provider API keys**: AES-256-GCM encrypted with a random master key, in owner-only files
  in the app's data folder; decrypted only for a request, passed to `curl` on standard input
  (never on the command line), never shown again, written to logs or reported to agents
  (`crates/ai`).
- **The control channel** (for agents and tests): off unless started with `--control`, bound to
  127.0.0.1, and every connection must present a 256-bit token from an owner-only file.
- **Updates**: downloaded over HTTPS from this repository's releases and checked against the
  SHA-256 in `latest.json` before they are installed.
- **TeX**: documents are typeset by the TeX distribution you installed. Unrestricted shell
  escape (running any program from a document) is off unless you turn it on in Settings ›
  Typesetting; TeX's own restricted list of safe programs (`texmf.cnf`) still applies.

## How the repository is kept clean

- A gitleaks pre-commit hook (`scripts/install-hooks.sh`, `.gitleaks.toml`) refuses secrets,
  private files, absolute home-folder paths and files over 5 MB; CI repeats the scan on every
  push to `main` and every pull request (`.github/workflows/checks.yml` runs `secrets.yml`).
- cargo-deny (`deny.toml`, `.github/workflows/audit.yml`) fails on dependencies with known
  vulnerabilities (RustSec), non-permissive licences or sources other than crates.io, in the same
  `checks.yml` runs and weekly; Dependabot proposes updates.
- Releases are built only after both checks pass, with pinned actions, no build cache and
  write access only for the publishing step (`.github/workflows/release.yml`).
