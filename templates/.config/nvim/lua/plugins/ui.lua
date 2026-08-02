-- JetBrains-feel polish, all Monokai Pro-coherent.
return {
  -- Statusline themed to Monokai Pro, single global bar.
  {
    "nvim-lualine/lualine.nvim",
    opts = function(_, opts)
      opts.options = opts.options or {}
      opts.options.theme = "monokai-pro"
      opts.options.globalstatus = true
      opts.options.section_separators = ""
      opts.options.component_separators = ""
    end,
  },

  -- Editor tabs that read like JetBrains tabs.
  {
    "akinsho/bufferline.nvim",
    opts = {
      options = {
        separator_style = "slant",
        diagnostics = "nvim_lsp",
        show_buffer_close_icons = true,
      },
    },
  },

  -- Smooth scroll + indent guides + active scope (polished IDE motion).
  {
    "folke/snacks.nvim",
    opts = {
      scroll = { enabled = true },
      indent = { enabled = true },
      scope = { enabled = true },
      input = { enabled = true },
      notifier = { enabled = true },
      dashboard = { enabled = true },
    },
  },
}
