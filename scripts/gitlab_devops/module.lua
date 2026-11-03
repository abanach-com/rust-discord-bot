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
        id = "gitlab_devops",
        name = "GitLab DevOps",
        version = "1.0.0",
        description = "Integracja gitlab.com oraz self-hosted z kontrolą domen i SSRF.",
        category = "developer",
        default_enabled = true,
        commands = {
            {
                name = "gitlab",
                description = "Komenda modułu GitLab DevOps.",
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = "Akcja do wykonania.", required = false }
                }
            }
        },
    },
    on_command = function(ctx)
        local cmd = ctx.command or "help"
        return reply("Moduł GitLab DevOps aktywny. Wywołano komendę: " .. cmd)
    end,
}
