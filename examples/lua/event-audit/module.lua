return {
    manifest = {
        id = "lua_event_audit",
        name = "Lua Event Audit",
        version = "1.0.0",
        description = "Records bounded event metadata when explicitly enabled in configuration.",
        category = "examples",
        events = { "message_create", "guild_member_add", "message_delete" },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                record_events = { type = "boolean" },
            },
            required = { "record_events" },
        },
    },
    on_event = function(event, ctx)
        local config = ctx.config or {}
        if config.record_events ~= true then
            return {}
        end
        local data = ctx.data or {}
        if event == "message_create" or event == "message_delete" then
            return {
                {
                    type = "audit",
                    event = "lua_demo_message_event",
                    data = {
                        event_name = event,
                        message_id = data.message_id,
                        channel_id = ctx.channel_id,
                    },
                },
            }
        end
        if event == "guild_member_add" then
            return {
                {
                    type = "audit",
                    event = "lua_demo_member_join",
                    data = { user_id = ctx.actor_id },
                },
            }
        end
        return {}
    end,
}
