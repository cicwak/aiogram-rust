#!/usr/bin/env sh
set -eu

root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$root"

destination="${1:-aiogram}"
repository="$(python3 -c 'import tomllib; print(tomllib.load(open("compatibility.toml", "rb"))["upstream"]["aiogram"]["repository"])')"
commit="$(python3 -c 'import tomllib; print(tomllib.load(open("compatibility.toml", "rb"))["upstream"]["aiogram"]["commit"])')"

if [ -e "$destination" ]; then
    if [ -d "$destination/.git" ] && [ "$(git -C "$destination" rev-parse HEAD)" = "$commit" ]; then
        git -C "$destination" show -s --format='Using existing aiogram %H (%s)'
        exit 0
    fi
    echo "Refusing to overwrite non-matching path: $destination" >&2
    exit 1
fi

git clone --filter=blob:none "$repository" "$destination"
git -C "$destination" checkout --detach "$commit"
git -C "$destination" show -s --format='Checked out aiogram %H (%s)'
