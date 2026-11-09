local t = (rustybot and rustybot.t) or (zuckerbot and zuckerbot.t) or function(k) return k end

return {
    manifest = {
        id = "music",
        name = t("modules.music.name"),
        version = "0.3.0",
        description = t("modules.music.description"),
        category = "voice",
        default_enabled = true,
        commands = {
            {
                name = "music",
                description = t("modules.music.commands.music.description"),
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "string",
                        name = "action",
                        description = t("modules.music.commands.music.options.action.description"),
                        required = true,
                        choices = {
                            { name = "Play", value = "play" },
                            { name = "Pause", value = "pause" },
                            { name = "Resume", value = "resume" },
                            { name = "Skip", value = "skip" },
                            { name = "Stop", value = "stop" },
                            { name = "Leave", value = "leave" },
                            { name = "Volume", value = "volume" },
                            { name = "Loop", value = "loop" },
                            { name = "Now Playing", value = "now_playing" },
                            { name = "Queue", value = "queue" },
                        },
                    },
                    {
                        type = "string",
                        name = "query",
                        description = t("modules.music.commands.music.options.query.description"),
                        required = false,
                        max_length = 512,
                    },
                },
            },
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                announce_tracks = {
                    type = "boolean",
                    default = true,
                },
                default_volume = {
                    type = "number",
                    minimum = 0.0,
                    maximum = 1.0,
                    default = 0.5,
                },
            },
        },
    },

    on_command = function(command, ctx)
        if command ~= "music" then
            return {}
        end

        local operation = ctx.options.action
        if operation == "play" and (not ctx.options.query or ctx.options.query == "") then
            return {
                {
                    type = "reply",
                    content = t("modules.music.missing_play_query", ctx),
                    ephemeral = true,
                },
            }
        end
        if operation == "volume" and (not ctx.options.query or ctx.options.query == "") then
            return {
                {
                    type = "reply",
                    content = t("modules.music.missing_volume_level", ctx),
                    ephemeral = true,
                },
            }
        end

        return {
            {
                type = "reply",
                content = t("modules.music.accepted", ctx),
                ephemeral = true,
            },
            {
                type = "music",
                operation = operation,
                query = ctx.options.query,
            },
        }
    end,
}
