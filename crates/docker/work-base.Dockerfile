# work-base: default isolated workspace image. Brings-your-own tools/logins.
FROM node:20-bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      git openssh-client ca-certificates tmux zsh bash curl jq build-essential sudo \
      ncurses-term locales \
 && rm -rf /var/lib/apt/lists/* \
 && localedef -i en_US -c -f UTF-8 en_US.UTF-8

# Terminal + locale defaults. `docker exec` does not propagate the host
# environment, so TUI agents (omp, Claude Code, …) and nested tmux need these
# baked into the image: a real TERM plus its terminfo (ncurses-term) and a
# UTF-8 locale. Without them, curses-based tools can't find a terminal
# description and render escape codes as literal text.
ENV TERM=xterm-256color \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8

RUN useradd -m -d /home/dev -s /usr/bin/zsh dev \
 && echo 'dev ALL=(ALL) NOPASSWD:ALL' >> /etc/sudoers

USER dev
WORKDIR /home/dev
CMD ["sleep", "infinity"]
