local t = (rustybot and rustybot.t) or (zuckerbot and zuckerbot.t) or function(k) return k end

local function reply(content)
    local trunc = (rustybot and rustybot.truncate) or (zuckerbot and zuckerbot.truncate) or function(s, max) return s end
    return {
        {
            type = "reply",
            content = trunc(content, 2000),
            ephemeral = true,
        },
    }
end

return {
    manifest = {
        id = "gaming_hub",
        name = t("modules.gaming_hub.name"),
        version = "1.0.0",
        description = t("modules.gaming_hub.description"),
        category = "games",
        default_enabled = true,
        commands = {
            {
                name = "games",
                description = t("modules.gaming_hub.commands.games.description"),
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = t("modules.gaming_hub.commands.games.options.action.description"), required = false }
                }
            }
        },
    },
    on_command = function(command, ctx)
        local cmd = (type(command) == "string" and command) or (ctx and ctx.command) or "help"
        local context = (type(command) == "table" and command) or ctx or {}
        return reply(t("modules.gaming_hub.active_reply", { cmd = cmd }, context))
    end,
}
