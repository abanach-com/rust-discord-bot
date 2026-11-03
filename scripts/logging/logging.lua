local t = (rustybot and rustybot.t) or (zuckerbot and zuckerbot.t) or function(k) return k end

return {
    manifest = {
        id = "logging",
        name = t("modules.logging.name"),
        version = "1.0.0",
        description = t("modules.logging.description"),
        category = "security",
        default_enabled = true,
        commands = {
            {
                name = "logging",
                description = t("modules.logging.commands.logging.description"),
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "config",
                        description = t("modules.logging.commands.logging.options.config.description"),
                        options = {
                            {
                                type = "channel",
                                name = "channel",
                                description = t("modules.logging.commands.logging.options.channel.description"),
                                required = true,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "test",
                        description = t("modules.logging.commands.logging.options.test.description"),
                        options = {},
                    },
                },
            },
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                log_channel_id = { type = "string" },
            },
        },
    },

    on_command = function(command, ctx)
        if command ~= "logging" then
            return {}
        end

        local options = ctx.options or {}
        local sub = options.subcommand or "test"

        if sub == "config" then
            local ch = options.channel or "0"
            return {
                {
                    type = "reply",
                    content = t("modules.logging.config_reply", { channel = ch }, ctx),
                    ephemeral = true,
                },
            }
        end

        if sub == "test" then
            local ch = (ctx.config and ctx.config.log_channel_id) or "0"
            return {
                {
                    type = "reply",
                    content = t("modules.logging.test_reply", { channel = ch }, ctx),
                    ephemeral = true,
                },
            }
        end

        return {}
    end,
}
