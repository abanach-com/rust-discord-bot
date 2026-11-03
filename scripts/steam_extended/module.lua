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
        id = "steam_extended",
        name = "Steam Extended",
        version = "1.0.0",
        description = "Profile Steam, gry, osiągnięcia i grupy LFG.",
        category = "games",
        default_enabled = true,
        commands = {
            {
                name = "steam",
                description = "Komenda modułu Steam Extended.",
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = "Akcja do wykonania.", required = false }
                }
            }
        },
    },
    on_command = function(ctx)
        local cmd = ctx.command or "help"
        return reply("Moduł Steam Extended aktywny. Wywołano komendę: " .. cmd)
    end,
}
