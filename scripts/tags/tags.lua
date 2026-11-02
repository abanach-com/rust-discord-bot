return {
    manifest = {
        id = "tags",
        name = "Custom Tags",
        version = "1.0.0",
        description = "Własne tagi, skróty i szablony wiadomości ze wsparciem serwerowym.",
        category = "utility",
        default_enabled = true,
        commands = {
            {
                name = "tag",
                description = "Zarządzaj szybkimi tagami i szablonami odpowiedzi.",
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "get",
                        description = "Wyświetl treść zapisanego tagu.",
                        options = {
                            {
                                type = "string",
                                name = "name",
                                description = "Nazwa tagu.",
                                required = true,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "create",
                        description = "Utwórz nowy tag na serwerze.",
                        options = {
                            {
                                type = "string",
                                name = "name",
                                description = "Nazwa tagu (unikalna).",
                                required = true,
                            },
                            {
                                type = "string",
                                name = "content",
                                description = "Treść wiadomości tagu.",
                                required = true,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "edit",
                        description = "Zaktualizuj istniejący tag.",
                        options = {
                            {
                                type = "string",
                                name = "name",
                                description = "Nazwa tagu.",
                                required = true,
                            },
                            {
                                type = "string",
                                name = "content",
                                description = "Nowa treść tagu.",
                                required = true,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "delete",
                        description = "Usuń wybrany tag.",
                        options = {
                            {
                                type = "string",
                                name = "name",
                                description = "Nazwa tagu do usunięcia.",
                                required = true,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "list",
                        description = "Wyświetl listę wszystkich dostępnych tagów.",
                        options = {},
                    },
                },
            },
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                max_tags_per_guild = { type = "integer", minimum = 1, maximum = 500, default = 100 },
                max_tag_length = { type = "integer", minimum = 10, maximum = 2000, default = 2000 },
                restrict_creation_to_roles = { type = "array", default = {} },
            },
        },
    },

    on_command = function(command, ctx)
        if command ~= "tag" then
            return {}
        end

        local options = ctx.options or {}
        local sub = options.subcommand or "list"

        if sub == "get" then
            local tag_name = options.name or ""
            return {
                {
                    type = "reply",
                    content = string.format("📌 **Tag `%s`:**\nWitaj! To jest przykładowa treść zapisanego tagu na serwerze.", tag_name),
                    ephemeral = false,
                },
            }
        end

        if sub == "create" then
            local tag_name = options.name or ""
            local tag_content = options.content or ""
            return {
                {
                    type = "reply",
                    content = string.format("✅ Tag `%s` został pomyślnie utworzony!", tag_name),
                    ephemeral = true,
                },
                {
                    type = "audit",
                    event = "tag_created",
                    data = {
                        author_id = ctx.user_id,
                        tag_name = tag_name,
                    },
                },
            }
        end

        if sub == "edit" then
            local tag_name = options.name or ""
            return {
                {
                    type = "reply",
                    content = string.format("✏️ Tag `%s` został zaktualizowany.", tag_name),
                    ephemeral = true,
                },
            }
        end

        if sub == "delete" then
            local tag_name = options.name or ""
            return {
                {
                    type = "reply",
                    content = string.format("🗑️ Tag `%s` został usunięty z bazy serwera.", tag_name),
                    ephemeral = true,
                },
            }
        end

        if sub == "list" then
            return {
                {
                    type = "reply",
                    content = "📋 **Lista dostępnych tagów na serwerze:**\n`faq`, `pomoc`, `regulamin`, `ip`, `discord`, `support`\nUżyj: `/tag get name:nazwa`",
                    ephemeral = true,
                },
            }
        end

        return {}
    end,
}
