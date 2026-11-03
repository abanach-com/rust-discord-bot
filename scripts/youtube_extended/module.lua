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
        id = "youtube_extended",
        name = "YouTube Extended",
        version = "1.0.0",
        description = "WebSub feedy, wykrywanie premier i wideo bez kosztownego pollingu.",
        category = "media",
        default_enabled = true,
        commands = {
            {
                name = "youtube",
                description = "Komenda modułu YouTube Extended.",
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = "Akcja do wykonania.", required = false }
                }
            }
        },
    },
    on_command = function(ctx)
        local cmd = ctx.command or "help"
        return reply("Moduł YouTube Extended aktywny. Wywołano komendę: " .. cmd)
    end,
}
