local t = (rustybot and rustybot.t) or (zuckerbot and zuckerbot.t) or function(k) return k end

local ADMINISTRATOR = 8
local MANAGE_GUILD = 32
local MANAGE_MESSAGES = 8192

local function permission_bits(ctx)
    return math.tointeger(tonumber(ctx.member_permissions or "0")) or 0
end

local function has_permission(ctx, permission)
    local bits = permission_bits(ctx)
    return (bits & ADMINISTRATOR) == ADMINISTRATOR or (bits & permission) == permission
end

local function denied()
    return {
        {
            type = "reply",
            content = t("modules.staff_tools.no_permission", ctx),
            ephemeral = true,
        },
    }
end

local function safe(value, limit)
    return zuckerbot.truncate(zuckerbot.escape_mentions(value or ""), limit or 1500)
end

local function reply(content)
    return {
        {
            type = "reply",
            content = content,
            ephemeral = true,
        },
    }
end

return {
    manifest = {
        id = "staff_tools",
        name = "Staff Communication Tools",
        version = "1.0.0",
        description = t("modules.staff_tools.description"),
        category = "administration",
        default_enabled = false,
        commands = {
            {
                name = "announce",
                description = "Publish formatted announcement to selected channel.",
                dm_permission = false,
                default_member_permissions = "8192",
                options = {
                    { type = "channel", name = "channel", description = "Target channel.", required = true },
                    { type = "string", name = "title", description = "Announcement title.", required = true, min_length = 1, max_length = 120 },
                    { type = "string", name = "message", description = "Announcement message.", required = true, min_length = 1, max_length = 1500 },
                    { type = "string", name = "footer", description = "Opcjonalna stopka.", required = false, max_length = 200 },
                },
            },
            {
                name = "broadcast",
                description = "Dispatches announcement across configured info channels.",
                dm_permission = false,
                default_member_permissions = "32",
                options = {
                    { type = "string", name = "message", description = "Announcement content.", required = true, min_length = 1, max_length = 1600 },
                },
            },
            {
                name = "staffalert",
                description = "Sends flagged alert to private staff channel.",
                dm_permission = false,
                default_member_permissions = "8192",
                options = {
                    {
                        type = "string",
                        name = "severity",
                        description = "Poziom alertu.",
                        required = true,
                        choices = {
                            { name = "Informacja", value = "info" },
                            { name = "Uwaga", value = "warning" },
                            { name = "Pilne", value = "urgent" },
                            { name = "Krytyczne", value = "critical" },
                        },
                    },
                    { type = "string", name = "message", description = "Alert description.", required = true, min_length = 1, max_length = 1400 },
                },
            },
            {
                name = "maintenance",
                description = "Publikuje status prac technicznych.",
                dm_permission = false,
                default_member_permissions = "32",
                options = {
                    { type = "channel", name = "channel", description = "Status channel.", required = true },
                    {
                        type = "string",
                        name = "status",
                        description = "Aktualny status.",
                        required = true,
                        choices = {
                            { name = "Started", value = "started" },
                            { name = "Aktualizacja", value = "update" },
                            { name = "Completed", value = "completed" },
                            { name = "Extended", value = "extended" },
                        },
                    },
                    { type = "string", name = "details", description = "Maintenance details.", required = true, min_length = 1, max_length = 1400 },
                },
            },
            {
                name = "rulespost",
                description = "Publikuje skonfigurowany regulamin na wybranym kanale.",
                dm_permission = false,
                default_member_permissions = "32",
                options = {
                    { type = "channel", name = "channel", description = "Rules channel.", required = true },
                },
            },
            {
                name = "auditnote",
                description = "Adds manual administrative note to bot audit log.",
                dm_permission = false,
                default_member_permissions = "8192",
                options = {
                    { type = "string", name = "subject", description = "Temat notatki.", required = true, min_length = 1, max_length = 120 },
                    { type = "string", name = "details", description = "Note details.", required = true, min_length = 1, max_length = 1500 },
                },
            },
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                staff_channel_id = { type = { "string", "null" }, title = "Private staff channel" },
                broadcast_channel_ids = {
                    type = "array",
                    title = "Broadcast channels",
                    maxItems = 10,
                    items = { type = "string" },
                },
                rules_text = { type = "string", title = "Regulamin", maxLength = 1800 },
                announcement_signature = { type = "string", title = "Announcement signature", maxLength = 120 },
            },
        },
    },

    on_command = function(command, ctx)
        if command == "announce" then
            if not has_permission(ctx, MANAGE_MESSAGES) then return denied() end
            local title = safe(ctx.options.title, 120)
            local message = safe(ctx.options.message, 1500)
            local footer = safe(ctx.options.footer or ctx.config.announcement_signature or "", 200)
            local content = "📢 **" .. title .. "**\n\n" .. message
            if footer ~= "" then content = content .. "\n\n— " .. footer end
            return {
                { type = "reply", content = t("modules.staff_tools.announce_success", ctx), ephemeral = true },
                { type = "send_message", channel_id = ctx.options.channel, content = content },
                { type = "audit", event = "announcement_published", data = { author_id = ctx.user_id, channel_id = ctx.options.channel, title = title } },
            }
        end

        if command == "broadcast" then
            if not has_permission(ctx, MANAGE_GUILD) then return denied() end
            local channels = ctx.config.broadcast_channel_ids or {}
            if #channels == 0 then return reply(t("modules.staff_tools.broadcast_none", ctx)) end
            local content = "📣 **Komunikat serwera**\n\n" .. safe(ctx.options.message, 1600)
            local actions = {
                { type = "reply", content = "Broadcast dispatched to " .. math.min(#channels, 10) .. " channels.", ephemeral = true },
            }
            for index, channel_id in ipairs(channels) do
                if index > 10 then break end
                table.insert(actions, { type = "send_message", channel_id = channel_id, content = content })
            end
            table.insert(actions, { type = "audit", event = "broadcast_published", data = { author_id = ctx.user_id, channel_count = math.min(#channels, 10) } })
            return actions
        end

        if command == "staffalert" then
            if not has_permission(ctx, MANAGE_MESSAGES) then return denied() end
            local channel_id = ctx.config.staff_channel_id
            if not channel_id or channel_id == "" then return reply(t("modules.staff_tools.alert_no_channel", ctx)) end
            local icons = { info = "ℹ️", warning = "⚠️", urgent = "🚨", critical = "🆘" }
            local severity = ctx.options.severity
            local content = string.format("%s **Staff Alert: %s**\nAuthor: <@%s> (`%s`)\n\n%s", icons[severity] or "ℹ️", string.upper(severity), ctx.user_id, ctx.user_id, safe(ctx.options.message, 1400))
            return {
                { type = "reply", content = "Alert has been sent to staff.", ephemeral = true },
                { type = "send_message", channel_id = channel_id, content = content },
                { type = "audit", event = "staff_alert_created", data = { author_id = ctx.user_id, severity = severity } },
            }
        end

        if command == "maintenance" then
            if not has_permission(ctx, MANAGE_GUILD) then return denied() end
            local labels = {
                started = "🛠️ Maintenance started",
                update = "🔄 Aktualizacja prac technicznych",
                completed = "✅ Maintenance completed",
                extended = "⏳ Maintenance extended",
            }
            local status = ctx.options.status
            local content = "**" .. (labels[status] or "Informacja techniczna") .. "**\n\n" .. safe(ctx.options.details, 1400)
            return {
                { type = "reply", content = "Maintenance status published.", ephemeral = true },
                { type = "send_message", channel_id = ctx.options.channel, content = content },
                { type = "audit", event = "maintenance_status_published", data = { author_id = ctx.user_id, status = status, channel_id = ctx.options.channel } },
            }
        end

        if command == "rulespost" then
            if not has_permission(ctx, MANAGE_GUILD) then return denied() end
            local rules = ctx.config.rules_text
            if not rules or rules == "" then return reply("Configure rules in module settings first.") end
            return {
                { type = "reply", content = "Rules have been published.", ephemeral = true },
                { type = "send_message", channel_id = ctx.options.channel, content = "📜 **Regulamin serwera**\n\n" .. safe(rules, 1800) },
                { type = "audit", event = "rules_published", data = { author_id = ctx.user_id, channel_id = ctx.options.channel } },
            }
        end

        if command == "auditnote" then
            if not has_permission(ctx, MANAGE_MESSAGES) then return denied() end
            local subject = safe(ctx.options.subject, 120)
            local details = safe(ctx.options.details, 1500)
            return {
                { type = "reply", content = "Note recorded in audit log.", ephemeral = true },
                { type = "audit", event = "manual_staff_note", data = { author_id = ctx.user_id, subject = subject, details = details } },
            }
        end

        return {}
    end,
}
