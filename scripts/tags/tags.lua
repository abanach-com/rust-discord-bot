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
        id = "tags",
        name = t("modules.tags.name"),
        version = "1.0.0",
        description = t("modules.tags.description"),
        category = "utility",
        default_enabled = true,
        commands = {
            {
                name = "tag",
                description = t("modules.tags.commands.tag.description"),
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "get",
                        description = t("modules.tags.commands.tag.options.get.description"),
                        options = {
                            {
                                type = "string",
                                name = "name",
                                description = t("modules.tags.commands.tag.options.name.description"),
                                required = true,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "create",
                        description = t("modules.tags.commands.tag.options.create.description"),
                        options = {
                            {
                                type = "string",
                                name = "name",
                                description = t("modules.tags.commands.tag.options.name.description"),
                                required = true,
                            },
                            {
                                type = "string",
                                name = "content",
                                description = t("modules.tags.commands.tag.options.content.description"),
                                required = true,
                                max_length = 1500,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "delete",
                        description = t("modules.tags.commands.tag.options.delete.description"),
                        options = {
                            {
                                type = "string",
                                name = "name",
                                description = t("modules.tags.commands.tag.options.name.description"),
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
            properties = {},
        },
    },

    on_command = function(command, ctx)
        if command ~= "tag" then
            return {}
        end

        local sub, opts = get_subcommand(ctx.options, { "get", "create", "delete" })
        if not sub then
            return {
                {
                    type = "reply",
                    content = "Unsupported or missing subcommand.",
                    ephemeral = true,
                },
            }
        end

        local name = opts.name or ""

        if sub == "get" then
            return {
                {
                    type = "tag",
                    operation = {
                        operation = "get",
                        name = name,
                    },
                },
            }
        end

        if sub == "create" then
            local content = opts.content or ""
            return {
                {
                    type = "tag",
                    operation = {
                        operation = "create",
                        name = name,
                        content = content,
                    },
                },
            }
        end

        if sub == "delete" then
            return {
                {
                    type = "tag",
                    operation = {
                        operation = "delete",
                        name = name,
                    },
                },
            }
        end

        return {}
    end,
}
