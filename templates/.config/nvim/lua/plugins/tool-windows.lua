-- JetBrains "tool window" layout:
--   Project tree (neo-tree)  -> RIGHT
--   Structure (aerial)       -> LEFT
-- Both plugins come from LazyVim extras (editor.neo-tree, editor.aerial);
-- these specs only override their layout opts and merge by repo name.
return {
  {
    "nvim-neo-tree/neo-tree.nvim",
    opts = {
      window = { position = "right", width = 34 },
      filesystem = {
        follow_current_file = { enabled = true },
        use_libuv_file_watcher = true,
      },
    },
  },
  {
    "stevearc/aerial.nvim",
    opts = {
      layout = { default_direction = "left" },
    },
  },
}
