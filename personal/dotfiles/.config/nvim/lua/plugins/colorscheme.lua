-- Monokai Pro ("pro" filter) — the palette Zedokai derives from.
-- Opaque background to keep the exact Monokai Pro surface (#2D2A2E).
return {
  {
    "loctvl842/monokai-pro.nvim",
    lazy = false,
    priority = 1000,
    opts = {
      filter = "pro",
      transparent_background = false,
      background_clear = { "float_win", "toggleterm", "telescope", "which-key", "notify" },
      styles = {
        comment = { italic = true },
        keyword = { italic = true },
      },
    },
  },
  {
    "LazyVim/LazyVim",
    opts = {
      colorscheme = "monokai-pro",
    },
  },
}
