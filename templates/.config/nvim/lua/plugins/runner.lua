-- Test / run / debug runner.
-- neotest (test.core extra) + python (lang.python) + rust (rustaceanvim) adapters
-- already wired. This adds JS/TS adapters and JetBrains-style F-key debug binds.
return {
  -- JS/TS test adapters (Rust + Python come from their lang extras).
  {
    "nvim-neotest/neotest",
    optional = true,
    dependencies = {
      "marilari88/neotest-vitest",
      "nvim-neotest/neotest-jest",
    },
    opts = {
      adapters = {
        ["neotest-vitest"] = {},
        ["neotest-jest"] = {},
      },
    },
  },

  -- JetBrains/VS Code-style debug F-keys (these survive the terminal).
  -- The full <leader>d* dap group from dap.core stays as-is.
  {
    "mfussenegger/nvim-dap",
    keys = {
      { "<F9>", function() require("dap").toggle_breakpoint() end, desc = "Debug: Toggle Breakpoint" },
      { "<F5>", function() require("dap").continue() end, desc = "Debug: Continue" },
      { "<F8>", function() require("dap").step_over() end, desc = "Debug: Step Over" },
      { "<F7>", function() require("dap").step_into() end, desc = "Debug: Step Into" },
      { "<S-F8>", function() require("dap").step_out() end, desc = "Debug: Step Out" },
    },
  },
}
