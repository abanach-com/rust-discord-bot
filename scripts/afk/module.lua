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
        id = "afk",
        name = t("modules.afk.name"),
        version = "1.0.0",
        description = t("modules.afk.description"),
        category = "utility",
        default_enabled = true,
        commands = {
            {
                name = "afk",
                description = t("modules.afk.commands.afk.description"),
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "set",
                        description = t("modules.afk.commands.afk.options.set.description"),
                        options = {
                            {
                                type = "string",
                                name = "reason",
                                description = t("modules.afk.commands.afk.options.reason.description"),
                                required = false,
                                max_length = 256,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "clear",
                        description = t("modules.afk.commands.afk.options.clear.description"),
                        options = {},
                    },
                    {
                        type = "subcommand",
                        name = "status",
                        description = t("modules.afk.commands.afk.options.status.description"),
                        options = {},
                    },
                },
            },
        },
        events = { "message_create" },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                auto_clear_on_message = { type = "boolean", default = true },
                max_reason_length = { type = "integer", minimum = 10, maximum = 500, default = 256 },
                afk_prefix = { type = "string", default = "[AFK] " },
            },
        },
    },

    on_command = function(command, ctx)
        if command ~= "afk" then
            return {}
        end

        local sub, opts = get_subcommand(ctx.options, { "set", "clear", "status" })
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
            local reason = opts.reason or t("modules.afk.default_reason", ctx)
            return {
                {
                    type = "reply",
                    content = t("modules.afk.set_reply", { user = user_id, reason = reason }, ctx),
                    ephemeral = false,
                },
                {
                    type = "audit",
                    event = "afk_set",
                    data = {
                        user_id = user_id,
                        reason = reason,
                    },
                },
            }
        end

        if sub == "clear" then
            return {
                {
                    type = "reply",
                    content = t("modules.afk.clear_reply", { user = user_id }, ctx),
                    ephemeral = true,
                },
            }
        end

        if sub == "status" then
            return {
                {
                    type = "reply",
                    content = t("modules.afk.status_reply", { user = user_id }, ctx),
                    ephemeral = true,
                },
            }
        end

        return {}
    end,

    on_event = function(event, ctx)
        return {}
    end,
}
