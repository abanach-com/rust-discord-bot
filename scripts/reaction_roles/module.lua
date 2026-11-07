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
            local message_id = tostring(opts.message_id or "")
            local emoji = tostring(opts.emoji or "")
            local role = tostring(opts.role or "")
            if message_id == "" or emoji == "" or role == "" then
                return {
                    {
                        type = "reply",
                        content = "Missing required parameters (message_id, emoji, role).",
                        ephemeral = true,
                    },
                }
            end
            return {
                {
                    type = "reaction_role",
                    operation = {
                        operation = "create",
                        message_id = message_id,
                        emoji = emoji,
                        role_id = role,
                    },
                },
            }
        end

        if sub == "remove" then
            local message_id = tostring(opts.message_id or "")
            local emoji = tostring(opts.emoji or "")
            if message_id == "" or emoji == "" then
                return {
                    {
                        type = "reply",
                        content = "Missing required parameters (message_id, emoji).",
                        ephemeral = true,
                    },
                }
            end
            return {
                {
                    type = "reaction_role",
                    operation = {
                        operation = "remove",
                        message_id = message_id,
                        emoji = emoji,
                    },
                },
            }
        end

        return {}
    end,
}
