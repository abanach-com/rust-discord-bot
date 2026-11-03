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
        id = "unified_identity",
        name = "Unified Identity",
        version = "1.0.0",
        description = "Centralne dobrowolne powiązania kont Discord z zewnętrznymi profilami.",
        category = "integrations",
        default_enabled = true,
        commands = {
            {
                name = "identity",
                description = "Komenda modułu Unified Identity.",
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = "Akcja do wykonania.", required = false }
                }
            }
        },
    },
    on_command = function(ctx)
        local cmd = ctx.command or "help"
        return reply("Moduł Unified Identity aktywny. Wywołano komendę: " .. cmd)
    end,
}
