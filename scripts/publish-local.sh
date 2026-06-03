#!/usr/bin/env sh
# Build the server module and publish it to the LOCAL SpacetimeDB instance.
# Start the instance first (scripts/start-local.sh) in another terminal.
#
# Usage: sh scripts/publish-local.sh [db-name]   (default: space4x)
set -eu
DB_NAME="${1:-space4x}"

spacetime publish "$DB_NAME" --module-path server --server local --yes

echo
echo "Published '$DB_NAME' to the local server."
echo "Inspect it with, e.g.:"
echo "  spacetime sql  $DB_NAME \"SELECT * FROM star_system\""
echo "  spacetime call $DB_NAME advance_days 1   # process one day = TICKS_PER_DAY ticks"
echo "  spacetime sql  $DB_NAME \"SELECT run_id, requested_ticks, from_tick, to_tick FROM sim_run\""
echo "  spacetime logs $DB_NAME"
