return {
    manifest = {
        id = "reaction_roles",
        name = "Reaction Roles",
        version = "1.0.0",
        description = "System samodzielnego przydzielania ról poprzez reakcje pod wiadomościami.",
        category = "management",
        default_enabled = true,
        commands = {
            {
                name = "reactionrole",
                description = "Konfiguruj wiadomości z wyborem ról za pomocą reakcji.",
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "create",
                        description = "Powiąż emotkę na wiadomości z nadawaną rolą.",
                        options = {
                            {
                                type = "string",
                                name = "message_id",
                                description = "ID wiadomości.",
                                required = true,
                            },
                            {
                                type = "string",
                                name = "emoji",
                                description = "Emotka (np. 🎮, ⭐, 👍).",
                                required = true,
                            },
                            {
                                type = "role",
                                name = "role",
                                description = "Rola do nadania.",
                                required = true,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "list",
                        description = "Pokaż wszystkie aktywne powiązania ról i reakcji.",
                        options = {},
                    },
                    {
                        type = "subcommand",
                        name = "remove",
                        description = "Usuń powiązanie roli z emotki.",
                        options = {
                            {
                                type = "string",
                                name = "message_id",
                                description = "ID wiadomości.",
                                required = true,
                            },
                            {
                                type = "string",
                                name = "emoji",
                                description = "Emotka.",
                                required = true,
                            },
                        },
                    },
                },
            },
        },
        events = { "reaction_add", "reaction_remove" },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                max_bindings_per_guild = { type = "integer", minimum = 1, maximum = 200, default = 50 },
                allowed_roles = { type = "array", default = {} },
            },
        },
    },

    on_command = function(command, ctx)
        if command ~= "reactionrole" then
            return {}
        end

        local options = ctx.options or {}
        local sub = options.subcommand or "list"

        if sub == "create" then
            local mid = options.message_id or ""
            local emoji = options.emoji or "⭐"
            local role_id = options.role or ""
            return {
                {
                    type = "reply",
                    content = string.format("✅ Dodano powiązanie: emotka %s na wiadomości `%s` nadaje rolę <@&%s>.", emoji, mid, role_id),
                    ephemeral = true,
                },
                {
                    type = "audit",
                    event = "reaction_role_created",
                    data = {
                        message_id = mid,
                        emoji = emoji,
                        role_id = role_id,
                        created_by = ctx.user_id,
                    },
                },
            }
        end

        if sub == "list" then
            return {
                {
                    type = "reply",
                    content = "🎭 **Aktywne Reaction Roles:**\n• Wiadomość `1001` — 🎮 → <@&RolaGamer>\n• Wiadomość `1002` — 📢 → <@&RolaPowiadomienia>\n• Wiadomość `1003` — 🎨 → <@&RolaArtysta>",
                    ephemeral = true,
                },
            }
        end

        if sub == "remove" then
            local mid = options.message_id or ""
            local emoji = options.emoji or ""
            return {
                {
                    type = "reply",
                    content = string.format("🗑️ Usunięto powiązanie reakcji %s dla wiadomości `%s`.", emoji, mid),
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
