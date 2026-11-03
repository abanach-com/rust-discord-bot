local t = (rustybot and rustybot.t) or (zuckerbot and zuckerbot.t) or function(k) return k end

local function reply(content, ephemeral)
    local trunc = (rustybot and rustybot.truncate) or (zuckerbot and zuckerbot.truncate) or function(s, max) return s end
    return {
        {
            type = "reply",
            content = trunc(content, 2000),
            ephemeral = ephemeral,
        },
    }
end

local function show(ctx, key, heading)
    local value = ctx.config and ctx.config[key]
    if not value or value == "" then
        return reply(t("modules.server_info.not_filled", { heading = heading }, ctx), true)
    end
    local prefix = ctx.config.show_headings == false and "" or (heading .. "\n\n")
    return reply(prefix .. value, ctx.config.ephemeral_responses == true)
end

return {
    manifest = {
        id = "server_info",
        name = t("modules.server_info.name"),
        version = "1.0.0",
        description = t("modules.server_info.description"),
        category = "information",
        default_enabled = false,
        commands = {
            { name = "rules", description = t("modules.server_info.commands.rules.description"), dm_permission = false },
            { name = "faq", description = t("modules.server_info.commands.faq.description"), dm_permission = false },
            { name = "links", description = t("modules.server_info.commands.links.description"), dm_permission = false },
            { name = "supportinfo", description = t("modules.server_info.commands.supportinfo.description"), dm_permission = false },
            { name = "staff", description = t("modules.server_info.commands.staff.description"), dm_permission = false },
            { name = "schedule", description = t("modules.server_info.commands.schedule.description"), dm_permission = false },
            { name = "aboutserver", description = t("modules.server_info.commands.aboutserver.description"), dm_permission = false },
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                rules = { type = "string", title = "Rules", maxLength = 1900 },
                faq = { type = "string", title = "FAQ", maxLength = 1900 },
                links = { type = "string", title = "Official Links", maxLength = 1900 },
                support = { type = "string", title = "Support Info", maxLength = 1900 },
                staff = { type = "string", title = "Staff", maxLength = 1900 },
                schedule = { type = "string", title = "Schedule", maxLength = 1900 },
                about = { type = "string", title = "About Server", maxLength = 1900 },
                show_headings = { type = "boolean", default = true },
                ephemeral_responses = { type = "boolean", default = false },
            },
        },
    },

    on_command = function(command, ctx)
        if command == "rules" then return show(ctx, "rules", "📜 **Server Rules**") end
        if command == "faq" then return show(ctx, "faq", "❓ **Frequently Asked Questions**") end
        if command == "links" then return show(ctx, "links", "🔗 **Official Links**") end
        if command == "supportinfo" then return show(ctx, "support", "🛟 **Help & Support**") end
        if command == "staff" then return show(ctx, "staff", "🛡️ **Server Team**") end
        if command == "schedule" then return show(ctx, "schedule", "📅 **Schedule**") end
        if command == "aboutserver" then return show(ctx, "about", "ℹ️ **About Server**") end
        return {}
    end,
}
