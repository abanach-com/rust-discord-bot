return {
    manifest = {
        id = "achievements",
        name = "Achievements & Badges",
        version = "1.0.0",
        description = "Odznaki i trofea za aktywność na czacie, gry, ekonomię i staż.",
        category = "progression",
        default_enabled = true,
        commands = {
            {
                name = "achievements",
                description = "Przeglądaj i odbieraj odznaki za osiągnięcia na serwerze.",
                integration_types = { "guild" },
                contexts = { "guild" },
                options = {
                    {
                        type = "subcommand",
                        name = "list",
                        description = "Zobacz pełną listę wszystkich dostępnych odznak.",
                        options = {},
                    },
                    {
                        type = "subcommand",
                        name = "view",
                        description = "Sprawdź odznaki zdobyte przez użytkownika.",
                        options = {
                            {
                                type = "user",
                                name = "user",
                                description = "Użytkownik do sprawdzenia.",
                                required = false,
                            },
                        },
                    },
                    {
                        type = "subcommand",
                        name = "claim",
                        description = "Odbierz nagrodę monetarną i XP za odblokowane osiągnięcie.",
                        options = {
                            {
                                type = "string",
                                name = "badge_id",
                                description = "Identyfikator odznaki do odebrania.",
                                required = true,
                            },
                        },
                    },
                },
            },
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {
                badges = { type = "array", default = {} },
            },
        },
    },

    on_command = function(command, ctx)
        if command ~= "achievements" then
            return {}
        end

        local options = ctx.options or {}
        local sub = options.subcommand or "list"
        local user_id = ctx.user_id

        if sub == "list" then
            local text = "🏆 **Dostępne Osiągnięcia i Odznaki Serwerowe:**\n\n" ..
                "1. 👣 **Pierwszy krok** (`first_message`) — Nagroda: 50 monet, 100 XP\n" ..
                "2. 🎖️ **Doświadczony** (`level_5`) — Nagroda: 150 monet, 250 XP\n" ..
                "3. 💰 **Milioner** (`rich`) — Nagroda: 300 monet, 500 XP\n" ..
                "4. 🧠 **Mistrz Quizu** (`trivia_master`) — Nagroda: 200 monet, 350 XP\n" ..
                "5. 🦉 **Nocny Marek** (`night_owl`) — Nagroda: 100 monet, 150 XP\n\n" ..
                "Użyj `/achievements claim badge_id:nazwa` aby odebrać nagrodę!"
            return {
                {
                    type = "reply",
                    content = text,
                    ephemeral = false,
                },
            }
        end

        if sub == "view" then
            local target = options.user or user_id
            return {
                {
                    type = "reply",
                    content = string.format("🎖️ **Odznaki użytkownika <@%s>:**\n• 👣 Pierwszy krok (Odblokowane!)\n• 🦉 Nocny Marek (Odblokowane!)\n• 🎖️ Doświadczony (W toku — 3/5)", target),
                    ephemeral = false,
                },
            }
        end

        if sub == "claim" then
            local bid = options.badge_id or "first_message"
            return {
                {
                    type = "progression",
                    operation = {
                        type = "adjust_member_progress",
                        user_id = user_id,
                        xp_delta = 100,
                        coins_delta = 50,
                        reputation_delta = 1,
                        reason = "Osiągnięcie: " .. bid,
                    },
                },
                {
                    type = "reply",
                    content = string.format("🎉 Gratulacje! Odebrałeś nagrodę za odznakę **%s**: +50 monet 🪙, +100 XP 🌟, +1 Reputacja ✨!", bid),
                    ephemeral = false,
                },
                {
                    type = "audit",
                    event = "achievement_claimed",
                    data = {
                        user_id = user_id,
                        badge_id = bid,
                    },
                },
            }
        end

        return {}
    end,
}
