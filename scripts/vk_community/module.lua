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
        id = "vk_community",
        name = "VK Integration",
        version = "1.0.0",
        description = "Społeczności VKontakte Callback API i powiadomienia o postach.",
        category = "social",
        default_enabled = true,
        commands = {
            {
                name = "vk",
                description = "Komenda modułu VK Integration.",
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = "Akcja do wykonania.", required = false }
                }
            }
        },
    },
    on_command = function(ctx)
        local cmd = ctx.command or "help"
        return reply("Moduł VK Integration aktywny. Wywołano komendę: " .. cmd)
    end,
}
