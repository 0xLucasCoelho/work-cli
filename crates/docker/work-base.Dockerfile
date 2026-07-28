# work-base: default isolated workspace image. Brings-your-own tools/logins.
FROM node:20-bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      git openssh-client ca-certificates tmux zsh curl jq build-essential sudo \
 && rm -rf /var/lib/apt/lists/*

RUN useradd -m -d /home/dev -s /usr/bin/zsh dev \
 && echo 'dev ALL=(ALL) NOPASSWD:ALL' >> /etc/sudoers

USER dev
WORKDIR /home/dev
CMD ["sleep", "infinity"]
