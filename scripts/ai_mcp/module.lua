local function reply(content)
    return {
        {
            type = "reply",
            content = zuckerbot.truncate(content, 2000),
            ephemeral = true,
        },
    }
end

return {
    manifest = {
        id = "ai_mcp",
        name = "AI + MCP Gateway",
        version = "1.0.0",
        description = "Bramka AI z twardym budżetem tokenów, RAG ACL i protokołem MCP.",
        category = "ai",
        default_enabled = true,
        commands = {
            {
                name = "ai",
                description = "Komenda modułu AI + MCP Gateway.",
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = "Akcja do wykonania.", required = false }
                }
            },
            {
                name = "mcp",
                description = "Komenda modułu AI + MCP Gateway.",
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = "Akcja do wykonania.", required = false }
                }
            }
        },
    },
    on_command = function(ctx)
        local cmd = ctx.command or "help"
        return reply("Moduł AI + MCP Gateway aktywny. Wywołano komendę: " .. cmd)
    end,
}
