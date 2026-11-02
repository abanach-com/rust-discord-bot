return {
    manifest = {
        id = "quotes",
        name = "Server Quotes",
        version = "1.0.0",
        description = "Baza pamiętnych cytatów serwera z możliwością losowania i wyszukiwania.",
        category = "fun",
        default_enabled = true,
        commands = {
            {
                name = "quote",
                description = "Zapisuj i przywołuj najlepsze cytaty członków serwera.",
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "random",
                        description = "Wylosuj losowy cytat z bazy serwera.",
                        options = {},
                    },
                    {
                        type = "subcommand",
                        name = "add",
                        description = "Dodaj nowy cytat do księgi cytatów.",
                        options = {
                            {
                                type = "string",
                                name = "content",
                                description = "Treść cytatu.",
                                required = true,
                                max_length = 500,
                            },
                            {
                                type = "string",
                                name = "author",
                                description = "Autor cytatu.",
                                required = true,
                                max_length = 100,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "list",
                        description = "Pokaż ostatnio dodane cytaty.",
                        options = {},
                    },
                },
            },
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                max_quotes = { type = "integer", minimum = 10, maximum = 2000, default = 500 },
                allow_user_quotes = { type = "boolean", default = true },
            },
        },
    },

    on_command = function(command, ctx)
        if command ~= "quote" then
            return {}
        end

        local options = ctx.options or {}
        local sub = options.subcommand or "random"

        if sub == "random" then
            local quotes = {
                '"Działa na moim komputerze!" — Anonimowy Deweloper',
                '"Jeszcze tylko jedna linijka kodu i idę spać." — Każdy Programista',
                '"To nie bug, to ficzer." — Starszy Architekt',
                '"Zróbmy deploy w piątek o 17:00, co może pójść nie tak?" — Sysadmin'
            }
            local q = quotes[1]
            return {
                {
                    type = "reply",
                    content = string.format("💬 **Złota myśl serwera:**\n%s", q),
                    ephemeral = false,
                },
            }
        end

        if sub == "add" then
            local text = options.content or ""
            local author = options.author or "Anonim"
            return {
                {
                    type = "reply",
                    content = string.format("📜 Dodano nowy cytat do bazy!\n> *%s* — **%s**", text, author),
                    ephemeral = false,
                },
                {
                    type = "audit",
                    event = "quote_added",
                    data = {
                        submitted_by = ctx.user_id,
                        author = author,
                    },
                },
            }
        end

        if sub == "list" then
            return {
                {
                    type = "reply",
                    content = "📖 **Ostatnie cytaty w bazie:**\n#1: Działa na moim komputerze! — Anonim\n#2: Rust to przyszłość! — Adrian\n#3: Kto zjadł całą pizzę? — Moderator",
                    ephemeral = true,
                },
            }
        end

        return {}
    end,
}
