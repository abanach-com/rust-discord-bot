local t = (rustybot and rustybot.t) or (zuckerbot and zuckerbot.t) or function(k) return k end

local function missing(message)
    return {
        {
            type = "reply",
            content = message,
            ephemeral = true,
        },
    }
end

return {
    manifest = {
        id = "moderation",
        name = "Moderation",
        version = "0.4.0",
        description = "Safe moderation actions, persistent numbered cases, warnings, and audit logging.",
        category = "safety",
        default_enabled = true,
        commands = {
            {
                name = "moderate",
                description = "Execute timeout, kick, ban, softban, unban, or message purge.",
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "string",
                        name = "action",
                        description = "Akcja moderacyjna.",
                        required = true,
                        choices = {
                            { name = "Timeout", value = "timeout" },
                            { name = "Kick", value = "kick" },
                            { name = "Ban", value = "ban" },
                            { name = "Softban", value = "softban" },
                            { name = "Unban", value = "unban" },
                            { name = "Purge", value = "purge" },
                        },
                    },
                    {
                        type = "user",
                        name = "user",
                        description = "Target user for timeout, kick, ban, softban, or unban.",
                        required = false,
                    },
                    {
                        type = "integer",
                        name = "seconds",
                        description = "Timeout duration in seconds.",
                        required = false,
                        min_value = 1,
                        max_value = 2419200,
                    },
                    {
                        type = "integer",
                        name = "delete_days",
                        description = "Days of message history to delete (0-7).",
                        required = false,
                        min_value = 0,
                        max_value = 7,
                    },
                    {
                        type = "integer",
                        name = "amount",
                        description = "Number of messages to purge.",
                        required = false,
                        min_value = 1,
                        max_value = 100,
                    },
                    {
                        type = "string",
                        name = "reason",
                        description = "Reason visible in Discord audit log.",
                        required = false,
                        max_length = 512,
                    },
                    {
                        type = "string",
                        name = "severity",
                        description = "Case severity level.",
                        required = false,
                        choices = {
                            { name = "Informacyjny", value = "info" },
                            { name = "Niski", value = "low" },
                            { name = "Medium", value = "medium" },
                            { name = "Wysoki", value = "high" },
                            { name = "Krytyczny", value = "critical" },
                        },
                    },
                    {
                        type = "integer",
                        name = "points",
                        description = "Punkty karne (0-1000).",
                        required = false,
                        min_value = 0,
                        max_value = 1000,
                    },
                },
            },
            {
                name = "warn",
                description = "Issue an official warning and record a moderation case.",
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "user",
                        name = "user",
                        description = "User to warn.",
                        required = true,
                    },
                    {
                        type = "string",
                        name = "reason",
                        description = "Warning reason.",
                        required = true,
                        max_length = 512,
                    },
                    {
                        type = "string",
                        name = "severity",
                        description = "Case severity level.",
                        required = false,
                        choices = {
                            { name = "Niski", value = "low" },
                            { name = "Medium", value = "medium" },
                            { name = "Wysoki", value = "high" },
                            { name = "Krytyczny", value = "critical" },
                        },
                    },
                    {
                        type = "integer",
                        name = "points",
                        description = "Penalty points (default 1).",
                        required = false,
                        min_value = 0,
                        max_value = 1000,
                    },
                },
            },
            {
                name = "case",
                description = "Manage moderation cases: view, note, void, and view stats.",
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "string",
                        name = "action",
                        description = "Moderation case action.",
                        required = true,
                        choices = {
                            { name = "View case", value = "view" },
                            { name = "Lista spraw", value = "list" },
                            { name = "Edit case", value = "edit" },
                            { name = "Add note", value = "note" },
                            { name = "Void case", value = "void" },
                            { name = "Restore case", value = "restore" },
                            { name = "Statystyki moderacji", value = "stats" },
                        },
                    },
                    {
                        type = "integer",
                        name = "case_number",
                        description = "Case number.",
                        required = false,
                        min_value = 1,
                    },
                    {
                        type = "user",
                        name = "user",
                        description = "Target user.",
                        required = false,
                    },
                    {
                        type = "string",
                        name = "text",
                        description = "Note content or void reason.",
                        required = false,
                        max_length = 1000,
                    },
                    {
                        type = "boolean",
                        name = "visible",
                        description = "Whether note is visible to sanctioned user.",
                        required = false,
                    },
                    {
                        type = "integer",
                        name = "version",
                        description = "Case version (for void/restore).",
                        required = false,
                        min_value = 1,
                    },
                },
            },
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                default_timeout_seconds = {
                    type = "integer",
                    minimum = 1,
                    maximum = 2419200,
                    default = 600,
                },
            },
        },
    },

    on_command = function(command, ctx)
        local accepted = {
            type = "reply",
            content = t("modules.moderation.accepted", ctx),
            ephemeral = true,
        }

        if command == "warn" then
            local user_id = ctx.options.user
            if not user_id then
                return missing("You must specify a user to warn.")
            end
            local reason = ctx.options.reason or "Brak powodu"
            local severity = ctx.options.severity or "medium"
            local points = tonumber(ctx.options.points) or 1

            return {
                accepted,
                {
                    type = "moderation_case",
                    operation = {
                        type = "create",
                        subject_user_id = user_id,
                        kind = "warning",
                        reason = reason,
                        severity = severity,
                        points = points,
                        visible_to_subject = true,
                    },
                },
            }
        end

        if command == "case" then
            local action = ctx.options.action
            if action == "view" then
                local num = tonumber(ctx.options.case_number)
                if not num then
                    return missing("You must specify a case number (case_number).")
                end
                return {
                    accepted,
                    {
                        type = "moderation_case",
                        operation = {
                            type = "view",
                            case_number = num,
                        },
                    },
                }
            elseif action == "list" then
                local user_id = ctx.options.user
                return {
                    accepted,
                    {
                        type = "moderation_case",
                        operation = {
                            type = "list",
                            subject_user_id = user_id,
                            limit = 15,
                            visible_to_subject_only = false,
                        },
                    },
                }
            elseif action == "edit" then
                local num = tonumber(ctx.options.case_number)
                local text = ctx.options.text
                if not num then
                    return missing("Edycja wymaga podania numeru sprawy (case_number).")
                end
                local version = tonumber(ctx.options.version) or 0
                return {
                    accepted,
                    {
                        type = "moderation_case",
                        operation = {
                            type = "edit",
                            case_number = num,
                            reason = text,
                            expected_version = version,
                        },
                    },
                }
            elseif action == "note" then
                local num = tonumber(ctx.options.case_number)
                local text = ctx.options.text
                if not num or not text or text == "" then
                    return missing("Dodanie notatki wymaga numeru sprawy i tekstu (case_number, text).")
                end
                local visible = ctx.options.visible == true
                return {
                    accepted,
                    {
                        type = "moderation_case",
                        operation = {
                            type = "add_note",
                            case_number = num,
                            body = text,
                            visible_to_subject = visible,
                        },
                    },
                }
            elseif action == "void" then
                local num = tonumber(ctx.options.case_number)
                local reason = ctx.options.text or "Voided by moderator decision"
                local version = tonumber(ctx.options.version) or 0
                if not num then
                    return missing("Voiding requires a case number (case_number).")
                end
                return {
                    accepted,
                    {
                        type = "moderation_case",
                        operation = {
                            type = "void",
                            case_number = num,
                            reason = reason,
                            expected_version = version,
                        },
                    },
                }
            elseif action == "restore" then
                local num = tonumber(ctx.options.case_number)
                local version = tonumber(ctx.options.version) or 0
                if not num then
                    return missing("Restoring requires a case number (case_number).")
                end
                return {
                    accepted,
                    {
                        type = "moderation_case",
                        operation = {
                            type = "restore",
                            case_number = num,
                            expected_version = version,
                        },
                    },
                }
            elseif action == "stats" then
                local user_id = ctx.options.user
                return {
                    accepted,
                    {
                        type = "moderation_case",
                        operation = {
                            type = "stats",
                            moderator_user_id = user_id,
                        },
                    },
                }
            end

            return missing("Nieznana operacja na sprawie.")
        end

        if command ~= "moderate" then
            return {}
        end

        local action = ctx.options.action
        local user_id = ctx.options.user
        local reason = ctx.options.reason or "Brak powodu"
        local severity = ctx.options.severity or "medium"
        local points = tonumber(ctx.options.points) or 0

        if action == "purge" then
            return {
                accepted,
                {
                    type = "purge",
                    amount = tonumber(ctx.options.amount) or 10,
                },
            }
        end

        if not user_id then
            return missing("For this action you must specify a user.")
        end

        if action == "timeout" then
            local default_seconds = tonumber(ctx.config.default_timeout_seconds) or 600
            local seconds = tonumber(ctx.options.seconds) or default_seconds
            return {
                accepted,
                {
                    type = "moderation_case",
                    operation = {
                        type = "create",
                        subject_user_id = user_id,
                        kind = "timeout",
                        reason = reason,
                        severity = severity,
                        points = points > 0 and points or 2,
                        visible_to_subject = true,
                        duration_seconds = seconds,
                    },
                },
            }
        end

        if action == "kick" then
            return {
                accepted,
                {
                    type = "moderation_case",
                    operation = {
                        type = "create",
                        subject_user_id = user_id,
                        kind = "kick",
                        reason = reason,
                        severity = severity,
                        points = points > 0 and points or 5,
                        visible_to_subject = true,
                    },
                },
            }
        end

        if action == "ban" then
            local delete_days = ctx.options.delete_days ~= nil and tonumber(ctx.options.delete_days) or nil
            return {
                accepted,
                {
                    type = "moderation_case",
                    operation = {
                        type = "create",
                        subject_user_id = user_id,
                        kind = "ban",
                        reason = reason,
                        severity = severity,
                        points = points > 0 and points or 10,
                        visible_to_subject = true,
                        delete_days = delete_days,
                    },
                },
            }
        end

        if action == "softban" then
            local delete_days = ctx.options.delete_days ~= nil and tonumber(ctx.options.delete_days) or nil
            return {
                accepted,
                {
                    type = "moderation_case",
                    operation = {
                        type = "create",
                        subject_user_id = user_id,
                        kind = "softban",
                        reason = reason,
                        severity = severity,
                        points = points > 0 and points or 3,
                        visible_to_subject = true,
                        delete_days = delete_days,
                    },
                },
            }
        end

        if action == "unban" then
            return {
                accepted,
                {
                    type = "moderation_case",
                    operation = {
                        type = "create",
                        subject_user_id = user_id,
                        kind = "unban",
                        reason = reason,
                        severity = "info",
                        points = 0,
                        visible_to_subject = true,
                    },
                },
            }
        end

        return missing("Nieznana akcja moderacyjna.")
    end,
}
