# work-base: default isolated workspace image. Brings-your-own tools/logins.
FROM debian:trixie-slim@sha256:3a39a0592364683e6bab97937b72cad5a8fa6dcbbee90edb3bb48c7f8e94f258

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      git openssh-client ca-certificates zsh bash curl jq build-essential sudo \
      ncurses-term locales \
 && rm -rf /var/lib/apt/lists/* \
 && localedef -i en_US -c -f UTF-8 en_US.UTF-8

# herdr: the in-container multiplexer (replaces tmux). Statically-linked ELF
# (no glibc dependency), so fetch the pinned release for this build's arch.
ARG HERDR_VERSION=0.8.0
RUN case "$(uname -m)" in \
      aarch64|arm64) HERDR_ARCH=aarch64 ;; \
      x86_64|amd64)  HERDR_ARCH=x86_64 ;; \
      *) echo "unsupported arch: $(uname -m)" >&2; exit 1 ;; \
    esac && \
    curl -fsSL -o /usr/local/bin/herdr \
      "https://github.com/herdrdev/herdr/releases/download/v${HERDR_VERSION}/herdr-linux-${HERDR_ARCH}" && \
    chmod +x /usr/local/bin/herdr

# Terminal + locale defaults. `docker exec` does not propagate the host
# environment, so TUI agents (omp, Claude Code, …) and the in-container
# multiplexer (herdr) need these baked into the image: a real TERM plus its
# terminfo (ncurses-term) and a UTF-8 locale. Without them, curses-based tools
# can't find a terminal description and render escape codes as literal text.
ENV TERM=xterm-256color \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8

RUN useradd -m -d /home/dev -s /usr/bin/zsh dev \
 && echo 'dev ALL=(ALL) NOPASSWD:ALL' >> /etc/sudoers

USER dev
WORKDIR /home/dev
CMD ["sleep", "infinity"]
