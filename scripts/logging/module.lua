return {
    manifest = {
        id = "logging",
        name = "Audit Logging",
        version = "1.0.0",
        description = "Rejestr operacji serwerowych i zdarzeń Discorda na wyznaczonym kanale audytowym.",
        category = "security",
        default_enabled = true,
        commands = {
            {
                name = "logging",
                description = "Konfiguracja kanału logów i test wysyłki zdarzeń audytowych.",
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "config",
                        description = "Ustaw kanał logów serwerowych.",
                        options = {
                            {
                                type = "channel",
                                name = "channel",
                                description = "Kanał do publikacji logów audytowych.",
                                required = true,
                            },
                            {
                                type = "boolean",
                                name = "log_joins",
                                description = "Loguj wejścia i wyjścia członków.",
                                required = false,
                            },
                            {
                                type = "boolean",
                                name = "log_messages",
                                description = "Loguj usunięte i edytowane wiadomości.",
                                required = false,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "test",
                        description = "Wyślij testowy log na skonfigurowany kanał.",
                        options = {},
                    },
                },
            },
        },
        events = { "guild_member_remove", "message_delete", "channel_create" },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                log_channel_id = { type = { "string", "null" } },
                log_joins = { type = "boolean", default = true },
                log_leaves = { type = "boolean", default = true },
                log_moderation = { type = "boolean", default = true },
                log_message_deletes = { type = "boolean", default = true },
            },
        },
    },

    on_command = function(command, ctx)
        if command ~= "logging" then
            return {}
        end

        local options = ctx.options or {}
        local sub = options.subcommand or "test"

        if sub == "config" then
            local ch = options.channel
            return {
                {
                    type = "reply",
                    content = string.format("🛡️ Kanał audytowy został ustawiony na <#%s>.", ch),
                    ephemeral = true,
                },
                {
                    type = "audit",
                    event = "logging_channel_updated",
                    data = {
                        channel_id = ch,
                        configured_by = ctx.user_id,
                    },
                },
            }
        end

        if sub == "test" then
            local dest = (ctx.config and ctx.config.log_channel_id) or ctx.channel_id
            return {
                {
                    type = "reply",
                    content = "Wysyłanie próbnego logu audytowego...",
                    ephemeral = true,
                },
                {
                    type = "send_message",
                    channel_id = dest,
                    content = string.format("📋 **[LOG TESTOWY]** Administrator <@%s> przetestował poprawność działania modułu Audit Logging.", ctx.user_id),
                },
            }
        end

        return {}
    end,

    on_event = function(event, ctx)
        local dest = ctx.config and ctx.config.log_channel_id
        if not dest then
            return {}
        end

        if event == "message_delete" and ctx.config.log_message_deletes then
            local data = ctx.data or {}
            return {
                {
                    type = "send_message",
                    channel_id = dest,
                    content = string.format("🗑️ **Wiadomość usunięta** na kanale <#%s> | Autor: <@%s>", data.channel_id or "", data.author_id or ""),
                },
            }
        end

        if event == "guild_member_remove" and ctx.config.log_leaves then
            return {
                {
                    type = "send_message",
                    channel_id = dest,
                    content = string.format("🚪 **Członek opuścił serwer:** <@%s>", ctx.actor_id or "Nieznany"),
                },
            }
        end

        return {}
    end,
}
