local t = (rustybot and rustybot.t) or (zuckerbot and zuckerbot.t) or function(k) return k end

local function reply(content)
    local trunc = (rustybot and rustybot.truncate) or (zuckerbot and zuckerbot.truncate) or function(s, max) return s end
    return {
        {
            type = "reply",
            content = trunc(content, 2000),
            ephemeral = true,
        },
    }
end

return {
    manifest = {
        id = "reddit_feed",
        name = t("modules.reddit_feed.name"),
        version = "1.0.0",
        description = t("modules.reddit_feed.description"),
        category = "integrations",
        default_enabled = true,
        commands = {
            {
                name = "reddit",
                description = t("modules.reddit_feed.commands.reddit.description"),
                dm_permission = false,
                options = {
                    { type = "string", name = "action", description = t("modules.reddit_feed.commands.reddit.options.action.description"), required = false }
                }
            }
        },
    },
    on_command = function(command, ctx)
        local cmd = (type(command) == "string" and command) or (ctx and ctx.command) or "help"
        local context = (type(command) == "table" and command) or ctx or {}
        return reply(t("modules.reddit_feed.active_reply", { cmd = cmd }, context))
    end,
}
