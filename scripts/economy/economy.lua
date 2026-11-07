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
        id = "economy",
        name = "Economy System",
        version = "1.0.0",
        description = "Wirtualna ekonomia: portfel, bank, praca, napady, sklep serwerowy i ekwipunek.",
        category = "economy",
        default_enabled = true,
        commands = {
            {
                name = "economy",
                description = t("modules.economy.description"),
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "balance",
                        description = "Check wallet, bank balance, and net worth.",
                        options = {},
                    },
                    {
                        type = "subcommand",
                        name = "work",
                        description = "Work a shift and earn coins.",
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
                        description = "Browse items and roles available in server shop.",
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
                        description = "Transfer coins to another user.",
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
                                description = "Amount of coins to transfer.",
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

        local sub, opts = get_subcommand(ctx.options, { "balance", "work", "crime", "shop", "buy", "inventory", "transfer" })
        if not sub then
            return {
                {
                    type = "reply",
                    content = "Unsupported or missing subcommand.",
                    ephemeral = true,
                },
            }
        end

        local user_id = ctx.user_id

        if sub == "balance" then
            return {
                {
                    type = "progression",
                    operation = {
                        type = "balance",
                        user_id = user_id,
                    },
                },
            }
        end

        if sub == "work" then
            return {
                {
                    type = "economy",
                    operation = {
                        type = "work",
                    },
                },
            }
        end

        if sub == "crime" then
            return {
                {
                    type = "economy",
                    operation = {
                        type = "crime",
                    },
                },
            }
        end

        if sub == "shop" then
            return {
                {
                    type = "economy",
                    operation = {
                        type = "shop",
                    },
                },
            }
        end

        if sub == "buy" then
            local item_id = opts.item
            if not item_id or tostring(item_id):match("^%s*$") then
                return {
                    {
                        type = "reply",
                        content = "Należy podać identyfikator lub nazwę przedmiotu do zakupu.",
                        ephemeral = true,
                    },
                }
            end
            return {
                {
                    type = "economy",
                    operation = {
                        type = "buy",
                        item = tostring(item_id),
                    },
                },
            }
        end

        if sub == "inventory" then
            return {
                {
                    type = "economy",
                    operation = {
                        type = "inventory",
                    },
                },
            }
        end

        if sub == "transfer" then
            local recipient = opts.recipient
            local amount = tonumber(opts.amount) or 10
            if not recipient then
                return {
                    {
                        type = "reply",
                        content = t("modules.economy.pay_no_recipient", ctx),
                        ephemeral = true,
                    },
                }
            end

            return {
                {
                    type = "progression",
                    operation = {
                        type = "transfer",
                        user_id = tostring(recipient),
                        amount = amount,
                    },
                },
            }
        end

        return {}
    end,
}
