local t = (rustybot and rustybot.t) or (zuckerbot and zuckerbot.t) or function(k) return k end

local function reply(content)
    return {
        {
            type = "reply",
            content = zuckerbot.truncate(content, 2000),
            ephemeral = true,
        },
    }
end

local function safe(value, limit)
    return zuckerbot.truncate(zuckerbot.escape_mentions(value or ""), limit or 1500)
end

local function destination(ctx, key, label)
    local value = ctx.config[key]
    if not value or value == "" then
        return nil, reply(t("modules.community.no_channel", { label = label }, ctx))
    end
    return value, nil
end

local function accepted(message, channel_id, event, data)
    return {
        {
            type = "publish_submission",
            channel_id = channel_id,
            content = data.message,
            success_reply = message,
            audit_event = event,
            audit_data = data.audit or {},
        },
    }
end

return {
    manifest = {
        id = "community",
        name = "Community Hub",
        version = "1.0.0",
        description = t("modules.community.description"),
        category = "community",
        default_enabled = false,
        commands = {
            {
                name = "suggest",
                description = "Forward suggestion to configured community channel.",
                dm_permission = false,
                options = {
                    { type = "string", name = "text", description = "Suggestion content.", required = true, min_length = 5, max_length = 1500 },
                    {
                        type = "string",
                        name = "category",
                        description = "Kategoria sugestii.",
                        required = false,
                        choices = {
                            { name = "Serwer", value = "server" },
                            { name = "Discord", value = "discord" },
                            { name = "Event", value = "event" },
                            { name = "Inne", value = "other" },
                        },
                    },
                },
            },
            {
                name = "report",
                description = "Send private report to server staff team.",
                dm_permission = false,
                options = {
                    { type = "string", name = "reason", description = "Reason for report.", required = true, min_length = 5, max_length = 1200 },
                    { type = "user", name = "user", description = "Optionally reported user.", required = false },
                    { type = "string", name = "evidence", description = "Link, message ID, or extra context.", required = false, max_length = 500 },
                },
            },
            {
                name = "feedback",
                description = "Send rating and feedback to administrators.",
                dm_permission = false,
                options = {
                    { type = "integer", name = "rating", description = "Ocena od 1 do 5.", required = true, min_value = 1, max_value = 5 },
                    { type = "string", name = "message", description = "Feedback content.", required = true, min_length = 3, max_length = 1200 },
                },
            },
            {
                name = "apply",
                description = "Submit an application to selected department or team.",
                dm_permission = false,
                options = {
                    { type = "string", name = "topic", description = "Role, department, or application type.", required = true, min_length = 2, max_length = 80 },
                    { type = "string", name = "application", description = "Application content.", required = true, min_length = 20, max_length = 1500 },
                },
            },
            {
                name = "bugreport",
                description = "Report a bug with reproduction steps.",
                dm_permission = false,
                options = {
                    {
                        type = "string",
                        name = "severity",
                        description = "Bug severity impact.",
                        required = true,
                        choices = {
                            { name = "Niski", value = "low" },
                            { name = "Medium", value = "medium" },
                            { name = "Wysoki", value = "high" },
                            { name = "Krytyczny", value = "critical" },
                        },
                    },
                    { type = "string", name = "description", description = "What is not working?", required = true, min_length = 5, max_length = 900 },
                    { type = "string", name = "steps", description = "How to reproduce the problem?", required = false, max_length = 700 },
                },
            },
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                suggestion_channel_id = { type = { "string", "null" }, title = "Suggestions Channel" },
                report_channel_id = { type = { "string", "null" }, title = "Private Reports Channel" },
                feedback_channel_id = { type = { "string", "null" }, title = "Feedback Channel" },
                application_channel_id = { type = { "string", "null" }, title = "Applications Channel" },
                bug_channel_id = { type = { "string", "null" }, title = "Bug Reports Channel" },
            },
        },
    },

    on_command = function(command, ctx)
        if command == "suggest" then
            local channel_id, error_reply = destination(ctx, "suggestion_channel_id", "sugestie")
            if not channel_id then return error_reply end
            local category = ctx.options.category or "other"
            local text = safe(ctx.options.text)
            return accepted(
                "Suggestion has been submitted. Thank you!",
                channel_id,
                "suggestion_created",
                {
                    message = string.format("💡 **Nowa sugestia**\nAuthor: <@%s> (`%s`)\nKategoria: `%s`\n\n%s", ctx.user_id, ctx.user_id, category, text),
                    audit = { author_id = ctx.user_id, category = category },
                }
            )
        end

        if command == "report" then
            local channel_id, error_reply = destination(ctx, "report_channel_id", "reports")
            if not channel_id then return error_reply end
            local target = ctx.options.user
            local target_line = target and ("<@" .. target .. "> (`" .. target .. "`)") or "Nie wskazano"
            local evidence = safe(ctx.options.evidence or "Nie podano", 500)
            local reason = safe(ctx.options.reason, 1200)
            return accepted(
                "Report has been safely delivered to staff.",
                channel_id,
                "user_report_created",
                {
                    message = string.format("🚨 **Private Report**\nReporter: <@%s> (`%s`)\nTarget: %s\nEvidence/context: %s\n\n**Reason:**\n%s", ctx.user_id, ctx.user_id, target_line, evidence, reason),
                    audit = { reporter_id = ctx.user_id, target_user_id = target, has_evidence = ctx.options.evidence ~= nil },
                }
            )
        end

        if command == "feedback" then
            local channel_id, error_reply = destination(ctx, "feedback_channel_id", "feedback")
            if not channel_id then return error_reply end
            local rating = math.tointeger(ctx.options.rating) or 1
            local stars = string.rep("⭐", rating)
            local message = safe(ctx.options.message, 1200)
            return accepted(
                "Feedback has been delivered. Thank you!",
                channel_id,
                "feedback_created",
                {
                    message = string.format("📝 **Nowy feedback**\nAuthor: <@%s> (`%s`)\nOcena: %s (`%d/5`)\n\n%s", ctx.user_id, ctx.user_id, stars, rating, message),
                    audit = { author_id = ctx.user_id, rating = rating },
                }
            )
        end

        if command == "apply" then
            local channel_id, error_reply = destination(ctx, "application_channel_id", "podania")
            if not channel_id then return error_reply end
            local topic = safe(ctx.options.topic, 80)
            local application = safe(ctx.options.application, 1500)
            return accepted(
                "Application has been submitted for review.",
                channel_id,
                "application_created",
                {
                    message = string.format("📨 **Nowe podanie**\nApplicant: <@%s> (`%s`)\nTemat: **%s**\n\n%s", ctx.user_id, ctx.user_id, topic, application),
                    audit = { applicant_id = ctx.user_id, topic = topic },
                }
            )
        end

        if command == "bugreport" then
            local channel_id, error_reply = destination(ctx, "bug_channel_id", "bug reports")
            if not channel_id then return error_reply end
            local severity = ctx.options.severity
            local description = safe(ctx.options.description, 900)
            local steps = safe(ctx.options.steps or "Nie podano", 700)
            return accepted(
                "Bug report has been submitted.",
                channel_id,
                "bug_report_created",
                {
                    message = string.format("🐞 **Bug Report**\nAuthor: <@%s> (`%s`)\nSeverity: `%s`\n\n**Description:**\n%s\n\n**Kroki odtworzenia:**\n%s", ctx.user_id, ctx.user_id, severity, description, steps),
                    audit = { author_id = ctx.user_id, severity = severity },
                }
            )
        end

        return {}
    end,
}
