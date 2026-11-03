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
        id = "ai_mcp",
        name = t("modules.ai_mcp.name"),
        version = "1.0.0",
        description = t("modules.ai_mcp.description"),
        category = "ai",
        default_enabled = true,
        commands = {
            {
                name = "ai",
                description = t("modules.ai_mcp.commands.ai.description"),
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = t("modules.ai_mcp.commands.ai.options.action.description"), required = false }
                }
            },
            {
                name = "mcp",
                description = t("modules.ai_mcp.commands.mcp.description"),
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = t("modules.ai_mcp.commands.mcp.options.action.description"), required = false }
                }
            }
        },
    },
    on_command = function(command, ctx)
        local cmd = (type(command) == "string" and command) or (ctx and ctx.command) or "help"
        local context = (type(command) == "table" and command) or ctx or {}
        return reply(t("modules.ai_mcp.active_reply", { cmd = cmd }, context))
    end,
}
