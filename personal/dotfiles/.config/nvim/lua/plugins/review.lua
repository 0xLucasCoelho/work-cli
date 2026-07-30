-- PR review & git — the #1 workflow.
--
-- octo base setup + its <leader>g* keymaps come from the LazyVim `util.octo`
-- extra (gp = PR list, gi = issues, ...). This file only overrides octo opts
-- and adds a dedicated `<leader>r` "Review" group as the JetBrains-flavored
-- entry points. We do NOT remap octo under <leader>o — that's the overseer group.
return {
  -- Diffview: changed-files panel + side-by-side diff with FULL LSP on both sides.
  {
    "sindrets/diffview.nvim",
    cmd = { "DiffviewOpen", "DiffviewClose", "DiffviewFileHistory", "DiffviewToggleFiles", "DiffviewFocusFiles" },
    keys = {
      -- generic diffview
      { "<leader>gd", "<cmd>DiffviewOpen<cr>", desc = "Diffview: open (working tree)" },
      { "<leader>gD", "<cmd>DiffviewClose<cr>", desc = "Diffview: close" },
      { "<leader>gv", "<cmd>DiffviewFileHistory %<cr>", desc = "Diffview: current file history" },
      { "<leader>gV", "<cmd>DiffviewFileHistory<cr>", desc = "Diffview: branch history" },
      -- REVIEW group: review your own / Claude's work, full LSP on both sides
      { "<leader>rd", "<cmd>DiffviewOpen origin/main...HEAD<cr>", desc = "Review: diff vs origin/main" },
      { "<leader>rD", "<cmd>DiffviewOpen origin/master...HEAD<cr>", desc = "Review: diff vs origin/master" },
      { "<leader>rl", "<cmd>DiffviewFileHistory %<cr>", desc = "Review: current file history" },
      { "<leader>rq", "<cmd>DiffviewClose<cr>", desc = "Review: close" },
    },
  },

  -- octo opts override (base setup + <leader>g* keys come from util.octo extra).
  {
    "pwntester/octo.nvim",
    opts = {
      use_local_fs = true,
      default_to_projects_v2 = false,
    },
    keys = {
      { "<leader>rp", "<cmd>Octo pr list<cr>", desc = "Review: GitHub PRs" },
    },
  },

  -- gitsigns: inline current-line blame, JetBrains-style.
  {
    "lewis6991/gitsigns.nvim",
    opts = {
      current_line_blame = true,
      current_line_blame_opts = { virt_text_pos = "eol", delay = 300 },
    },
  },

  -- Register the Review group label for which-key discoverability.
  {
    "folke/which-key.nvim",
    opts = {
      spec = {
        { "<leader>r", group = "review" },
      },
    },
  },
}
