#!/bin/sh
# Regenerates the oracle fixtures in this directory: each case directory holds the TeX/bib
# sources; this script compiles a copy of it in a temporary directory with the real toolchain
# and copies the resulting `.log` / `.blg` back next to the sources.
#
# Usage: crates/log/tests/data/generate.sh [case ...]   (default: all cases)
# Needs TeX Live (pdflatex, xelatex, lualatex, latexmk, bibtex); tectonic is optional.
# The home directory is replaced in the copied logs by a placeholder of the same length, so
# TeX's 79/80-column line wrapping is unchanged.
#
# Fixtures were produced with TeX Live 2022 (MacTeX) and Tectonic 0.16.9. When regenerating
# with another version, re-check the expectations in ../oracle.rs.

set -u
here=$(cd "$(dirname "$0")" && pwd)
PATH="/Library/TeX/texbin:/opt/homebrew/bin:$PATH"
export PATH

lmk="latexmk -synctex=1 -interaction=nonstopmode -file-line-error -f"

run_case() {
    case_name=$1
    shift
    src="$here/$case_name"
    work=$(mktemp -d)
    cp -R "$src/." "$work/"
    rm -f "$work"/*.log "$work"/*.blg
    (cd "$work" && "$@") >/dev/null 2>&1
    for f in "$work"/main.log "$work"/main.blg; do
        if [ -f "$f" ]; then
            perl -pe 's/\Q$ENV{HOME}\E/"\/home\/" . ("x" x (length($ENV{HOME}) - 6))/ge' "$f" >"$src/$(basename "$f")"
        fi
    done
    rm -rf "$work"
    echo "generated $case_name"
}

all="latexmk-pdf pdflatex-plain missing-package missing-package-plain no-document no-end \
xelatex xelatex-font lualatex biblatex bibtex-missing-db tectonic tectonic-halt"
[ $# -gt 0 ] && all="$*"

for c in $all; do
    case $c in
        latexmk-pdf) run_case "$c" $lmk -pdf main.tex ;;
        pdflatex-plain) run_case "$c" pdflatex -interaction=nonstopmode main.tex ;;
        missing-package) run_case "$c" pdflatex -interaction=nonstopmode -file-line-error main.tex ;;
        missing-package-plain) run_case "$c" pdflatex -interaction=nonstopmode main.tex ;;
        no-document) run_case "$c" pdflatex -interaction=nonstopmode main.tex ;;
        no-end) run_case "$c" pdflatex -interaction=nonstopmode -file-line-error main.tex ;;
        xelatex | xelatex-font) run_case "$c" $lmk -xelatex main.tex ;;
        lualatex) run_case "$c" $lmk -lualatex main.tex ;;
        # biber itself is not run here (the MacTeX 2022 biber binary is x86_64 only), so this
        # log shows biblatex asking for a biber run.
        biblatex) run_case "$c" $lmk -pdf main.tex ;;
        # latexmk skips bibtex when the database is missing, so run the steps by hand.
        bibtex-missing-db) run_case "$c" sh -c \
            'pdflatex -interaction=nonstopmode main.tex; bibtex main; pdflatex -interaction=nonstopmode main.tex' ;;
        # tectonic may need the network to fetch its bundle on first use.
        tectonic) run_case "$c" tectonic --keep-logs --synctex -Z continue-on-errors main.tex ;;
        tectonic-halt) run_case "$c" tectonic --keep-logs main.tex ;;
        *) echo "unknown case $c" >&2 ;;
    esac
done
