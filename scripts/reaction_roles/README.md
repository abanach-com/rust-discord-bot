# Moduł: Reaction Roles (`reaction_roles`)

Wersja: `1.0.0`  
Kategoria: `management`  
Autor: **Adrian Banach**  
Silnik: **rusty-discord-bot (Rust + Lua 5.4)**

---

## 📌 Opis
Interaktywny wybór ról poprzez reakcje pod wiadomościami na serwerze.

### Komendy
- `/reactionrole create message_id:<id> emoji:<emoji> role:<role>` — Tworzy mapowanie reakcji na wiadomość do roli. Wymaga uprawnienia `MANAGE_ROLES` oraz roli bota i administratora wyższej w hierarchii niż nadawana rola.
- `/reactionrole remove message_id:<id> emoji:<emoji>` — Usuwa przypisane mapowanie roli za reakcję.

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
<module id="reaction_roles" enabled="true" version="1.0.0">
    <name>Reaction Roles</name>
    <description>Interaktywny wybór ról poprzez reakcje pod wiadomościami na serwerze.</description>
    <folder>reaction_roles</folder>
    <entry>module.lua</entry>
    <config>config.json</config>
    <author>Adrian Banach</author>
    <category>management</category>
</module>
```

---

## 🛡️ Bezpieczeństwo i audyt
Wszystkie krytyczne operacje podlegają walidacji uprawnień Discorda, limitom zapytań (rate limiting) oraz rejestracji w audycie platformy.

### Ochrona pochodzenia ról i współdzielonych źródeł (Issue #286)
System Reaction Roles korzysta z trwałego ledgera grantów `reaction_role_grants`, który śledzi pochodzenie każdej roli przyznanej przez bota:
- **Ochrona ról pre-existing (nadanych ręcznie przez moderatora lub z innych źródeł)**: Jeżeli użytkownik posiadał rolę przed kliknięciem reakcji, usunięcie reakcji **nigdy** nie odbiera mu tej roli (`granted_by_bot = false`).
- **Współdzielone źródła ról (wiele bindingów do tej samej roli)**: Jeżeli ta sama rola jest skonfigurowana pod wieloma różnymi wiadomościami lub emoji, usunięcie jednej reakcji nie odbiera roli, dopóki użytkownik utrzymuje przynajmniej jedną inną aktywną reakcję dla tej samej roli.
- **Kaskadowość**: Usunięcie powiązania (`/reactionrole remove` lub REST delete) automatycznie kaskadowo czyści powiązane granty z ledgera.
