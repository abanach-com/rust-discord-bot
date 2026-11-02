# Moduł: Fun (`fun`)

Wersja: `1.0.0`  
Kategoria: `fun`  
Autor: **Adrian Banach**  
Silnik: **rusty-discord-bot (Rust + Lua 5.4)**

---

## 📌 Opis
Zestaw rozrywkowych komend: rzuty kośćmi, 8ball, losowania i zabawy tekstowe.

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
<module id="fun" enabled="true" version="1.0.0">
    <name>Fun</name>
    <description>Zestaw rozrywkowych komend: rzuty kośćmi, 8ball, losowania i zabawy tekstowe.</description>
    <folder>fun</folder>
    <entry>module.lua</entry>
    <config>config.json</config>
    <author>Adrian Banach</author>
    <category>fun</category>
</module>
```

---

## 🛡️ Bezpieczeństwo i audyt
Wszystkie krytyczne operacje podlegają walidacji uprawnień Discorda, limitom zapytań (rate limiting) oraz rejestracji w audycie platformy.
