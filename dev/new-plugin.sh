#!/bin/sh
# Creates a new ConsoleUI plugin from templates/plugin (Story 9.4).
#   dev/new-plugin.sh <name> [directory]     name in kebab-case, directory defaults to ../<name>
# The template is CC0: the plugin may use any license. It needs no ConsoleUI crate, only zbus.
set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
name="${1:-}"
case "$name" in
    "" | *[!a-z0-9-]* | -*)
        echo "Usage: dev/new-plugin.sh <name-in-kebab-case> [directory]" >&2
        exit 2
        ;;
esac
target="${2:-$root/../$name}"
if [ -e "$target" ]; then
    echo "$target already exists" >&2
    exit 1
fi

# my-cool-plugin -> My Cool Plugin, ID org.example.MyCoolPlugin (to be changed to the author's own)
title="$(printf '%s' "$name" | tr '-' ' ' | awk '{ for (i = 1; i <= NF; i++) $i = toupper(substr($i, 1, 1)) substr($i, 2); print }')"
app_id="org.example.$(printf '%s' "$title" | tr -d ' ')"

cp -r "$root/templates/plugin" "$target"
mv "$target/my-plugin.desktop" "$target/$name.desktop"
mv "$target/flatpak.yml" "$target/$app_id.yml"
find "$target" -type f -exec sed -i \
    -e "s|@APP_ID@|$app_id|g" -e "s|my-plugin|$name|g" -e "s|My Plugin|$title|g" {} +
echo "Created $title in $target (bus name $app_id)"
echo "Run it with: cd $target && cargo run"
