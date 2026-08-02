return {
  -- 0. JetBrains-style live HTML preview in the browser (mermaid, KaTeX, scroll-sync)
  {
    "iamcco/markdown-preview.nvim",
    cmd = { "MarkdownPreviewToggle", "MarkdownPreview", "MarkdownPreviewStop" },
    ft = { "markdown" },
    -- prebuilt binary download (no yarn/npm app build needed)
    build = function()
      vim.fn["mkdp#util#install"]()
    end,
    keys = {
      { "<leader>mp", "<cmd>MarkdownPreviewToggle<cr>", ft = "markdown", desc = "Markdown Preview (browser)" },
    },
    init = function()
      vim.g.mkdp_filetypes = { "markdown" }
      vim.g.mkdp_theme = "dark"
      vim.g.mkdp_auto_close = 0 -- keep the preview open when switching buffers
    end,
  },

  -- 1. Markdown styling (headings, tables, callouts, code blocks, LaTeX)
  {
    "MeanderingProgrammer/render-markdown.nvim",
    dependencies = { "nvim-treesitter/nvim-treesitter", "nvim-tree/nvim-web-devicons" },
    ft = { "markdown" },
    opts = {
      code = {
        sign = false,
        width = "block",
        right_pad = 1,
      },
      heading = {
        sign = false,
        icons = {},
      },
    },
  },

  -- 2. Image rendering backend
  {
    "3rd/image.nvim",
    build = false, -- set to "luarocks --local --lua-version=5.1 install magick" if needed
    ft = { "markdown" },
    opts = {
      backend = "kitty", -- Ghostty speaks this
      processor = "magick_cli", -- avoids the magick rock dependency
      integrations = {
        markdown = {
          enabled = true,
          clear_in_insert_mode = false,
          download_remote_images = true,
          only_render_image_at_cursor = false,
        },
      },
      max_width = 100,
      max_height = 12,
      max_height_window_percentage = 30,
      window_overlap_clear_enabled = true,
      window_overlap_clear_ft_ignore = { "cmp_menu", "cmp_docs", "snacks_notif" },
    },
  },

  -- 3. Mermaid (+ plantuml, d2) renderer
  {
    "3rd/diagram.nvim",
    dependencies = { "3rd/image.nvim" },
    ft = { "markdown" },
    -- opts as a function so the require runs at plugin-load time (rtp ready),
    -- not while lazy.nvim is parsing this spec file at startup.
    opts = function()
      return {
        integrations = {
          require("diagram.integrations.markdown"),
        },
        renderer_options = {
          mermaid = {
            background = "transparent",
            theme = "dark", -- "default" | "dark" | "forest" | "neutral"
            scale = 2, -- bump for higher DPI
          },
        },
      }
    end,
  },
}
