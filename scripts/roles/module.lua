local t = (rustybot and rustybot.t) or (zuckerbot and zuckerbot.t) or function(k) return k end

return {
    manifest = {
        id = "roles",
        name = t("modules.roles.name"),
        version = "0.2.0",
        description = t("modules.roles.description"),
        category = "administration",
        default_enabled = true,
        commands = {
            {
                name = "role",
                description = t("modules.roles.commands.role.description"),
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "string",
                        name = "action",
                        description = t("modules.roles.commands.role.options.action.description"),
                        required = true,
                        choices = {
                            { name = "Add", value = "add" },
                            { name = "Remove", value = "remove" },
                        },
                    },
                    {
                        type = "user",
                        name = "user",
                        description = t("modules.roles.commands.role.options.user.description"),
                        required = true,
                    },
                    {
                        type = "role",
                        name = "role",
                        description = t("modules.roles.commands.role.options.role.description"),
                        required = true,
                    },
                    {
                        type = "string",
                        name = "reason",
                        description = t("modules.roles.commands.role.options.reason.description"),
                        required = false,
                        max_length = 512,
                    },
                },
            },
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {},
        },
    },

    on_command = function(command, ctx)
        if command ~= "role" then
            return {}
        end

        local action_type = ctx.options.action == "add" and "add_role" or "remove_role"
        return {
            {
                type = "reply",
                content = t("modules.roles.queued", ctx),
                ephemeral = true,
            },
            {
                type = action_type,
                user_id = ctx.options.user,
                role_id = ctx.options.role,
                reason = ctx.options.reason or "Changed via RustyBot",
            },
        }
    end,
}
