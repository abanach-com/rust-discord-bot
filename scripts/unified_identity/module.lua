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
        id = "unified_identity",
        name = t("modules.unified_identity.name"),
        version = "1.0.0",
        description = t("modules.unified_identity.description"),
        category = "integrations",
        default_enabled = true,
        commands = {
            {
                name = "identity",
                description = t("modules.unified_identity.commands.identity.description"),
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = t("modules.unified_identity.commands.identity.options.action.description"), required = false }
                }
            }
        },
    },
    on_command = function(command, ctx)
        local cmd = (type(command) == "string" and command) or (ctx and ctx.command) or "help"
        local context = (type(command) == "table" and command) or ctx or {}
        return reply(t("modules.unified_identity.active_reply", { cmd = cmd }, context))
    end,
}
