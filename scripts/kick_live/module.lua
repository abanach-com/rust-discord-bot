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
        id = "kick_live",
        name = "Kick Integration",
        version = "1.0.0",
        description = "Oficjalny adapter Kick Dev API dla transmisji i alertów.",
        category = "media",
        default_enabled = true,
        commands = {
            {
                name = "kick",
                description = "Komenda modułu Kick Integration.",
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = "Akcja do wykonania.", required = false }
                }
            }
        },
    },
    on_command = function(ctx)
        local cmd = ctx.command or "help"
        return reply("Moduł Kick Integration aktywny. Wywołano komendę: " .. cmd)
    end,
}
