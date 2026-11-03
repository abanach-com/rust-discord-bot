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
        id = "workflow_builder",
        name = "Workflow Builder",
        version = "1.0.0",
        description = "Wizualne automatyzacje DAG z detekcją cykli i dry-run.",
        category = "automation",
        default_enabled = true,
        commands = {
            {
                name = "workflow",
                description = "Komenda modułu Workflow Builder.",
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = "Akcja do wykonania.", required = false }
                }
            }
        },
    },
    on_command = function(ctx)
        local cmd = ctx.command or "help"
        return reply("Moduł Workflow Builder aktywny. Wywołano komendę: " .. cmd)
    end,
}
