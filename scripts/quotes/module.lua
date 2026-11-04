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
        id = "quotes",
        name = t("modules.quotes.name"),
        version = "1.0.0",
        description = t("modules.quotes.description"),
        category = "fun",
        default_enabled = true,
        commands = {
            {
                name = "quote",
                description = t("modules.quotes.commands.quote.description"),
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "random",
                        description = t("modules.quotes.commands.quote.options.random.description"),
                        options = {},
                    },
                    {
                        type = "subcommand",
                        name = "add",
                        description = t("modules.quotes.commands.quote.options.add.description"),
                        options = {
                            {
                                type = "user",
                                name = "author",
                                description = t("modules.quotes.commands.quote.options.author.description"),
                                required = true,
                            },
                            {
                                type = "string",
                                name = "text",
                                description = t("modules.quotes.commands.quote.options.text.description"),
                                required = true,
                                max_length = 500,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "list",
                        description = t("modules.quotes.commands.quote.options.list.description"),
                        options = {},
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
        if command ~= "quote" then
            return {}
        end

        local sub, opts = get_subcommand(ctx.options, { "random", "add", "list" })
        if not sub then
            return {
                {
                    type = "reply",
                    content = "Unsupported or missing subcommand.",
                    ephemeral = true,
                },
            }
        end

        if sub == "random" then
            return {
                {
                    type = "reply",
                    content = t("modules.quotes.no_quotes", ctx),
                    ephemeral = false,
                },
            }
        end

        if sub == "add" then
            return {
                {
                    type = "reply",
                    content = t("modules.quotes.added", ctx),
                    ephemeral = true,
                },
            }
        end

        if sub == "list" then
            return {
                {
                    type = "reply",
                    content = t("modules.quotes.no_quotes", ctx),
                    ephemeral = false,
                },
            }
        end

        return {}
    end,
}
