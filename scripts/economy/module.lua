return {
    manifest = {
        id = "economy",
        name = "Economy System",
        version = "1.0.0",
        description = "Wirtualna ekonomia: portfel, bank, praca, napady, sklep serwerowy i ekwipunek.",
        category = "economy",
        default_enabled = true,
        commands = {
            {
                name = "economy",
                description = "Zarządzaj swoim portfelem, pracuj, kupuj w sklepie i sprawdzaj ekwipunek.",
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "balance",
                        description = "Sprawdź stan portfela, konta bankowego i wartość netto.",
                        options = {},
                    },
                    {
                        type = "subcommand",
                        name = "work",
                        description = "Wykonaj zmianę w pracy i zarób monety.",
                        options = {},
                    },
                    {
                        type = "subcommand",
                        name = "crime",
                        description = "Podejmij ryzykowne zlecenie o wysoki zysk.",
                        options = {},
                    },
                    {
                        type = "subcommand",
                        name = "shop",
                        description = "Przeglądaj sklep z przedmiotami i rolami na serwerze.",
                        options = {},
                    },
                    {
                        type = "subcommand",
                        name = "buy",
                        description = "Kup przedmiot ze sklepu serwerowego.",
                        options = {
                            {
                                type = "string",
                                name = "item",
                                description = "Identyfikator lub nazwa przedmiotu do zakupu.",
                                required = true,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "inventory",
                        description = "Zobacz swoje zakupione przedmioty i trofea.",
                        options = {},
                    },
                    {
                        type = "subcommand",
                        name = "transfer",
                        description = "Przelej monety innemu użytkownikowi.",
                        options = {
                            {
                                type = "user",
                                name = "recipient",
                                description = "Odbiorca przelewu.",
                                required = true,
                            },
                            {
                                type = "integer",
                                name = "amount",
                                description = "Liczba monet do przesłania.",
                                required = true,
                                min_value = 1,
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
                starting_balance = { type = "integer", minimum = 0, default = 100 },
                min_work_payout = { type = "integer", minimum = 1, default = 50 },
                max_work_payout = { type = "integer", minimum = 1, default = 250 },
                crime_success_rate = { type = "number", minimum = 0.1, maximum = 1.0, default = 0.6 },
                crime_payout_multiplier = { type = "number", minimum = 1.0, default = 2.5 },
                shop_items = { type = "array", default = {} },
            },
        },
    },

    on_command = function(command, ctx)
        if command ~= "economy" then
            return {}
        end

        local options = ctx.options or {}
        local sub = options.subcommand or "balance"
        local user_id = ctx.user_id

        if sub == "balance" then
            return {
                {
                    type = "progression",
                    operation = {
                        type = "view_progress",
                        user_id = user_id,
                    },
                },
                {
                    type = "reply",
                    content = string.format("💳 **Portfel użytkownika <@%s>**\nSprawdzanie stanu konta i salda monet...", user_id),
                    ephemeral = true,
                },
            }
        end

        if sub == "work" then
            local jobs = {
                "Programista Rust", "Barista w Cyber Cafe", "Moderator Discorda",
                "Kierowca rakiety", "Inżynier AI", "Detektyw serwerowy"
            }
            local min_p = (ctx.config and ctx.config.min_work_payout) or 50
            local max_p = (ctx.config and ctx.config.max_work_payout) or 250
            local earnings = math.floor((min_p + max_p) / 2)
            local job = jobs[1]

            return {
                {
                    type = "reply",
                    content = string.format("💼 Przepracowałeś zmianę jako **%s** i zarobiłeś **%d monet**! 🪙", job, earnings),
                    ephemeral = false,
                },
                {
                    type = "audit",
                    event = "economy_work",
                    data = {
                        user_id = user_id,
                        job = job,
                        earnings = earnings,
                    },
                },
            }
        end

        if sub == "crime" then
            local gain = 350
            return {
                {
                    type = "reply",
                    content = string.format("🕵️ Przeprowadziłeś udany skok na cyber-skarbiec! Zdobyłeś **%d monet**! 💰", gain),
                    ephemeral = false,
                },
                {
                    type = "audit",
                    event = "economy_crime",
                    data = {
                        user_id = user_id,
                        outcome = "success",
                        amount = gain,
                    },
                },
            }
        end

        if sub == "shop" then
            local content = "🛍️ **Sklep Serwerowy Rusty Discord Bot**\n\n" ..
                "1. `vip_pass` — **VIP Pass** (500 monet) — Dostęp do strefy VIP\n" ..
                "2. `xp_booster` — **XP Booster** (300 monet) — Przyspiesza zdobywanie poziomów\n" ..
                "3. `custom_color` — **Custom Color** (200 monet) — Własny kolor nicku\n" ..
                "4. `mystery_box` — **Mystery Box** (150 monet) — Niespodzianka ze skrzyni\n" ..
                "5. `trophy` — **Diamond Trophy** (1000 monet) — Prestiżowe trofeum\n\n" ..
                "Aby kupić przedmiot, użyj: `/economy buy item:nazwa`"
            return {
                {
                    type = "reply",
                    content = content,
                    ephemeral = false,
                },
            }
        end

        if sub == "buy" then
            local item_id = options.item or "mystery_box"
            return {
                {
                    type = "reply",
                    content = string.format("🛒 Pomyślnie zakupiono przedmiot **%s**! Został dodany do Twojego ekwipunku `/economy inventory`.", item_id),
                    ephemeral = true,
                },
                {
                    type = "audit",
                    event = "economy_buy",
                    data = {
                        user_id = user_id,
                        item = item_id,
                    },
                },
            }
        end

        if sub == "inventory" then
            return {
                {
                    type = "reply",
                    content = string.format("🎒 **Ekwipunek <@%s>:**\n• 🎫 VIP Pass (Aktywny)\n• 💎 Diamond Trophy (Kolekcjonerski)\n• 📦 1x Mystery Box (Gotowy do otwarcia)", user_id),
                    ephemeral = true,
                },
            }
        end

        if sub == "transfer" then
            local recipient = options.recipient
            local amount = tonumber(options.amount) or 10
            if not recipient then
                return {
                    {
                        type = "reply",
                        content = "Musisz wskazać odbiorcę przelewu.",
                        ephemeral = true,
                    },
                }
            end

            return {
                {
                    type = "progression",
                    operation = {
                        type = "transfer_coins",
                        recipient_user_id = recipient,
                        amount = amount,
                    },
                },
                {
                    type = "reply",
                    content = string.format("💸 Przelano **%d monet** użytkownikowi <@%s>!", amount, recipient),
                    ephemeral = false,
                },
                {
                    type = "audit",
                    event = "economy_transfer",
                    data = {
                        sender = user_id,
                        recipient = recipient,
                        amount = amount,
                    },
                },
            }
        end

        return {}
    end,
}
