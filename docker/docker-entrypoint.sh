#!/bin/sh

# Exit immediately if a command exits with a non-zero status
set -e

# Production dependencies are baked into the image at build time (see Dockerfile),
# so no `pnpm install` is needed here — this makes container restarts fast.

# Run database migrations before starting the application to prevent
# race conditions where the app starts before the database is ready.
pnpm db:migrate

# Execute the main container command
exec "$@"
