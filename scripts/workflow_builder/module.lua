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
        id = "workflow_builder",
        name = t("modules.workflow_builder.name"),
        version = "1.0.0",
        description = t("modules.workflow_builder.description"),
        category = "automation",
        default_enabled = true,
        commands = {
            {
                name = "workflow",
                description = t("modules.workflow_builder.commands.workflow.description"),
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = t("modules.workflow_builder.commands.workflow.options.action.description"), required = false }
                }
            }
        },
    },
    on_command = function(command, ctx)
        local cmd = (type(command) == "string" and command) or (ctx and ctx.command) or "help"
        local context = (type(command) == "table" and command) or ctx or {}
        return reply(t("modules.workflow_builder.active_reply", { cmd = cmd }, context))
    end,
}
