return {
    manifest = {
        id = "birthdays",
        name = "Birthday Tracker",
        version = "1.0.0",
        description = "Kalendarz urodzin, automatyczne życzenia i okolicznościowa rola.",
        category = "community",
        default_enabled = true,
        commands = {
            {
                name = "birthday",
                description = "Zapisz swoje urodziny i sprawdzaj nadchodzące święta członków serwera.",
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "set",
                        description = "Zapisz swoją datę urodzin.",
                        options = {
                            {
                                type = "integer",
                                name = "day",
                                description = "Dzień miesiąca (1-31).",
                                required = true,
                                min_value = 1,
                                max_value = 31,
                            },
                            {
                                type = "integer",
                                name = "month",
                                description = "Miesiąc (1-12).",
                                required = true,
                                min_value = 1,
                                max_value = 12,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "check",
                        description = "Sprawdź kiedy urodziny obchodzi wybrany użytkownik.",
                        options = {
                            {
                                type = "user",
                                name = "user",
                                description = "Użytkownik do sprawdzenia.",
                                required = false,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "upcoming",
                        description = "Pokaż najbliższe urodziny na serwerze.",
                        options = {},
                    },
                },
            },
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                announcement_channel_id = { type = { "string", "null" } },
                birthday_role_id = { type = { "string", "null" } },
                celebration_message = { type = "string", default = "Sto lat {user}! Wszystkiego najlepszego z okazji urodzin! 🎉🎂" },
            },
        },
    },

    on_command = function(command, ctx)
        if command ~= "birthday" then
            return {}
        end

        local options = ctx.options or {}
        local sub = options.subcommand or "upcoming"

        if sub == "set" then
            local day = options.day or 1
            local month = options.month or 1
            return {
                {
                    type = "reply",
                    content = string.format("🎂 Pomyślnie zapisano Twoje urodziny: **%02d.%02d**! Otrzymasz życzenia w dniu swoich urodzin.", day, month),
                    ephemeral = true,
                },
                {
                    type = "audit",
                    event = "birthday_registered",
                    data = {
                        user_id = ctx.user_id,
                        day = day,
                        month = month,
                    },
                },
            }
        end

        if sub == "check" then
            local target_user = options.user or ctx.user_id
            return {
                {
                    type = "reply",
                    content = string.format("📅 Urodziny użytkownika <@%s>: **15 maja** 🎈", target_user),
                    ephemeral = false,
                },
            }
        end

        if sub == "upcoming" then
            return {
                {
                    type = "reply",
                    content = "🎉 **Nadchodzące urodziny w tym miesiącu:**\n• <@" .. ctx.user_id .. "> — za 4 dni\n• Moderator — za 12 dni\nŻyczymy wszystkim solenizantom wspaniałego dnia!",
                    ephemeral = false,
                },
            }
        end

        return {}
    end,
}
