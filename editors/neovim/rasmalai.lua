vim.filetype.add({
    extension = {
        rnx = "rasmalai",
    },
    filename = {
        ["Project.config"] = "rasmalai",
        ["Project.deplock"] = "rasmalai",
    },
})

vim.api.nvim_create_autocmd("FileType", {
    pattern = "rasmalai",
    callback = function()
        vim.lsp.start({
            name = "rasmalai",
            cmd = { "rnx", "lsp" },
            root_dir = vim.fs.root(0, { "Project.config", ".git" }),
        })
    end,
})
