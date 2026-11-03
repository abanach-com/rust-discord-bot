local t = (rustybot and rustybot.t) or (zuckerbot and zuckerbot.t) or function(k) return k end

return {
    manifest = {
        id = "welcome",
        name = "Welcome and Goodbye",
        version = "1.0.0",
        description = t("modules.welcome.description"),
        category = "automation",
        default_enabled = false,
        events = {
            "guild_member_add",
            "guild_member_remove",
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                channel_id = {
                    type = "string",
                    title = "Welcome channel ID",
                },
                message = {
                    type = "string",
                    title = "Welcome message content",
                    default = "Welcome {user} to {server}! You are member #{member_count}.",
                    maxLength = 1800,
                },
                dm_welcome = {
                    type = "boolean",
                    title = "Send welcome in direct message (DM)",
                    default = false,
                },
                dm_message = {
                    type = "string",
                    title = "Welcome message content w DM",
                    default = "Welcome to {server}! Please review the rules and enjoy your stay.",
                    maxLength = 1800,
                },
                auto_roles = {
                    type = "array",
                    title = "Roles automatically granted on join",
                    items = { type = "string" },
                    maxItems = 10,
                },
                send_goodbye = {
                    type = "boolean",
                    title = "Send goodbye messages",
                    default = false,
                },
                goodbye_channel_id = {
                    type = "string",
                    title = "Goodbye channel ID",
                },
                goodbye_message = {
                    type = "string",
                    title = "Goodbye message content",
                    default = "Goodbye {username}. We will miss you!",
                    maxLength = 1800,
                },
            },
        },
    },

    on_event = function(event, ctx)
        if event == "guild_member_add" then
            local actions = {}
            local user = ctx.data.user or {}
            local user_id = tostring(user.id or "")
            if user_id == "" then return {} end

            local server_name = tostring(ctx.data.guild_name or "naszym serwerze")
            local member_count = tostring(ctx.data.member_count or "")
            local safe_mention = "<@" .. user_id .. ">"
            local raw_display = (type(user.global_name) == "string" and user.global_name ~= "") and user.global_name
                or (type(user.name) == "string" and user.name ~= "") and user.name
                or ""
            local safe_name = zuckerbot.escape_mentions(raw_display)

            local function format_text(template)
                local content = string.gsub(template, "{user}", function() return safe_mention end)
                content = string.gsub(content, "{username}", function() return safe_name end)
                content = string.gsub(content, "{server}", function() return server_name end)
                content = string.gsub(content, "{member_count}", function() return member_count end)
                return zuckerbot.truncate(content, 2000)
            end

            -- Welcome Channel
            if ctx.config.channel_id and ctx.config.channel_id ~= "" then
                local template = ctx.config.message or "Witaj {user} na serwerze {server}."
                table.insert(actions, {
                    type = "send_message",
                    channel_id = ctx.config.channel_id,
                    content = format_text(template),
                })
                table.insert(actions, {
                    type = "audit",
                    event = "welcome_sent",
                    data = {
                        user_id = user_id,
                        channel_id = ctx.config.channel_id,
                    },
                })
            end

            -- Direct message (DM)
            if ctx.config.dm_welcome == true and not user.bot then
                local dm_template = ctx.config.dm_message or "Welcome to {server}! Please review the rules and enjoy your stay."
                table.insert(actions, {
                    type = "send_direct_message",
                    user_id = user_id,
                    content = format_text(dm_template),
                })
            end

            -- Auto roles for new member
            if type(ctx.config.auto_roles) == "table" then
                for _, role_id in ipairs(ctx.config.auto_roles) do
                    local rid = tostring(role_id or "")
                    if rid ~= "" then
                        table.insert(actions, {
                            type = "add_role",
                            user_id = user_id,
                            role_id = rid,
                            reason = "Auto welcome role for new member",
                        })
                    end
                end
            end

            return actions
        end

        if event == "guild_member_remove" then
            if ctx.config.send_goodbye ~= true then return {} end

            local target_channel = ctx.config.goodbye_channel_id
            if not target_channel or target_channel == "" then
                target_channel = ctx.config.channel_id
            end
            if not target_channel or target_channel == "" then return {} end

            local user = ctx.data.user or {}
            local user_id = tostring(user.id or "")
            local server_name = tostring(ctx.data.guild_name or "naszym serwerze")
            local member_count = tostring(ctx.data.member_count or "")
            local raw_goodbye_display = (type(user.global_name) == "string" and user.global_name ~= "") and user.global_name
                or (type(user.name) == "string" and user.name ~= "") and user.name
                or "member"
            local safe_name = zuckerbot.escape_mentions(raw_goodbye_display)

            local template = ctx.config.goodbye_message or "Goodbye {username}. We will miss you!"
            local content = string.gsub(template, "{username}", function() return safe_name end)
            content = string.gsub(content, "{server}", function() return server_name end)
            content = string.gsub(content, "{member_count}", function() return member_count end)
            content = zuckerbot.truncate(content, 2000)

            return {
                {
                    type = "send_message",
                    channel_id = target_channel,
                    content = content,
                },
                {
                    type = "audit",
                    event = "goodbye_sent",
                    data = {
                        user_id = user_id,
                        channel_id = target_channel,
                    },
                },
            }
        end

        return {}
    end,
}
