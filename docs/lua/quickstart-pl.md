# Lua: szybki start po polsku

Lua opisuje zachowanie modułu i zwraca żądania akcji. Rust sprawdza je, egzekwuje uprawnienia i wykonuje operacje Discord oraz zapis danych.
Ten przewodnik dotyczy API 1; pełny kontrakt znajdziesz w [spisie rozdziałów](README.md) i [angielskim szybkim starcie](quickstart.md).

## 1. Przygotuj paczkę

Paczka zawiera `module.lua` i `config.json`; wybiera ją `install.xml` w katalogu wskazanym przez `SCRIPTS_DIR`.
Gotowe paczki to [hello](../../examples/lua/hello/module.lua), [nested-router](../../examples/lua/nested-router/module.lua) i [event-audit](../../examples/lua/event-audit/module.lua).
Każda ma własne ustawienia i wspólny [przykładowy instalator](../../examples/lua/install.xml), niezależny od produkcyjnego `scripts/install.xml`.

## 2. Zwróć tabelę modułu z manifestem i funkcją

Poniżej krótszy wariant `hello`. Plik musi zwrócić tabelę z polem `manifest` oraz używanymi callbackami.
Sam manifest albo funkcja globalna `on_command` nie wystarczą: host odczytuje funkcję z tabeli zwróconej przez plik.

```lua
local manifest = {
    id = "lua_hello", name = "Lua Hello", version = "1.0.0",
    description = "Pierwszy moduł Lua", category = "examples",
    default_enabled = true,
    commands = {{
        name = "lua-hello", description = "Wyświetl powitanie",
        options = {{
            type = "string", name = "text", description = "Treść powitania",
            required = false, min_length = 1, max_length = 100
        }}
    }},
    config_schema = {
        type = "object", additionalProperties = false,
        properties = {
            greeting = { type = "string", minLength = 1, maxLength = 80 }
        },
        required = { "greeting" }
    }
}

local function on_command(command, ctx)
    if command ~= "lua-hello" then return {} end
    local config = type(ctx.config) == "table" and ctx.config or {}
    local text = (ctx.options or {}).text or config.greeting or "Cześć"
    local content = rustybot.truncate(rustybot.escape_mentions(text), 1800)
    return {{ type = "reply", content = content, ephemeral = true }}
end

return { manifest = manifest, on_command = on_command }
```

Odpowiadający mu `config.json`:

```json
{"greeting": "Cześć z Lua"}
```

`config_schema` używa nazw JSON Schema, np. `minLength`; opcje komend mają osobne pola, np. `min_length`.
Loader kompiluje schemat, lecz nie sprawdza nim domyślnego JSON. Zapisana konfiguracja gildii zastępuje całe defaults; nie jest z nimi scalana.
W module zdarzeń dodaj `events` do manifestu i wyeksportuj `on_event(event, ctx)` jako pole tej samej tabeli.

## 3. Załaduj trzy przykłady na gildii developerskiej

Przykładowy instalator wybiera wszystkie trzy paczki. Korzeń XML to `installer`, a `modules` jest jego dzieckiem:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<installer version="1.0.0">
  <bot name="Przykłady Lua" />
  <modules>
    <module id="lua_hello" enabled="true">
      <folder>hello</folder><entry>module.lua</entry><config>config.json</config>
    </module>
    <module id="lua_router" enabled="true">
      <folder>nested-router</folder><entry>module.lua</entry><config>config.json</config>
    </module>
    <module id="lua_event_audit" enabled="true">
      <folder>event-audit</folder><entry>module.lua</entry><config>config.json</config>
    </module>
  </modules>
