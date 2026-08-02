-- Options are automatically loaded before lazy.nvim startup
-- Default options that are always set: https://github.com/LazyVim/LazyVim/blob/main/lua/lazyvim/config/options.lua
-- Add any additional options here

-- Hybrid line numbers: absolute on the current line, relative everywhere else.
vim.opt.number = true
vim.opt.relativenumber = true

-- Stable gutter (never jumps when diagnostics/git signs appear), JetBrains-like.
vim.opt.signcolumn = "yes"
vim.opt.cursorline = true
vim.opt.scrolloff = 8

-- Pick up files Claude Code / external tools write on disk.
vim.opt.autoread = true

-- Rounded popups everywhere (Neovim 0.11+).
vim.o.winborder = "rounded"

-- Python LSP: basedpyright + native ruff server (runtime comes from mise).
vim.g.lazyvim_python_lsp = "basedpyright"
vim.g.lazyvim_python_ruff = "ruff"
