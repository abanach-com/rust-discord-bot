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
        id = "integrations_core",
        name = "Integration Core & OAuth",
        version = "1.0.0",
        description = "Fundament providerów, autoryzacja OAuth2 oraz webhooki.",
        category = "integrations",
        default_enabled = true,
        commands = {
            {
                name = "integrations",
                description = "Komenda modułu Integration Core & OAuth.",
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = "Akcja do wykonania.", required = false }
                }
            }
        },
    },
    on_command = function(ctx)
        local cmd = ctx.command or "help"
        return reply("Moduł Integration Core & OAuth aktywny. Wywołano komendę: " .. cmd)
    end,
}