</installer>
```

Zachowaj zgodność `module.id` instalatora z `manifest.id`. Wyłączenie wpisu w XML usuwa paczkę ze zbioru ładowanych modułów.
Skonfiguruj środowisko bota zgodnie z [README projektu](../../README.md), użyj aplikacji testowej i zastąp poniższe ID identyfikatorem własnej gildii.
Z katalogu głównego repozytorium uruchom:

```bash
export DISCORD_DEV_GUILD_ID=123456789012345678
SCRIPTS_DIR=examples/lua cargo run --release
```

Rejestracja zastępuje zestaw komend aplikacji w wybranym zakresie. Podane ID ogranicza rejestrację do gildii developerskiej.
Sprawdź `/lua-hello`, `/lua-hello text:Witaj`, `/lua-route echo text:Test` i `/lua-route inspect user person:@użytkownik`.
`event-audit` domyślnie ma `record_events=false`; włączenie tej opcji pozwala sprawdzić audyt zdarzeń bez wysyłania wiadomości.

## 4. Czytaj zagnieżdżone opcje

Argument callbacka `command` zawiera główną nazwę komendy. Podkomendę wskazuje obecna gałąź `ctx.options`:

```lua
local options = ctx.options or {}
if options.echo then
    local text = options.echo.text
    -- /lua-route echo text:Test
elseif options.inspect and options.inspect.user then
    local user_id = options.inspect.user.person
    -- /lua-route inspect user person:@użytkownik
end
```

Nie ma pola `options.subcommand`, a argumenty podkomendy nie są spłaszczone. Pełny dispatcher jest w [nested-router](../../examples/lua/nested-router/module.lua).
ID Discorda są ciągami znaków; nie zmieniaj ich przez `tonumber`. Dla opcji logicznej sprawdzaj `~= nil`, jeśli `false` ma własne znaczenie.

## 5. Korzystaj z usług i helperów hosta

Każde wywołanie ma nową maszynę Lua: lokalna tabela ani zmienna globalna nie zachowuje stanu między komendami.
Nie ma ogólnego magazynu klucz–wartość, dostępu do SQL/HTTP/systemu plików ani funkcji `require`. Trwałe operacje zapewniają konkretne usługi Rust.
Akcję `progression` może wykonywać moduł o ID `progression`, `scheduler` — `scheduler`, a `ticket` — `tickets`; dwie ostatnie usługi wymagają komendy.
Dodanie dowolnego `capabilities` do manifestu nie nadaje dostępu. Usługi mają także własne kontrole użytkownika, gildii i uprawnień.
Helpery obejmują `rustybot.escape_mentions`, `truncate`, `unix_time`, `t` i `translate`; alias `zuckerbot` udostępnia tę samą tabelę.
Wywołuj `rustybot.truncate` po dodaniu tekstu i escapowaniu wzmianek. Lua `#tekst` liczy bajty, a limit treści hosta — znaki Unicode.

## 6. Sprawdź wynik i granice testu

Callback zwraca zwartą listę od indeksu 1: `{{type="reply", content="OK"}}`; `{}` oznacza brak akcji.
Limit to 25 akcji i 1–2000 znaków zwykłej wiadomości. Domyślne limity VM to 8 MiB oraz 500 000 instrukcji; szczegóły są w [bezpieczeństwie](security-errors.md).
Host waliduje listę przed wykonaniem, ale późniejsze operacje nie tworzą jednej transakcji. Pierwsza odpowiedź komendy może poprzedzić pozostałe skutki.
Sprawdź składnię Lua 5.4, schemat z domyślną konfiguracją i wynik callbacka dla konkretnych kontekstów. Test silnika uruchomisz przez `cargo test --test lua_engine`.
Sam test Lua ze stubami helperów nie potwierdza deserializacji Rust, uprawnień, zapisu w bazie ani dostarczenia na Discord. Te warstwy wymagają osobnych testów.

## Pełna dokumentacja: 15 rozdziałów

1. [Runtime i cykl życia](runtime.md)
2. [Kompletny szybki start](quickstart.md)
3. [Manifest, komendy i instalator](manifests.md)
4. [Callbacki, konteksty i zdarzenia](contexts-events.md)
5. [Wszystkie akcje](actions.md)
6. [Stan i usługi hosta](services.md)
7. [Punkty, poziomy i ekonomia](progression.md)
8. [Harmonogram zadań](scheduler.md)
9. [Tickety](tickets.md)
10. [Sprawy moderacyjne](moderation-cases.md)
11. [Helpery i tłumaczenia](helpers-i18n.md)
12. [Bezpieczeństwo i błędy](security-errors.md)
13. [Testy i diagnozowanie](testing.md)
14. [Praktyczne wzorce](recipes.md)
15. [Obsługiwane funkcje i ograniczenia](support-matrix.md)
