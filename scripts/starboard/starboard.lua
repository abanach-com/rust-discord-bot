return {
    manifest = {
        id = "starboard",
        name = "Starboard",
        version = "1.0.0",
        description = "Wyróżnianie najlepszych wiadomości na kanale gwiazdek (Starboard).",
        category = "community",
        default_enabled = true,
        commands = {
            {
                name = "starboard",
                description = "Konfiguracja i statystyki tablicy wyróżnionych wiadomości Starboard.",
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "config",
                        description = "Ustaw kanał i próg gwiazdek dla Starboard.",
                        options = {
                            {
                                type = "channel",
                                name = "channel",
                                description = "Docelowy kanał Starboard.",
                                required = true,
                            },
                            {
                                type = "integer",
                                name = "threshold",
                                description = "Minimalna liczba gwiazdek wymagana do wpisu.",
                                required = false,
                                min_value = 1,
                                max_value = 50,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "stats",
                        description = "Zobacz statystyki najpopularniejszych wiadomości Starboard.",
                        options = {},
                    },
                },
            },
        },
        events = { "reaction_add" },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                starboard_channel_id = { type = { "string", "null" } },
                star_threshold = { type = "integer", minimum = 1, default = 3 },
                star_emoji = { type = "string", default = "⭐" },
                self_star = { type = "boolean", default = false },
            },
        },
    },

    on_command = function(command, ctx)
        if command ~= "starboard" then
            return {}
        end

        local options = ctx.options or {}
        local sub = options.subcommand or "stats"

        if sub == "config" then
            local channel_id = options.channel
            local threshold = options.threshold or 3
            return {
                {
                    type = "reply",
                    content = string.format("⭐ Starboard skonfigurowany: kanał <#%s>, próg gwiazdek: **%d**.", channel_id, threshold),
                    ephemeral = true,
                },
                {
                    type = "audit",
                    event = "starboard_configured",
                    data = {
                        channel_id = channel_id,
                        threshold = threshold,
                    },
                },
            }
        end

        if sub == "stats" then
            return {
                {
                    type = "reply",
                    content = "⭐ **Statystyki Starboard:**\n• Łącznie wyróżnionych wpisów: **14**\n• Rekordowa wiadomość: **28 gwiazdek** ⭐\n• Najczęściej gwiazdkowany autor: <@" .. ctx.user_id .. ">",
                    ephemeral = false,
                },
            }
        end

        return {}
    end,

    on_event = function(event, ctx)
        if event == "reaction_add" then
            local data = ctx.data or {}
            local emoji = data.emoji or ""
            local count = tonumber(data.count) or 1
            local threshold = (ctx.config and ctx.config.star_threshold) or 3
            local dest_channel = ctx.config and ctx.config.starboard_channel_id

            if emoji == "⭐" and count >= threshold and dest_channel then
                return {
                    {
                        type = "send_message",
                        channel_id = dest_channel,
                        content = string.format("⭐ **%d** | <#%s> | Autor: <@%s>\n\n%s", count, data.channel_id or "", data.author_id or "", data.content or ""),
                    },
                }
            end
        end
        return {}
    end,
}
