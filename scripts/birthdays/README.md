# Moduł: Birthday Tracker (`birthdays`)

Wersja: `1.0.0`  
Kategoria: `community`  
Autor: **Adrian Banach**  
Silnik: **rusty-discord-bot (Rust + Lua 5.4)**

---

## 📌 Opis
Śledzenie urodzin członków serwera, życzenia i dedykowana ranga jubileuszowa.

---

## 📂 Zawartość folderu
- `module.lua` — Pełna logika wykonawcza modułu w piaskownicy Lua (mlua).
- `config.json` — Domyślna konfiguracja startowa modułu.
- `README.md` — Niniejsza dokumentacja techniczna.

---

## ⚙️ Konfiguracja (`config.json`)
Moduł ładuje swoje parametry początkowe z pliku `config.json`. Administratorzy mogą nadpisywać te wartości per-serwer w bazie PostgreSQL oraz przez panel WWW.

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
Wszystkie krytyczne operacje podlegają walidacji uprawnień Discorda, limitom zapytań (rate limiting) oraz rejestracji w audycie platformy.
