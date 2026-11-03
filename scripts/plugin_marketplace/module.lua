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
        id = "plugin_marketplace",
        name = "Plugin Marketplace",
        version = "1.0.0",
        description = "Katalog weryfikowanych kryptograficznie modułów z transakcyjną instalacją.",
        category = "management",
        default_enabled = true,
        commands = {
            {
                name = "plugin",
                description = "Komenda modułu Plugin Marketplace.",
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = "Akcja do wykonania.", required = false }
                }
            }
        },
    },
    on_command = function(ctx)
        local cmd = ctx.command or "help"
        return reply("Moduł Plugin Marketplace aktywny. Wywołano komendę: " .. cmd)
    end,
}
