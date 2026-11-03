local function reply(content)
    return {
        {
            type = "reply",
            content = zuckerbot.truncate(content, 2000),
            ephemeral = true,
        },
    }
end

return {
    manifest = {
        id = "github_devops",
        name = "GitHub DevOps",
        version = "1.0.0",
        description = "Zdarzenia Git, workflowy CI/CD i mapowanie wątków forum.",
        category = "developer",
        default_enabled = true,
        commands = {
            {
                name = "github",
                description = "Komenda modułu GitHub DevOps.",
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = "Akcja do wykonania.", required = false }
                }
            }
        },
    },
    on_command = function(ctx)
        local cmd = ctx.command or "help"
        return reply("Moduł GitHub DevOps aktywny. Wywołano komendę: " .. cmd)
    end,
}
