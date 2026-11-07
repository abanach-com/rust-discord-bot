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
        id = "birthdays",
        name = t("modules.birthdays.name"),
        version = "1.0.0",
        description = t("modules.birthdays.description"),
        category = "community",
        default_enabled = true,
        commands = {
            {
                name = "birthday",
                description = t("modules.birthdays.commands.birthday.description"),
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "set",
                        description = t("modules.birthdays.commands.birthday.options.set.description"),
                        options = {
                            {
                                type = "integer",
                                name = "day",
                                description = t("modules.birthdays.commands.birthday.options.day.description"),
                                required = true,
                                min_value = 1,
                                max_value = 31,
                            },
                            {
                                type = "integer",
                                name = "month",
                                description = t("modules.birthdays.commands.birthday.options.month.description"),
                                required = true,
                                min_value = 1,
                                max_value = 12,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "check",
                        description = t("modules.birthdays.commands.birthday.options.check.description"),
                        options = {
                            {
                                type = "user",
                                name = "user",
                                description = t("modules.birthdays.commands.birthday.options.user.description"),
                                required = false,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "upcoming",
                        description = t("modules.birthdays.commands.birthday.options.upcoming.description"),
                        options = {
                            {
                                type = "integer",
                                name = "limit",
                                description = "Liczba nadchodzących urodzin do wyświetlenia (1-50)",
                                required = false,
                                min_value = 1,
                                max_value = 50,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "delete",
                        description = "Usuń swoje zapisane urodziny",
                        options = {},
                    },
                },
            },
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                announcement_channel_id = { type = { "string", "null" } },
                birthday_role_id = { type = { "string", "null" } },
            },
        },
    },

    on_command = function(command, ctx)
        if command ~= "birthday" then
            return {}
        end

        local sub, opts = get_subcommand(ctx.options, { "set", "check", "upcoming", "delete" })
        if not sub then
            return {
                {
                    type = "reply",
                    content = "Unsupported or missing subcommand.",
                    ephemeral = true,
                },
            }
        end

        if sub == "set" then
            local day = tonumber(opts.day)
            local month = tonumber(opts.month)
            if not day or not month then
                return {
                    {
                        type = "reply",
                        content = "Missing required options: day and month are required.",
                        ephemeral = true,
                    },
                }
            end
            return {
                {
                    type = "birthday",
                    operation = {
                        operation = "set",
                        day = day,
                        month = month,
                    },
                },
            }
        end

        if sub == "check" then
            local op = { operation = "check" }
            if opts.user and tostring(opts.user) ~= "" then
                op.user_id = tostring(opts.user)
            end
            return {
                {
                    type = "birthday",
                    operation = op,
                },
            }
        end

        if sub == "upcoming" then
            local limit = tonumber(opts.limit) or 10
            return {
                {
                    type = "birthday",
                    operation = {
                        operation = "upcoming",
                        limit = limit,
                    },
                },
            }
        end

        if sub == "delete" then
            return {
                {
                    type = "birthday",
                    operation = {
                        operation = "delete",
                    },
                },
            }
        end

        return {}
    end,
}
