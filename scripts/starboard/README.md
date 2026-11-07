# Moduł: Starboard (`starboard`)

Wersja: `1.0.0`  
Kategoria: `community`  
Autor: **Adrian Banach**  
Silnik: **rusty-discord-bot (Rust + Lua 5.4)**

---

## 📌 Opis
Automatyczna tablica chwały: wyróżnianie najlepszych wiadomości oznaczonych gwiazdkami.

---

## 📂 Zawartość folderu
- `module.lua` — Pełna logika wykonawcza modułu w piaskownicy Lua (mlua).
- `config.json` — Domyślna konfiguracja startowa modułu.
- `README.md` — Niniejsza dokumentacja techniczna.

---

## ⚙️ Konfiguracja (`config.json` i komendy)
Moduł ładuje swoje parametry początkowe z pliku `config.json`. Administratorzy mogą nadpisywać te wartości per-serwer w bazie PostgreSQL oraz przez komendy slash:
- `/starboard config channel:#kanal threshold:5` — Konfiguracja kanału docelowego tablicy gwiazdek oraz wymaganego progu reakcji (1–25). Wymaga uprawnienia `MANAGE_GUILD` lub `ADMINISTRATOR`. Kanał musi być kanałem tekstowym bieżącej gildii, a bot musi posiadać w nim uprawnienia `VIEW_CHANNEL` i `SEND_MESSAGES`. Konfiguracja jest trwale zapisywana w per-guild module settings w storage z audytem `starboard_config_updated`.
- `/starboard stats` — Wyświetla aktualną konfigurację kanału i progu pobraną ze storage (`ctx.config`).

---

## 🚀 Rejestracja w `install.xml`
Wpis modułu w pliku instalatora `scripts/install.xml`:
```xml
<module id="starboard" enabled="true" version="1.0.0">
    <name>Starboard</name>
    <description>Automatyczna tablica chwały: wyróżnianie najlepszych wiadomości oznaczonych gwiazdkami.</description>
    <folder>starboard</folder>
    <entry>module.lua</entry>
    <config>config.json</config>
    <author>Adrian Banach</author>
    <category>community</category>
</module>
```

---

## 🛡️ Bezpieczeństwo i audyt
Wszystkie krytyczne operacje podlegają walidacji uprawnień Discorda, limitom zapytań (rate limiting) oraz rejestracji w audycie platformy.
