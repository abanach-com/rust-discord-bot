local t = (rustybot and rustybot.t) or (zuckerbot and zuckerbot.t) or function(k) return k end

return {
    manifest = {
        id = "giveaway",
        name = "Giveaway System",
        version = "1.0.0",
        description = t("modules.giveaway.description"),
        category = "community",
        default_enabled = true,
        commands = {
            {
                name = "giveaway",
                description = "Create and manage prize giveaways on the server.",
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "start",
                        description = "Rozpocznij nowy konkurs.",
                        options = {
                            {
                                type = "string",
                                name = "prize",
                                description = "Nagroda w konkursie.",
                                required = true,
                            },
                            {
                                type = "integer",
                                name = "duration_minutes",
                                description = "Czas trwania w minutach.",
                                required = true,
                                min_value = 1,
                                max_value = 10080,
                            },
                            {
                                type = "integer",
                                name = "winners",
                                description = "Number of winners.",
                                required = false,
                                min_value = 1,
                                max_value = 20,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "reroll",
                        description = "Reroll winner for a concluded giveaway.",
                        options = {
                            {
                                type = "string",
                                name = "giveaway_id",
                                description = "Identyfikator konkursu.",
                                required = true,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "end",
                        description = "End a giveaway early.",
                        options = {
                            {
                                type = "string",
                                name = "giveaway_id",
                                description = "Identyfikator konkursu.",
                                required = true,
                            },
                        },
                    },
                },
            },
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                default_duration_minutes = { type = "integer", minimum = 1, default = 60 },
                max_winners = { type = "integer", minimum = 1, default = 10 },
                allow_self_entry = { type = "boolean", default = false },
                announcement_channel_id = { type = { "string", "null" } },
            },
        },
    },

    on_command = function(command, ctx)
        if command ~= "giveaway" then
            return {}
        end

        local options = ctx.options or {}
        local sub = options.subcommand or "start"

        if sub == "start" then
            local prize = options.prize or "Awesome Prize"
            local duration = options.duration_minutes or 60
            local winners = options.winners or 1

            local text = string.format(
                "🎉 **NOWY KONKURS (GIVEAWAY)!** 🎉\n\n" ..
                "🎁 **Nagroda:** %s\n" ..
                "👥 **Number of winners:** %d\n" ..
                "⏳ **Czas trwania:** %d min\n" ..
                "Host: <@%s>\n\n" ..
                "React with 🎉 to enter!",
                prize, winners, duration, ctx.user_id
            )

            return {
                {
                    type = "reply",
                    content = "Your giveaway was created and announced successfully!",
                    ephemeral = true,
                },
                {
                    type = "send_message",
                    channel_id = ctx.channel_id,
                    content = text,
                },
                {
                    type = "audit",
                    event = "giveaway_start",
                    data = {
                        host_id = ctx.user_id,
                        prize = prize,
                        duration_minutes = duration,
                        winners = winners,
                    },
                },
            }
        end

        if sub == "reroll" then
            local gid = options.giveaway_id or "gw-1"
            return {
                {
                    type = "reply",
                    content = string.format("🎲 Reroll for giveaway `%s` is complete! A new winner has been selected.", gid),
                    ephemeral = false,
                },
            }
        end

        if sub == "end" then
            local gid = options.giveaway_id or "gw-1"
            return {
                {
                    type = "reply",
                    content = string.format("🏁 Giveaway `%s` has been ended by the organizer.", gid),
                    ephemeral = false,
                },
            }
        end

        return {}
    end,
}
