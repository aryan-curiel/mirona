#!/bin/sh
# Builds the docs site into target/book (NGR-176): the Markdown from the repo, laid out as on
# GitHub so relative links keep working, plus the Rust API docs under api/.
# Needs mdbook (https://github.com/rust-lang/mdBook/releases) on PATH, or MDBOOK=/path/to/mdbook.
set -eu
cd "$(dirname "$0")/.."
src=target/book-src
rm -rf "$src" target/book
mkdir -p "$src"
cp README.md CONTRIBUTING.md PRIVACY.md SECURITY.md CODE_OF_CONDUCT.md LICENSE NOTICE "$src/"
cp -R docs "$src/docs"
cp book/* "$src/"
# mdBook serves README.md as index.html; point links at that name (the copy only).
find "$src" -name '*.md' ! -name SUMMARY.md -exec sed -i.bak -E 's#\]\(((\.\./)*)README\.md#](\1index.md#g' {} +
find "$src" -name '*.bak' -delete
"${MDBOOK:-mdbook}" build "$src" -d "$PWD/target/book"
cargo doc --workspace --no-deps
cp -R target/doc target/book/api
echo "docs in target/book (open target/book/index.html)"
