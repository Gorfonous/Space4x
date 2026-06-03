#!/usr/bin/env sh
# Start a local SpacetimeDB instance (listens on 127.0.0.1:3000 by default).
# Leave this running in its own terminal, then publish with publish-local.sh.
#
# Requires the `spacetime` CLI on PATH: https://install.spacetimedb.com
set -eu
exec spacetime start "$@"
