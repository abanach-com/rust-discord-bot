local t = (rustybot and rustybot.t) or (zuckerbot and zuckerbot.t) or function(k) return k end

local function get_subcommand(options, allowed)
    local root = options or {}
    for _, name in ipairs(allowed) do
        if type(root[name]) == "table" then
            return name, root[name]
        end
    end
    if type(root.subcommand) == "string" then
        for _, name in ipairs(allowed) do
            if root.subcommand == name then
                return name, root
            end
        end
    end
    return nil, {}
end

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

        local sub, opts = get_subcommand(ctx.options, { "start", "end", "reroll" })
        if not sub then
            return {
                {
                    type = "reply",
                    content = "Unsupported or missing subcommand.",
                    ephemeral = true,
                },
            }
        end

        if sub == "start" then
            local prize = opts.prize or "Awesome Prize"
            local duration = opts.duration_minutes or 60
            local winners = opts.winners or 1

            return {
                {
                    type = "giveaway",
                    operation = {
                        operation = "start",
                        prize = prize,
                        duration_minutes = duration,
                        winners = winners,
                        channel_id = ctx.channel_id,
                    },
                },
            }
        end

        if sub == "reroll" then
            local gid = opts.giveaway_id or ""
            return {
                {
                    type = "giveaway",
                    operation = {
                        operation = "reroll",
                        giveaway_id = gid,
                    },
                },
            }
        end

        if sub == "end" then
            local gid = opts.giveaway_id or ""
            return {
                {
                    type = "giveaway",
                    operation = {
                        operation = "end",
                        giveaway_id = gid,
                    },
                },
            }
        end

        return {}
    end,
}
