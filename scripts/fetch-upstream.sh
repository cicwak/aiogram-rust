#!/usr/bin/env sh
set -eu

repository="https://github.com/aiogram/aiogram.git"
commit="c1b0353ce3d3f8d70f90469038939a956e9e09f7"
destination="${1:-aiogram}"

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
