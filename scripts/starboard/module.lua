local t = (rustybot and rustybot.t) or (zuckerbot and zuckerbot.t) or function(k) return k end

return {
    manifest = {
        id = "starboard",
        name = t("modules.starboard.name"),
        version = "1.0.0",
        description = t("modules.starboard.description"),
        category = "community",
        default_enabled = true,
        commands = {
            {
                name = "starboard",
                description = t("modules.starboard.commands.starboard.description"),
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "config",
                        description = t("modules.starboard.commands.starboard.options.config.description"),
                        options = {
                            {
                                type = "channel",
                                name = "channel",
                                description = t("modules.starboard.commands.starboard.options.channel.description"),
                                required = true,
                            },
                            {
                                type = "integer",
                                name = "threshold",
                                description = t("modules.starboard.commands.starboard.options.threshold.description"),
                                required = false,
                                min_value = 1,
                                max_value = 25,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "stats",
                        description = t("modules.starboard.commands.starboard.options.stats.description"),
                        options = {},
                    },
                },
            },
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                channel_id = { type = "string" },
                threshold = { type = "integer", minimum = 1, maximum = 25, default = 3 },
            },
        },
    },

    on_command = function(command, ctx)
        if command ~= "starboard" then
            return {}
        end

        local options = ctx.options or {}
        local sub = options.subcommand or "stats"

        if sub == "config" then
            local ch = options.channel or "0"
            local thresh = options.threshold or 3
            return {
                {
                    type = "reply",
                    content = t("modules.starboard.config_reply", { channel = ch, threshold = thresh }, ctx),
                    ephemeral = true,
                },
            }
        end

        if sub == "stats" then
            local ch = (ctx.config and ctx.config.channel_id) or "0"
            local thresh = (ctx.config and ctx.config.threshold) or 3
            return {
                {
                    type = "reply",
                    content = t("modules.starboard.stats_reply", { channel = ch, threshold = thresh }, ctx),
                    ephemeral = false,
                },
            }
        end

        return {}
    end,
}
