local function reply(content)
    return { { type = "reply", content = content, ephemeral = true } }
end

return {
    manifest = {
        id = "lua_router",
        name = "Lua Nested Router",
        version = "1.0.0",
        description = "Subcommands and groups using nested option tables.",
        category = "examples",
        commands = {
            {
                name = "lua-route",
                description = "Try nested slash-command options.",
                options = {
                    {
                        type = "subcommand",
                        name = "echo",
                        description = "Echo a short string.",
                        options = {
                            {
                                type = "string",
                                name = "text",
                                description = "Text to echo.",
                                required = true,
                                min_length = 1,
                                max_length = 100,
                            },
                        },
                    },
                    {
                        type = "subcommand_group",
                        name = "inspect",
                        description = "Inspect a selected value.",
                        options = {
                            {
                                type = "subcommand",
                                name = "user",
                                description = "Show an opaque user ID.",
                                options = {
                                    {
                                        type = "user",
                                        name = "person",
                                        description = "User to inspect.",
                                        required = true,
                                    },
                                },
                            },
                        },
                    },
                },
            },
        },
        config_schema = { type = "object", additionalProperties = false },
    },
    on_command = function(command, ctx)
        if command ~= "lua-route" then
            return {}
        end
        local options = ctx.options or {}
        if options.echo then
            local text = options.echo.text
            if type(text) ~= "string" or text == "" then
                return reply("Please provide text.")
            end
            return reply(rustybot.escape_mentions(text))
        end
        if options.inspect and options.inspect.user then
            local person = options.inspect.user.person
            if type(person) ~= "string" then
                return reply("Please select a user.")
            end
            return reply("Selected user ID: " .. person)
        end
        return reply("Choose echo or inspect user.")
    end,
}
