#!/bin/sh
# Points git at the repository's hooks (scripts/hooks): a pre-commit secret scan with gitleaks.
# Run once per clone:  scripts/install-hooks.sh
set -eu
cd "$(dirname "$0")/.."
chmod +x scripts/hooks/*
git config core.hooksPath scripts/hooks
echo "Git hooks installed (core.hooksPath = scripts/hooks)."
if ! command -v gitleaks >/dev/null 2>&1; then
  echo "Now install gitleaks, or commits will be refused:" >&2
  echo "  macOS: brew install gitleaks    Windows: winget install Gitleaks.Gitleaks" >&2
fi
