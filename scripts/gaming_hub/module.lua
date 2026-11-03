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
        id = "gaming_hub",
        name = "Gaming Hub",
        version = "1.0.0",
        description = "Agregacja statusów serwerów gier, PoE1/PoE2 oraz międzyplatformowe LFG.",
        category = "games",
        default_enabled = true,
        commands = {
            {
                name = "games",
                description = "Komenda modułu Gaming Hub.",
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = "Akcja do wykonania.", required = false }
                }
            }
        },
    },
    on_command = function(ctx)
        local cmd = ctx.command or "help"
        return reply("Moduł Gaming Hub aktywny. Wywołano komendę: " .. cmd)
    end,
}
