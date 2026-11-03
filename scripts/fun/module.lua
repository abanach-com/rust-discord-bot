local t = (rustybot and rustybot.t) or (zuckerbot and zuckerbot.t) or function(k) return k end

local memes = {
    "Developer: I fixed the bug. QA: Which one? Developer: The one I saw.",
    "Code works. Nobody knows why. Do not touch before deploy.",
    "It is not a bug. It is an undocumented server feature.",
    "Shortest developer horror story: It worked on my machine.",
    "Admin does not sleep. Admin waits for the next alert.",
}

local answers = {
    "Definitely yes.",
    "Most likely.",
    "Looks promising.",
    "Ask again later.",
    "Do not count on it.",
    "Definitely no.",
}

return {
    manifest = {
        id = "fun",
        name = "Fun and Memes",
        version = "0.2.0",
        description = t("modules.fun.description"),
        category = "engagement",
        default_enabled = true,
        commands = {
            {
                name = "meme",
                description = "Losuje mem tekstowy.",
                integration_types = { "guild", "user" },
                contexts = { "guild", "bot_dm", "private_channel" },
            },
            {
                name = "roll",
                description = "Rolls a random number from 1 to specified maximum.",
                integration_types = { "guild", "user" },
                contexts = { "guild", "bot_dm", "private_channel" },
                options = {
                    {
                        type = "integer",
                        name = "max",
                        description = "Maximum possible number.",
                        required = false,
                        min_value = 2,
                        max_value = 1000000,
                    },
                },
            },
            {
                name = "choose",
                description = "Picks one item from a comma-separated list.",
                integration_types = { "guild", "user" },
                contexts = { "guild", "bot_dm", "private_channel" },
                options = {
                    {
                        type = "string",
                        name = "options",
                        description = "For example: tank, healer, damage.",
                        required = true,
                        min_length = 3,
                        max_length = 500,
                    },
                },
            },
            {
                name = "eightball",
                description = "Odpowiada na pytanie jak magiczna kula.",
                integration_types = { "guild", "user" },
                contexts = { "guild", "bot_dm", "private_channel" },
                options = {
                    {
                        type = "string",
                        name = "question",
                        description = "Question for the bot to answer.",
                        required = true,
                        min_length = 2,
                        max_length = 300,
                    },
                },
            },
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {},
        },
    },

    on_command = function(command, ctx)
        local user_suffix = tonumber(string.sub(ctx.user_id or "", -8)) or 1
        math.randomseed(zuckerbot.unix_time() + user_suffix)

        if command == "meme" then
            return {
                {
                    type = "reply",
                    content = memes[math.random(#memes)],
                    ephemeral = false,
                },
            }
        end

        if command == "roll" then
            local raw_max = math.tointeger(ctx.options.max) or 100
            local maximum = math.max(2, raw_max)
            return {
                {
                    type = "reply",
                    content = string.format("%s rolled: **%d** (1–%d)", zuckerbot.escape_mentions(ctx.user_name), math.random(maximum), maximum),
                    ephemeral = false,
                },
            }
        end

        if command == "choose" then
            local values = {}
            for value in string.gmatch(ctx.options.options or "", "([^,]+)") do
                value = string.gsub(value, "^%s*(.-)%s*$", "%1")
                if value ~= "" then
                    table.insert(values, value)
                end
            end

            if #values < 2 then
                return {
                    {
                        type = "reply",
                        content = "Podaj co najmniej dwie opcje rozdzielone przecinkami.",
                        ephemeral = true,
                    },
                }
            end

            return {
                {
                    type = "reply",
                    content = "Wybieram: **" .. zuckerbot.escape_mentions(values[math.random(#values)]) .. "**",
                    ephemeral = false,
                },
            }
        end

        if command == "eightball" then
            local question = ctx.options.question or ""
            local answer = answers[math.random(#answers)]
            return {
                {
                    type = "reply",
                    content = string.format("> *%s*\n🎱 %s", zuckerbot.escape_mentions(question), answer),
                    ephemeral = false,
                },
            }
        end

        return {}
    end,
}
