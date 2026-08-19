# Bundled developer shell configuration. Host Fish config is opt-in via
# `work new --shell fish --import-shell-config`.
if type -q starship
    starship init fish | source
end
if type -q zoxide
    zoxide init fish | source
end
if type -q mise
    mise activate fish | source
end
if type -q eza
    alias ls 'eza -lah --git --icons'
end
set -gx PATH $HOME/.local/bin /usr/local/bin $PATH

function fish_prompt
    set_color magenta; echo -n '⬡ '
    set_color cyan; echo -n "$WORK "
    set_color blue; echo -n (prompt_pwd)
    set_color normal; echo -n '> '
end
