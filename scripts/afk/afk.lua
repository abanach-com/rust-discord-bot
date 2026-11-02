return {
    manifest = {
        id = "afk",
        name = "AFK System",
        version = "1.0.0",
        description = "Statusy nieobecności (Away From Keyboard) z automatyczną odpowiedzią na wzmianki.",
        category = "utility",
        default_enabled = true,
        commands = {
            {
                name = "afk",
                description = "Ustaw lub wyczyść swój status nieobecności AFK.",
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "set",
                        description = "Ustaw status AFK z opcjonalnym powodem.",
                        options = {
                            {
                                type = "string",
                                name = "reason",
                                description = "Powód nieobecności.",
                                required = false,
                                max_length = 256,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "clear",
                        description = "Wyczyść swój status AFK.",
                        options = {},
                    },
                    {
                        type = "subcommand",
                        name = "status",
                        description = "Sprawdź swój aktualny status AFK.",
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

        local options = ctx.options or {}
        local sub = options.subcommand or "status"
        local user_id = ctx.user_id

        if sub == "set" then
            local reason = options.reason or "Zaraz wracam"
            return {
                {
                    type = "reply",
                    content = string.format("💤 <@%s> jest teraz AFK: *%s*", user_id, reason),
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
                    content = string.format("👋 Witaj z powrotem <@%s>! Twój status AFK został wyłączony.", user_id),
                    ephemeral = true,
                },
            }
        end

        if sub == "status" then
            return {
                {
                    type = "reply",
                    content = string.format("ℹ️ Status AFK dla <@%s>: Aktywny na serwerze.", user_id),
                    ephemeral = true,
                },
            }
        end

        return {}
    end,

    on_event = function(event, ctx)
        if event == "message_create" then
            local data = ctx.data or {}
            local mentions = data.mentions or {}
            -- If user returns, greet them if configured
            return {}
        end
        return {}
    end,
}
