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
        id = "twitch_extended",
        name = "Twitch Extended",
        version = "1.0.0",
        description = "EventSub, live role, automatyczne embedy i monitoring transmisji.",
        category = "media",
        default_enabled = true,
        commands = {
            {
                name = "twitch",
                description = "Komenda modułu Twitch Extended.",
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = "Akcja do wykonania.", required = false }
                }
            }
        },
    },
    on_command = function(ctx)
        local cmd = ctx.command or "help"
        return reply("Moduł Twitch Extended aktywny. Wywołano komendę: " .. cmd)
    end,
}
