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
        id = "social_publisher",
        name = "Social Publisher",
        version = "1.0.0",
        description = "Wieloplatformowy kompozytor wpisów z harmonogramem i zatwierdzeniami.",
        category = "social",
        default_enabled = true,
        commands = {
            {
                name = "publish",
                description = "Komenda modułu Social Publisher.",
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = "Akcja do wykonania.", required = false }
                }
            }
        },
    },
    on_command = function(ctx)
        local cmd = ctx.command or "help"
        return reply("Moduł Social Publisher aktywny. Wywołano komendę: " .. cmd)
    end,
}
