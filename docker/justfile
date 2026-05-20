default:
    @just --list

set dotenv-load := true

# Update container images
update:
    podman compose -f compose.yaml up -d --build

# Start service
up *args:
    podman compose -f compose.yaml up -d {{args}}

# Stop service
down *args:
    podman compose -f compose.yaml down {{args}}

# Restart service
restart *args:
    podman compose -f compose.yaml restart {{args}}

# View logs
logs *args:
    podman compose -f compose.yaml logs -f {{args}}
