# Moduł: Music (`music`)

Wersja: `1.0.0`  
Kategoria: `multimedia`  
Autor: **Adrian Banach**  
Silnik: **rusty-discord-bot (Rust + Lua 5.4)**

---

## 📌 Opis
Odtwarzacz audio bazujący na silniku Songbird z obsługą kolejki i streamingu.

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
<module id="music" enabled="true" version="1.0.0">
    <name>Music</name>
    <description>Odtwarzacz audio bazujący na silniku Songbird z obsługą kolejki i streamingu.</description>
    <folder>music</folder>
    <entry>module.lua</entry>
    <config>config.json</config>
    <author>Adrian Banach</author>
    <category>multimedia</category>
</module>
```

---

## 🛡️ Bezpieczeństwo i audyt
Wszystkie krytyczne operacje podlegają walidacji uprawnień Discorda, limitom zapytań (rate limiting) oraz rejestracji w audycie platformy.
