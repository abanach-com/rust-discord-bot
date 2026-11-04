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
        id = "achievements",
        name = t("modules.achievements.name"),
        version = "1.0.0",
        description = t("modules.achievements.description"),
        category = "progression",
        default_enabled = true,
        commands = {
            {
                name = "achievements",
                description = t("modules.achievements.commands.achievements.description"),
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "list",
                        description = t("modules.achievements.commands.achievements.options.list.description"),
                        options = {},
                    },
                    {
                        type = "subcommand",
                        name = "view",
                        description = t("modules.achievements.commands.achievements.options.view.description"),
                        options = {
                            {
                                type = "user",
                                name = "user",
                                description = t("modules.achievements.commands.achievements.options.user.description"),
                                required = false,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "claim",
                        description = t("modules.achievements.commands.achievements.options.claim.description"),
                        options = {
                            {
                                type = "string",
                                name = "badge_id",
                                description = t("modules.achievements.commands.achievements.options.badge_id.description"),
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
                badges = { type = "array", default = {} },
            },
        },
    },

    on_command = function(command, ctx)
        if command ~= "achievements" then
            return {}
        end

        local sub, opts = get_subcommand(ctx.options, { "list", "view", "claim" })
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

        if sub == "list" then
            return {
                {
                    type = "reply",
                    content = t("modules.achievements.list_reply", ctx),
                    ephemeral = false,
                },
            }
        end

        if sub == "view" then
            local target = opts.user or user_id
            return {
                {
                    type = "reply",
                    content = t("modules.achievements.view_reply", { user = target }, ctx),
                    ephemeral = false,
                },
            }
        end

        if sub == "claim" then
            return {
                {
                    type = "reply",
                    content = "Achievement claiming is not yet available.",
                    ephemeral = true,
                },
            }
        end

        return {}
    end,
}
