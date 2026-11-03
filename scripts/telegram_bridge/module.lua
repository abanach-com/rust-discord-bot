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
        id = "telegram_bridge",
        name = "Telegram Bridge",
        version = "1.0.0",
        description = "Dwukierunkowy przekaźnik wiadomości z filtrem pętli i mapowaniem.",
        category = "integrations",
        default_enabled = true,
        commands = {
            {
                name = "telegram",
                description = "Komenda modułu Telegram Bridge.",
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = "Akcja do wykonania.", required = false }
                }
            }
        },
    },
    on_command = function(ctx)
        local cmd = ctx.command or "help"
        return reply("Moduł Telegram Bridge aktywny. Wywołano komendę: " .. cmd)
    end,
}
