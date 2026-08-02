-- Keymaps are automatically loaded on the VeryLazy event
-- Default keymaps that are always set: https://github.com/LazyVim/LazyVim/blob/main/lua/lazyvim/config/keymaps.lua
-- Add any additional keymaps here

vim.keymap.set("n", "gr", vim.lsp.buf.references, { desc = "LSP: References", nowait = true })

-- JetBrains Cmd+G "go to line" feel. (Native `:42` + Enter and `42G` still work.)
vim.keymap.set("n", "<leader>gl", function()
  vim.ui.input({ prompt = "Go to line: " }, function(v)
    if v and v:match("^%d+$") then
      vim.cmd("normal! " .. v .. "G")
    end
  end)
end, { desc = "Go to line" })

-- Git worktree switcher (native, snacks-rendered via vim.ui.select; no telescope).
-- Pairs with `claude --worktree <name>` and sesh for the multi-agent flow.
vim.keymap.set("n", "<leader>gw", function()
  local out = vim.fn.systemlist({ "git", "worktree", "list", "--porcelain" })
  if vim.v.shell_error ~= 0 then
    vim.notify("Not a git repo (or no worktrees)", vim.log.levels.WARN)
    return
  end
  local trees = {}
  for _, line in ipairs(out) do
    local path = line:match("^worktree (.+)$")
    if path then
      trees[#trees + 1] = path
    end
  end
  if #trees == 0 then
    vim.notify("No worktrees found", vim.log.levels.INFO)
    return
  end
  vim.ui.select(trees, { prompt = "Switch to worktree:" }, function(choice)
    if not choice then
      return
    end
    vim.cmd("tcd " .. vim.fn.fnameescape(choice))
    vim.notify("cwd → " .. choice)
    pcall(function()
      require("neo-tree.sources.manager").refresh("filesystem")
    end)
  end)
end, { desc = "Git: switch worktree" })
