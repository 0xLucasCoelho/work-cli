# Default company image. Tools belong here — the runtime box is cap-dropped
# and has no passwordless sudo.
FROM debian:trixie-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash \
        ca-certificates \
        curl \
        git \
        jq \
        openssh-client \
        zsh \
    && useradd --create-home --uid 1000 --shell /usr/bin/zsh dev \
    && mkdir -p /home/dev/src /tmp/runtime-dev \
    && chown -R dev:dev /home/dev /tmp/runtime-dev \
    && rm -rf /var/lib/apt/lists/*

USER dev
ENV HOME=/home/dev \
    SHELL=/usr/bin/zsh \
    XDG_CONFIG_HOME=/home/dev/.config \
    XDG_DATA_HOME=/home/dev/.local/share \
    XDG_STATE_HOME=/home/dev/.local/state \
    XDG_CACHE_HOME=/home/dev/.cache \
    XDG_RUNTIME_DIR=/tmp/runtime-dev
WORKDIR /home/dev
