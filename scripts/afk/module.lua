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

        if sub == "set" then
            local reason = opts.reason
            return {
                {
                    type = "afk",
                    operation = {
                        operation = "set",
                        reason = reason,
                    },
                },
            }
        end

        if sub == "clear" then
            return {
                {
                    type = "afk",
                    operation = {
                        operation = "clear",
                    },
                },
            }
        end

        if sub == "status" then
            return {
                {
                    type = "afk",
                    operation = {
                        operation = "status",
                    },
                },
            }
        end

        return {}
    end,

    on_event = function(event, ctx)
        if event ~= "message_create" then
            return {}
        end

        local author_id = tostring(ctx.actor_id or "")
        if author_id == "" then
            return {}
        end

        local mentions = {}
        if type(ctx.data) == "table" and type(ctx.data.mentions) == "table" then
            for _, m in ipairs(ctx.data.mentions) do
                local mid = tostring(m or "")
                if mid ~= "" then
                    table.insert(mentions, mid)
                end
            end
        end

        local auto_clear = true
        if type(ctx.config) == "table" and ctx.config.auto_clear_on_message == false then
            auto_clear = false
        end

        return {
            {
                type = "afk",
                operation = {
                    operation = "on_message",
                    author_id = author_id,
                    mentioned_user_ids = mentions,
                    auto_clear = auto_clear,
                },
            },
        }
    end,
}
