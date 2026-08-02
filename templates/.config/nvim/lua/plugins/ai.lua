-- Claude Code bridge (the core).
-- claudecode.nvim comes from the LazyVim `ai.claudecode` extra (keymaps under
-- <leader>a*). This tunes opts AND adds `cmd` so the :ClaudeCode* commands are
-- registered directly — the extra only lazy-loads on a <leader>a keypress, so
-- typing `:ClaudeCode` cold otherwise throws E492 until a key has been pressed.
--
-- Claude lives in a right-side split; diffs open vertically with the blocking
-- accept/reject gate; the WebSocket IDE server auto-starts so `claude`
-- (launched from inside this nvim) auto-connects. Verify with :ClaudeCodeStatus.
--
-- NOTE: ai.sidekick is intentionally NOT enabled — its keymaps collide with
-- claudecode (<leader>aa/as/ad/af) and its Next-Edit-Suggestions feature needs
-- the Copilot LSP, which we don't use.
return {
  {
    "coder/claudecode.nvim",
    cmd = {
      "ClaudeCode",
      "ClaudeCodeFocus",
      "ClaudeCodeOpen",
      "ClaudeCodeClose",
      "ClaudeCodeSend",
      "ClaudeCodeAdd",
      "ClaudeCodeTreeAdd",
      "ClaudeCodeDiffAccept",
      "ClaudeCodeDiffDeny",
      "ClaudeCodeStatus",
      "ClaudeCodeStart",
      "ClaudeCodeStop",
      "ClaudeCodeSelectModel",
    },
    opts = {
      auto_start = true,
      terminal = {
        split_side = "left",
        split_width_percentage = 0.40,
        provider = "snacks",
      },
      diff_opts = {
        layout = "vertical", -- canonical (vertical_split is a legacy alias)
        open_in_new_tab = false,
      },
    },
  },
}
