local t = (rustybot and rustybot.t) or (zuckerbot and zuckerbot.t) or function(k) return k end

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

        local options = ctx.options or {}
        local sub = options.subcommand or "list"
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
            local target = options.user or user_id
            return {
                {
                    type = "reply",
                    content = t("modules.achievements.view_reply", { user = target }, ctx),
                    ephemeral = false,
                },
            }
        end

        if sub == "claim" then
            local bid = options.badge_id or "first_message"
            return {
                {
                    type = "progression",
                    operation = {
                        type = "adjust_member_progress",
                        user_id = user_id,
                        xp_delta = 100,
                        coins_delta = 50,
                        reputation_delta = 1,
                        reason = "Achievement: " .. bid,
                    },
                },
                {
                    type = "reply",
                    content = t("modules.achievements.claim_reply", { badge = bid }, ctx),
                    ephemeral = false,
                },
                {
                    type = "audit",
                    event = "achievement_claimed",
                    data = {
                        user_id = user_id,
                        badge_id = bid,
                    },
                },
            }
        end

        return {}
    end,
}
