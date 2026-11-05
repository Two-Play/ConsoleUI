#!/bin/sh
# Creates a new ConsoleUI app from templates/app (Story 8.3).
#   dev/new-app.sh <name> [directory]     name in kebab-case, directory defaults to ../<name>
# The app finds the SDK through a path dependency on this checkout (the crates are not published yet).
set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
name="${1:-}"
case "$name" in
    "" | *[!a-z0-9-]* | -*)
        echo "Usage: dev/new-app.sh <name-in-kebab-case> [directory]" >&2
        exit 2
        ;;
esac
target="${2:-$root/../$name}"
if [ -e "$target" ]; then
    echo "$target already exists" >&2
    exit 1
fi

# my-cool-app -> My Cool App
title="$(printf '%s' "$name" | tr '-' ' ' | awk '{ for (i = 1; i <= NF; i++) $i = toupper(substr($i, 1, 1)) substr($i, 2); print }')"

cp -r "$root/templates/app" "$target"
mv "$target/my-app.desktop" "$target/$name.desktop"
find "$target" -type f -exec sed -i \
    -e "s|@SDK_PATH@|$root/crates/sdk|g" -e "s|my-app|$name|g" -e "s|My App|$title|g" {} +
echo "Created $title in $target"
echo "Run it with: cd $target && cargo run"
