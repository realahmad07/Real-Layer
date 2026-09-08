#!/bin/sh
set -eu

: "${GHOST_LISTEN_ADDRESS:?GHOST_LISTEN_ADDRESS is required}"
: "${GHOST_ADVERTISED_ADDRESS:?GHOST_ADVERTISED_ADDRESS is required}"
: "${GHOST_IDENTITY_PATH:?GHOST_IDENTITY_PATH is required}"

exec /usr/local/bin/ghost-layer-relay
