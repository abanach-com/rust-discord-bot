local t = (rustybot and rustybot.t) or (zuckerbot and zuckerbot.t) or function(k) return k end

local DISCORD_EPOCH_MS = 1420070400000

local function reply(content, ephemeral)
    return {
        {
            type = "reply",
            content = zuckerbot.truncate(content, 2000),
            ephemeral = ephemeral ~= false,
        },
    }
end

local function seed(ctx)
    local tail = tonumber(string.sub(ctx.user_id or "0", -9)) or 0
    math.randomseed(zuckerbot.unix_time() + tail)
end

local function format_number(value)
    if value == math.huge or value == -math.huge or value ~= value then
        return nil
    end
    return string.format("%.12g", value)
end

local function discord_timestamp(raw_id)
    if not string.match(raw_id or "", "^%d+$") then
        return nil
    end

    local value = math.tointeger(tonumber(raw_id))
    if not value or value <= 0 then
        return nil
    end

    local milliseconds = (value >> 22) + DISCORD_EPOCH_MS
    return milliseconds // 1000
end

return {
    manifest = {
        id = "utility",
        name = "Utility Toolkit",
        version = "1.0.0",
        description = t("modules.utility.description"),
        category = "utility",
        default_enabled = true,
        commands = {
            {
                name = "calculate",
                description = "Perform safe basic mathematical calculation.",
                dm_permission = true,
                options = {
                    { type = "number", name = "a", description = "Pierwsza liczba.", required = true },
                    {
                        type = "string",
                        name = "operation",
                        description = "Math operation.",
                        required = true,
                        choices = {
                            { name = "Dodawanie", value = "add" },
                            { name = "Odejmowanie", value = "subtract" },
                            { name = "Multiplication", value = "multiply" },
                            { name = "Dzielenie", value = "divide" },
                            { name = "Modulo", value = "modulo" },
                            { name = "Power", value = "power" },
                        },
                    },
                    { type = "number", name = "b", description = "Druga liczba.", required = true },
                },
            },
            {
                name = "convert",
                description = "Konwertuje popularne jednostki.",
                dm_permission = true,
                options = {
                    { type = "number", name = "value", description = "Value to convert.", required = true },
                    {
                        type = "string",
                        name = "unit",
                        description = "Kierunek konwersji.",
                        required = true,
                        choices = {
                            { name = "Kilometry → mile", value = "km_mi" },
                            { name = "Mile → kilometry", value = "mi_km" },
                            { name = "Kilogramy → funty", value = "kg_lb" },
                            { name = "Funty → kilogramy", value = "lb_kg" },
                            { name = "Celsjusz → Fahrenheit", value = "c_f" },
                            { name = "Fahrenheit → Celsjusz", value = "f_c" },
                        },
                    },
                },
            },
            {
                name = "snowflake",
                description = "Odczytuje czas utworzenia identyfikatora Discorda.",
                dm_permission = true,
                options = {
                    {
                        type = "string",
                        name = "id",
                        description = "Identifier for user, role, channel, or message.",
                        required = true,
                        min_length = 5,
                        max_length = 20,
                    },
                },
            },
            {
                name = "timestamp",
                description = "Tworzy gotowy znacznik czasu Discorda.",
                dm_permission = true,
                options = {
                    {
                        type = "integer",
                        name = "offset",
                        description = "Offset from now in seconds; negative values allowed.",
                        required = false,
                        max_value = 31536000,
                    },
                    {
                        type = "string",
                        name = "style",
                        description = "Styl znacznika.",
                        required = false,
                        choices = {
                            { name = "Short Time", value = "t" },
                            { name = "Long Time", value = "T" },
                            { name = "Short Date", value = "d" },
                            { name = "Long Date", value = "D" },
                            { name = "Data i czas", value = "f" },
                            { name = "Full Date and Time", value = "F" },
                            { name = "Relative", value = "R" },
                        },
                    },
                },
            },
            {
                name = "randomnumber",
                description = "Random integer within chosen range.",
                dm_permission = true,
                options = {
                    { type = "integer", name = "minimum", description = "Lower bound; may be negative.", required = true, max_value = 1000000000 },
                    { type = "integer", name = "maximum", description = "Upper bound; may be negative.", required = true, max_value = 1000000000 },
                },
            },
            {
                name = "texttool",
                description = "Transform text or measure its length.",
                dm_permission = true,
                options = {
                    {
                        type = "string",
                        name = "action",
                        description = "Text operation.",
                        required = true,
                        choices = {
                            { name = "Wielkie litery", value = "upper" },
                            { name = "Lower Case", value = "lower" },
                            { name = "Reverse", value = "reverse" },
                            { name = "Policz znaki", value = "length" },
                        },
                    },
                    { type = "string", name = "text", description = "Input text.", required = true, min_length = 1, max_length = 1500 },
                },
            },
            {
                name = "ids",
                description = "Pokazuje wybrane identyfikatory Discorda.",
                dm_permission = false,
                options = {
                    { type = "user", name = "user", description = "Optional user.", required = false },
                    { type = "role", name = "role", description = "Opcjonalna rola.", required = false },
                    { type = "channel", name = "channel", description = "Optional channel.", required = false },
                },
            },
            {
                name = "color",
                description = "Sprawdza i normalizuje kolor zapisany jako HEX.",
                dm_permission = true,
                options = {
                    { type = "string", name = "hex", description = "Kolor, np. #5865F2.", required = true, min_length = 3, max_length = 9 },
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
        if command == "calculate" then
            local a = tonumber(ctx.options.a)
            local b = tonumber(ctx.options.b)
            local operation = ctx.options.operation
            local result
            local symbol

            if operation == "add" then result, symbol = a + b, "+"
            elseif operation == "subtract" then result, symbol = a - b, "−"
            elseif operation == "multiply" then result, symbol = a * b, "×"
            elseif operation == "divide" then
                if b == 0 then return reply(t("modules.utility.div_zero", ctx)) end
                result, symbol = a / b, "÷"
            elseif operation == "modulo" then
                if b == 0 then return reply("Modulo by zero is not allowed.") end
                result, symbol = a % b, "%"
            elseif operation == "power" then
                if math.abs(b) > 100 or math.abs(a) > 1000000000 then
                    return reply(t("modules.utility.overflow", ctx))
                end
                result, symbol = a ^ b, "^"
            else
                return reply(t("modules.utility.unknown_op", ctx))
            end

            local formatted = format_number(result)
            if not formatted then return reply("Result is not a finite number.") end
            return reply(string.format("`%.12g %s %.12g = %s`", a, symbol, b, formatted), false)
        end

        if command == "convert" then
            local value = tonumber(ctx.options.value)
            local unit = ctx.options.unit
            local result, label
            if unit == "km_mi" then result, label = value * 0.6213711922, "mi"
            elseif unit == "mi_km" then result, label = value * 1.609344, "km"
            elseif unit == "kg_lb" then result, label = value * 2.2046226218, "lb"
            elseif unit == "lb_kg" then result, label = value / 2.2046226218, "kg"
            elseif unit == "c_f" then result, label = (value * 9 / 5) + 32, "°F"
            elseif unit == "f_c" then result, label = (value - 32) * 5 / 9, "°C"
            else return reply("Unknown conversion.") end
            return reply(string.format("**%.6g** → **%.6g %s**", value, result, label), false)
        end

        if command == "snowflake" then
            local created = discord_timestamp(ctx.options.id)
            if not created then return reply(t("modules.utility.invalid_snowflake", ctx)) end
            return reply(string.format("Created: <t:%d:F> — <t:%d:R>\nUnix: `%d`", created, created, created), false)
        end

        if command == "timestamp" then
            local offset = math.tointeger(ctx.options.offset or 0) or 0
            if offset < -31536000 then return reply("Minimum offset is minus 365 days.") end
            local value = zuckerbot.unix_time() + offset
            local style = ctx.options.style or "F"
            local markup = string.format("<t:%d:%s>", value, style)
            return reply("Preview: " .. markup .. "\nCode: `" .. markup .. "`", false)
        end

        if command == "randomnumber" then
            local minimum = math.tointeger(ctx.options.minimum)
            local maximum = math.tointeger(ctx.options.maximum)
            if not minimum or not maximum then return reply("Range must contain integers.") end
            if minimum < -1000000000 or maximum < -1000000000 then
                return reply("Minimum allowed value is `-1000000000`.")
            end
            if minimum > maximum then minimum, maximum = maximum, minimum end
            seed(ctx)
            return reply(string.format("Wylosowano **%d** z zakresu `%d–%d`.", math.random(minimum, maximum), minimum, maximum), false)
        end

        if command == "texttool" then
            local action = ctx.options.action
            local text = ctx.options.text or ""
            if action == "upper" then return reply(string.upper(text), false) end
            if action == "lower" then return reply(string.lower(text), false) end
            if action == "reverse" then return reply(string.reverse(text), false) end
            if action == "length" then
                local count = utf8 and utf8.len(text) or #text
                return reply(string.format("Character count: **%d**", count or #text), false)
            end
            return reply("Nieznana operacja tekstowa.")
        end

        if command == "ids" then
            local lines = {}
            if ctx.options.user then table.insert(lines, "User: `" .. ctx.options.user .. "`") end
            if ctx.options.role then table.insert(lines, "Rola: `" .. ctx.options.role .. "`") end
            if ctx.options.channel then table.insert(lines, "Channel: `" .. ctx.options.channel .. "`") end
            if #lines == 0 then
                table.insert(lines, "Ty: `" .. ctx.user_id .. "`")
                table.insert(lines, "Channel: `" .. ctx.channel_id .. "`")
                if ctx.guild_id then table.insert(lines, "Serwer: `" .. ctx.guild_id .. "`") end
            end
            return reply(table.concat(lines, "\n"))
        end

        if command == "color" then
            local value = string.upper(string.gsub(ctx.options.hex or "", "#", ""))
            if #value == 3 and string.match(value, "^[0-9A-F]+$") then
                value = string.sub(value, 1, 1) .. string.sub(value, 1, 1)
                    .. string.sub(value, 2, 2) .. string.sub(value, 2, 2)
                    .. string.sub(value, 3, 3) .. string.sub(value, 3, 3)
            end
            if #value ~= 6 or not string.match(value, "^[0-9A-F]+$") then
                return reply("Provide color as 3- or 6-digit HEX, e.g. `#5865F2`.")
            end
            local decimal = tonumber(value, 16)
            return reply(string.format("HEX: `#%s`\nRGB: `%d, %d, %d`\nDecimal: `%d`", value, decimal >> 16, (decimal >> 8) & 255, decimal & 255, decimal), false)
        end

        return {}
    end,
}
