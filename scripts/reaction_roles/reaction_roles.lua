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
        id = "reaction_roles",
        name = t("modules.reaction_roles.name"),
        version = "1.0.0",
        description = t("modules.reaction_roles.description"),
        category = "management",
        default_enabled = true,
        commands = {
            {
                name = "reactionrole",
                description = t("modules.reaction_roles.commands.reactionrole.description"),
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "create",
                        description = t("modules.reaction_roles.commands.reactionrole.options.create.description"),
                        options = {
                            {
                                type = "string",
                                name = "message_id",
                                description = t("modules.reaction_roles.commands.reactionrole.options.message_id.description"),
                                required = true,
                            },
                            {
                                type = "string",
                                name = "emoji",
                                description = t("modules.reaction_roles.commands.reactionrole.options.emoji.description"),
                                required = true,
                            },
                            {
                                type = "role",
                                name = "role",
                                description = t("modules.reaction_roles.commands.reactionrole.options.role.description"),
                                required = true,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "remove",
                        description = t("modules.reaction_roles.commands.reactionrole.options.remove.description"),
                        options = {
                            {
                                type = "string",
                                name = "message_id",
                                description = t("modules.reaction_roles.commands.reactionrole.options.message_id.description"),
                                required = true,
                            },
                            {
                                type = "string",
                                name = "emoji",
                                description = t("modules.reaction_roles.commands.reactionrole.options.emoji.description"),
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
        if command ~= "reactionrole" then
            return {}
        end

        local sub, opts = get_subcommand(ctx.options, { "create", "remove" })
        if not sub then
            return {
                {
                    type = "reply",
                    content = "Unsupported or missing subcommand.",
                    ephemeral = true,
                },
            }
        end

        if sub == "create" then
            return {
                {
                    type = "reply",
                    content = t("modules.reaction_roles.created", { emoji = opts.emoji or "⭐", role = opts.role or "0" }, ctx),
                    ephemeral = true,
                },
            }
        end

        if sub == "remove" then
            return {
                {
                    type = "reply",
                    content = t("modules.reaction_roles.removed", { emoji = opts.emoji or "⭐" }, ctx),
                    ephemeral = true,
                },
            }
        end

        return {}
    end,
}
