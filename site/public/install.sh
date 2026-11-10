#!/bin/sh
# https://dzulfikar08.github.io/irit/install.sh: runs the irit installer from the main branch, so
# this copy never goes stale. Same options as scripts/install.sh in the repository:
#   curl -fsSL https://dzulfikar08.github.io/irit/install.sh | sh
#   curl -fsSL https://dzulfikar08.github.io/irit/install.sh | sh -s -- --no-gui
#   curl -fsSL https://dzulfikar08.github.io/irit/install.sh | sh -s -- --uninstall
set -eu
script="$(curl -fsSL https://raw.githubusercontent.com/dzulfikar08/irit/main/scripts/install.sh)"
printf '%s\n' "$script" | sh -s -- "$@"
