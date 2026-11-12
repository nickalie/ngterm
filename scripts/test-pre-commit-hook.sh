#!/usr/bin/env bash
# Tests .githooks/pre-commit in a throwaway Git repository holding a small Cargo workspace: a commit of unformatted
# staged Rust is refused, everything else goes through. Needs cargo and rustfmt on PATH.
#
# Usage: scripts/test-pre-commit-hook.sh
set -euo pipefail

hook="$(cd "$(dirname "$0")/.." && pwd)/.githooks/pre-commit"
repo="$(mktemp -d)"
trap 'rm -rf "$repo"' EXIT
errors="$repo/.git/commit.err"
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid GIT_COMMITTER_NAME=test
export GIT_COMMITTER_EMAIL=test@example.invalid

failures=0
pass() { printf 'ok   %s\n' "$1"; }
flunk() {
  printf 'FAIL %s\n' "$1" >&2
  failures=$((failures + 1))
}
commit() { git -C "$repo" commit --quiet -m test 2>"$errors"; }
expect_commit() {
  if commit; then pass "$1"; else flunk "$1: $(cat "$errors")"; fi
}
expect_refusal() {
  if commit; then
    flunk "$1: the commit went through"
  elif ! grep -qF "$2" "$errors" || ! grep -qF 'cargo fmt --all' "$errors"; then
    flunk "$1: the refusal doesn't name $2 and the fix: $(cat "$errors")"
  else
    pass "$1"
  fi
  git -C "$repo" reset --quiet
}

cd "$repo"
git init --quiet
git config core.hooksPath .githooks
mkdir -p .githooks crates/a/src vendor/v/src
cp "$hook" .githooks/pre-commit
printf '[workspace]\nresolver = "3"\nmembers = ["crates/*"]\nexclude = ["vendor"]\n' >Cargo.toml
printf 'edition = "2024"\n' >rustfmt.toml
printf '[package]\nname = "a"\nversion = "0.1.0"\nedition = "2024"\n' >crates/a/Cargo.toml
printf '[package]\nname = "v"\nversion = "0.1.0"\nedition = "2024"\n' >vendor/v/Cargo.toml
printf 'pub fn a() {}\n' >crates/a/src/lib.rs
printf 'pub fn e() {}\n' >crates/a/src/extra.rs
printf 'pub fn v() {}\n' >vendor/v/src/lib.rs
git add -A
expect_commit "formatted Rust commits"

printf 'pub fn  a( ) {}\n' >crates/a/src/lib.rs
git add crates/a/src/lib.rs
expect_refusal "unformatted staged Rust is refused" crates/a/src/lib.rs
if grep -qF 'git add crates/a/src/lib.rs' "$errors"; then
  pass "a fully staged file's fix stages it"
else
  flunk "a fully staged file's fix doesn't stage it: $(cat "$errors")"
fi

printf 'pub fn a() {}' >crates/a/src/lib.rs
git add crates/a/src/lib.rs
expect_refusal "a missing final newline is refused" crates/a/src/lib.rs

printf 'pub fn a() {}\npub fn b() {}\n' >crates/a/src/lib.rs
git add crates/a/src/lib.rs
printf 'pub fn a() {}\npub fn  b( ) {}\n' >crates/a/src/lib.rs
expect_commit "only the staged contents count, not the working tree"

printf 'pub fn a() {}\npub fn  c( ) {}\n' >crates/a/src/lib.rs
git add crates/a/src/lib.rs
printf 'pub fn a() {}\npub fn c() {}\n' >crates/a/src/lib.rs
expect_refusal "unformatted staged contents are refused under a formatted working tree" crates/a/src/lib.rs
if grep -qF 'git add -p' "$errors" && ! grep -qF 'git add crates' "$errors"; then
  pass "a partially staged file's fix doesn't stage its unstaged edits"
else
  flunk "a partially staged file's fix stages the whole file: $(cat "$errors")"
fi
git checkout --quiet -- crates/a/src/lib.rs

printf 'mod missing;\npub fn  d( ) {}\n' >crates/a/src/lib.rs
git add crates/a/src/lib.rs
expect_refusal "a file naming a module that doesn't exist yet is still checked" crates/a/src/lib.rs
git checkout --quiet -- crates/a/src/lib.rs

printf 'pub fn  v( ) {}\n' >vendor/v/src/lib.rs
git add vendor/v/src/lib.rs
expect_commit "Rust outside the workspace's packages is not checked"

printf 'not rust\n' >notes.txt
git add notes.txt
expect_commit "a commit without Rust is not checked"

git rm --quiet crates/a/src/extra.rs
expect_commit "a deleted Rust file is not checked"

((failures == 0)) || {
  printf '%d pre-commit hook test(s) failed\n' "$failures" >&2
  exit 1
}
