# The Linux workspace for Cryzo cloud projects: Node 22, git, tmux for the terminal, and the
# sandbox-shim helper that @cloudflare/sandbox's Files API runs. Keep the shim tag equal to the
# installed @cloudflare/sandbox version.
FROM docker.io/cloudflare/sandbox:1.0.3 AS shim

FROM docker.io/library/node:22-bookworm
RUN apt-get update \
  && apt-get install -y --no-install-recommends git tmux ca-certificates curl python3 build-essential \
  && rm -rf /var/lib/apt/lists/*
COPY --from=shim /usr/local/bin/sandbox-shim /usr/local/bin/sandbox-shim
RUN mkdir -p /workspace
WORKDIR /workspace
CMD ["sleep", "infinity"]
