#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

command -v mdbook >/dev/null 2>&1 || {
    echo "mdbook is required; install it with: cargo install mdbook --version 0.5.4 --locked" >&2
    exit 1
}

test "$(mdbook --version)" = "mdbook v0.5.4" || {
    echo "mdbook 0.5.4 is required for reproducible documentation builds" >&2
    exit 1
}

mdbook build "$repo_root/docs/user-guide"
ruby "$repo_root/build-aux/check-doc-site.rb" "$repo_root/target/docs-site"
