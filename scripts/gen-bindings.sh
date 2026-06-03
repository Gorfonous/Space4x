#!/usr/bin/env sh
# Generate Rust client bindings from the server module into the client crate.
# Run after changing the schema or reducers so the client stays in sync.
set -eu
spacetime generate --lang rust --out-dir client/src/module_bindings --module-path server
echo "Generated client bindings into client/src/module_bindings"
