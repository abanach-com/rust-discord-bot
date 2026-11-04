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
                log_channel_id = { type = { "string", "null" } },
            },
        },
    },

    on_command = function(command, ctx)
        if command ~= "logging" then
            return {}
        end

        local sub, opts = get_subcommand(ctx.options, { "config", "test" })
        if not sub then
            return {
                {
                    type = "reply",
                    content = "Unsupported or missing subcommand.",
                    ephemeral = true,
                },
            }
        end

        if sub == "config" then
            local ch = opts.channel or "0"
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
