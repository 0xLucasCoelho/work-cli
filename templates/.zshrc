# zinit
if [[ -r "${HOME}/.local/share/zinit/zinit.git/zinit.zsh" ]]; then
source "${HOME}/.local/share/zinit/zinit.git/zinit.zsh"

# zinit annexes — pinned to specific commits (same threat model as the plugins
# below: zdharma-continuum code runs at every shell start inside a container
# that may hold customer source). Bump via:
#   gh api repos/zdharma-continuum/<repo>/commits/<default-branch> --jq .sha
zinit ice ver="e1f9b7274f4df3d15c92d46c49f52ce58f947766"
zinit light zdharma-continuum/zinit-annex-as-monitor
zinit ice ver="89739902eafacb3c9c389f896177fb25e8a95f38"
zinit light zdharma-continuum/zinit-annex-bin-gem-node
zinit ice ver="ddb174be3aa308e7428691216f09aa69e8e2f94f"
zinit light zdharma-continuum/zinit-annex-patch-dl
zinit ice ver="c747bf9c5a2b85347238fa433c0addcfc7745c6e"
zinit light zdharma-continuum/zinit-annex-rust

# plugins — pinned to specific commits (zinit ice ver applies to the next
# `zinit light`). Bump by resolving the new SHA on each repo's default branch:
#   gh api repos/<owner>/<repo>/commits/<default-branch> --jq .sha
zinit ice ver="24105b15714bfec37989ed5c5b6e60f572253019"
zinit light Aloxaf/fzf-tab
zinit ice ver="85919cd1ffa7d2d5412f6d3fe437ebdbeeec4fc5"
zinit light zsh-users/zsh-autosuggestions
zinit ice ver="3d574ccf48804b10dca52625df13da5edae7f553"
zinit light zdharma-continuum/fast-syntax-highlighting

# ensure compinit ran (zinit usually handles this, but be explicit)
autoload -Uz compinit && compinit
fi

# tools — order matters: prompt → history → navigation
command -v starship >/dev/null 2>&1 && eval "$(starship init zsh)"
command -v atuin >/dev/null 2>&1 && eval "$(atuin init zsh)"
command -v zoxide >/dev/null 2>&1 && eval "$(zoxide init zsh --cmd cd)"

command -v eza >/dev/null 2>&1 && alias ls='eza -lah --git --icons'
command -v zed >/dev/null 2>&1 && alias z='zed'

export PATH="$HOME/.local/bin:$PATH"
export NVM_DIR="$HOME/.nvm"
if command -v mise >/dev/null 2>&1; then
  eval "$(mise activate zsh)"
fi
