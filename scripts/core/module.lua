local t = (rustybot and rustybot.t) or (zuckerbot and zuckerbot.t) or function(k) return k end

return {
    manifest = {
        id = "core",
        name = t("modules.core.name"),
        version = "0.2.0",
        description = t("modules.core.description"),
        category = "platform",
        default_enabled = true,
        commands = {
            {
                name = "ping",
                description = t("modules.core.commands.ping.description"),
                integration_types = { "guild", "user" },
                contexts = { "guild", "bot_dm", "private_channel" },
            },
            {
                name = "about",
                description = t("modules.core.commands.about.description"),
                integration_types = { "guild", "user" },
                contexts = { "guild", "bot_dm", "private_channel" },
            },
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {},
        },
    },

    on_command = function(command, ctx)
        if command == "ping" then
            return {
                {
                    type = "reply",
                    content = t("modules.core.commands.ping.reply", ctx),
                    ephemeral = true,
                },
                {
                    type = "audit",
                    event = "ping",
                    data = {
                        user_id = ctx.user_id,
                        locale = ctx.locale,
                    },
                },
            }
        end

        if command == "about" then
            return {
                {
                    type = "reply",
                    content = t("modules.core.commands.about.reply", ctx),
                    ephemeral = true,
                },
            }
        end

        return {}
    end,
}
