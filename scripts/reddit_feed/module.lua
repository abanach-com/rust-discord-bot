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
        id = "reddit_feed",
        name = "Reddit Feed",
        version = "1.0.0",
        description = "Oficjalny poller subreddita z filtrem flairów i paginacją.",
        category = "social",
        default_enabled = true,
        commands = {
            {
                name = "reddit",
                description = "Komenda modułu Reddit Feed.",
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = "Akcja do wykonania.", required = false }
                }
            }
        },
    },
    on_command = function(ctx)
        local cmd = ctx.command or "help"
        return reply("Moduł Reddit Feed aktywny. Wywołano komendę: " .. cmd)
    end,
}
