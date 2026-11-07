# Moduł: Server Quotes (`quotes`)

Wersja: `1.0.0`  
Kategoria: `fun`  
Autor: **Adrian Banach**  
Silnik: **rusty-discord-bot (Rust + Lua 5.4)**

---

## 📌 Opis
Złote myśli i pamiętne cytaty użytkowników z trwałym zapisem w storage, losowaniem i numeracją.

---

## 📂 Zawartość folderu
- `module.lua` — Pełna logika wykonawcza modułu w piaskownicy Lua (mlua).
- `config.json` — Domyślna konfiguracja startowa modułu.
- `README.md` — Niniejsza dokumentacja techniczna.

---

## ⚙️ Komendy i operacje
- `/quote add author:@użytkownik text:"treść"` — Zapisuje cytat w bazie danych serwera (limit 1–500 znaków) z identyfikatorem UUID i powiązaniem z dodającym (`added_by`). Rejestruje zdarzenie audytowe `quote_add`.
- `/quote random` — Zwraca losowy cytat należący do bieżącego serwera.
- `/quote list` — Zwraca listę ostatnich cytatów serwera z numeracją i identyfikatorami.

---

## 🚀 Rejestracja w `install.xml`
Wpis modułu w pliku instalatora `scripts/install.xml`:
```xml
<module id="quotes" enabled="true" version="1.0.0">
    <name>Server Quotes</name>
    <description>Złote myśli i pamiętne cytaty użytkowników z losowaniem i numeracją.</description>
    <folder>quotes</folder>
    <entry>module.lua</entry>
    <config>config.json</config>
    <author>Adrian Banach</author>
    <category>fun</category>
</module>
```

---

## 🛡️ Bezpieczeństwo i audyt
Wszystkie operacje podlegają izolacji per-gildia (`guild_id`), walidacji snowflake, limitom zapytań (rate limiting `quote`) oraz rejestracji w audycie platformy (`quote_add`, `quote_delete`). Usuwanie cytatów wymaga uprawnienia `MANAGE_MESSAGES` lub `ADMINISTRATOR`.
