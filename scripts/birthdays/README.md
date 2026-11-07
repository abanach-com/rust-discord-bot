# Moduł: Birthday Tracker (`birthdays`)

Wersja: `1.0.0`  
Kategoria: `community`  
Autor: **Adrian Banach**  
Silnik: **rusty-discord-bot (Rust + Lua 5.4)**

---

## 📌 Opis
Śledzenie urodzin członków serwera, kalendarz nadchodzących jubileuszy oraz trwałe przechowywanie dat w relacyjnym magazynie danych PostgreSQL i MemoryStore.

---

## 📂 Komendy slash
- `/birthday set day:<1-31> month:<1-12>` — Zapisuje lub aktualizuje Twoją datę urodzin (walidacja poprawności dni w miesiącu z obsługą 29 lutego).
- `/birthday check [user:@user]` — Wyświetla datę urodzin Twoją lub wskazanego użytkownika.
- `/birthday upcoming [limit:1-50]` — Wyświetla listę najbliższych urodzin na serwerze z uwzględnieniem kolejności od bieżącego dnia.
- `/birthday delete` — Usuwa Twoją datę urodzin z bazy serwera.

---

## 📂 Zawartość folderu
- `module.lua` — Pełna logika wykonawcza modułu w piaskownicy Lua (mlua).
- `birthdays.lua` — Kopia zgodnościowa dla modułu.
- `config.json` — Domyślna konfiguracja startowa modułu.
- `README.md` — Niniejsza dokumentacja techniczna.

---

## ⚙️ Konfiguracja (`config.json`)
Moduł ładuje swoje parametry początkowe z pliku `config.json`. Administratorzy mogą nadpisywać te wartości per-serwer w bazie PostgreSQL oraz przez panel WWW.
- `announcement_channel_id` (opcjonalny) — ID kanału do ogłaszania życzeń urodzinowych.
- `birthday_role_id` (opcjonalny) — ID roli jubileuszowej nadawanej w dniu urodzin.

---

## 🚀 Rejestracja w `install.xml`
Wpis modułu w pliku instalatora `scripts/install.xml`:
```xml
<module id="birthdays" enabled="true" version="1.0.0">
    <name>Birthday Tracker</name>
    <description>Śledzenie urodzin członków serwera, życzenia i dedykowana ranga jubileuszowa.</description>
    <folder>birthdays</folder>
    <entry>module.lua</entry>
    <config>config.json</config>
    <author>Adrian Banach</author>
    <category>community</category>
</module>
```

---

## 🛡️ Bezpieczeństwo i audyt
Wszystkie operacje podlegają izolacji per-gildia (tenant isolation), walidacji formatu i daty, limitom zapytań (rate limiting `birthday`) oraz rejestracji w audycie platformy:
- `birthday_set` — Rejestrowane przy zapisie lub aktualizacji daty użytkownika.
- `birthday_delete` — Rejestrowane przy usunięciu wpisu.
