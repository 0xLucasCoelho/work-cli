# zinit
source "${HOME}/.local/share/zinit/zinit.git/zinit.zsh"

# zinit annexes (installer added these, keep them)
zinit light-mode for \
    zdharma-continuum/zinit-annex-as-monitor \
    zdharma-continuum/zinit-annex-bin-gem-node \
    zdharma-continuum/zinit-annex-patch-dl \
    zdharma-continuum/zinit-annex-rust

# plugins
zinit light Aloxaf/fzf-tab
zinit light zsh-users/zsh-autosuggestions
zinit light zdharma-continuum/fast-syntax-highlighting

# ensure compinit ran (zinit usually handles this, but be explicit)
autoload -Uz compinit && compinit

# tools — order matters: prompt → history → navigation
eval "$(starship init zsh)"
eval "$(atuin init zsh)"
eval "$(zoxide init zsh --cmd cd)"

alias ls='eza -lah --git --icons'
alias z='zed'

export PATH="$HOME/.local/bin:$PATH"
export NVM_DIR="$HOME/.nvm"
[ -s "/opt/homebrew/opt/nvm/nvm.sh" ] && \. "/opt/homebrew/opt/nvm/nvm.sh"  # This loads nvm
[ -s "/opt/homebrew/opt/nvm/etc/bash_completion.d/nvm" ] && \. "/opt/homebrew/opt/nvm/etc/bash_completion.d/nvm"  # This loads nvm bash_completion
eval "$(mise activate zsh)"
