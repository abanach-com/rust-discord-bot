local function reply(content)
    return { { type = "reply", content = content, ephemeral = true } }
end

return {
    manifest = {
        id = "lua_hello",
        name = "Lua Hello",
        version = "1.0.0",
        description = "A complete documentation example with ordinary slash options.",
        category = "examples",
        default_enabled = true,
        commands = {
            {
                name = "lua-hello",
                description = "Try the Lua documentation example.",
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "string",
                        name = "text",
                        description = "A short greeting.",
                        required = false,
                        min_length = 1,
                        max_length = 100,
                    },
                    {
                        type = "user",
                        name = "person",
                        description = "Show the selected user ID.",
                        required = false,
                    },
                },
            },
        },
        events = {},
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                greeting = { type = "string", minLength = 1, maxLength = 80 },
            },
            required = { "greeting" },
        },
    },
    on_command = function(command, ctx)
        if command ~= "lua-hello" then
            return {}
        end
        local options = ctx.options or {}
        local config = ctx.config or {}
        local text = options.text or config.greeting or "Hello"
        local name = rustybot.escape_mentions(ctx.user_name or "friend")
        local content = rustybot.escape_mentions(text) .. ", " .. name .. "!"
        if options.person then
            content = content .. " Selected user ID: " .. options.person
        end
        return reply(content)
    end,
}
