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

        local sub, opts = get_subcommand(ctx.options, { "set", "check", "upcoming" })
        if not sub then
            return {
                {
                    type = "reply",
                    content = "Unsupported or missing subcommand.",
                    ephemeral = true,
                },
            }
        end

        local user_id = ctx.user_id

        if sub == "set" then
            local day = opts.day or 1
            local month = opts.month or 1
            return {
                {
                    type = "reply",
                    content = t("modules.birthdays.set_reply", { day = day, month = month }, ctx),
                    ephemeral = true,
                },
            }
        end

        if sub == "check" then
            local target = opts.user or user_id
            return {
                {
                    type = "reply",
                    content = t("modules.birthdays.check_not_found", { user = target }, ctx),
                    ephemeral = false,
                },
            }
        end

        if sub == "upcoming" then
            return {
                {
                    type = "reply",
                    content = t("modules.birthdays.upcoming_empty", ctx),
                    ephemeral = false,
                },
            }
        end

        return {}
    end,
}
