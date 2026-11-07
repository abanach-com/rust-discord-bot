# Moduł: AFK System (`afk`)

Wersja: `1.0.0`  
Kategoria: `utility`  
Autor: **Adrian Banach**  
Silnik: **rusty-discord-bot (Rust + Lua 5.4)**

---

## 📌 Opis
Zarządzanie statusem nieobecności AFK, automatyczne powiadomienia przy wzmiankach użytkowników oraz automatyczne wyłączanie AFK po wysłaniu nowej wiadomości. Trwały stan przechowywany jest w PostgreSQL i MemoryStore z pełną izolacją per-gildia.

---

## 📂 Komendy slash
- `/afk set [reason:string]` — Włącza status AFK z opcjonalnym powodem (maks. 256 znaków).
- `/afk clear` — Wyłącza Twój status AFK.
- `/afk status` — Sprawdza Twój obecny status AFK oraz czas jego ustawienia.

---

## ⚡ Obsługa zdarzeń
- `message_create`:
  - Jeśli autor wiadomości był oznaczony jako AFK oraz włączona jest opcja `auto_clear_on_message` (domyślnie `true`), status jest automatycznie wyłączany i bot wysyła powiadomienie powitalne.
  - Jeśli w treści wiadomości wspomniano użytkowników posiadających status AFK, bot informuje o ich nieobecności i powodzie bez pingowania (`allowed_mentions` wyczyszczone).

---

## ⚙️ Konfiguracja (`config.json`)
- `auto_clear_on_message` (`boolean`, domyślnie `true`) — Automatyczne zdejmowanie AFK po napisaniu wiadomości.
- `max_reason_length` (`integer`, min `10`, max `500`, domyślnie `256`) — Maksymalna długość powodu.
- `afk_prefix` (`string`, domyślnie `"[AFK] "`) — Prefiks dla powiadomień.

---

## 🛡️ Bezpieczeństwo i audyt
- Wszystkie operacje podlegają izolacji per-gildia (tenant isolation).
- Audytowane zdarzenia: `afk_set`, `afk_clear`, `afk_auto_clear`.
- Usunięcie danych użytkownika w ramach RODO (`/privacy/delete`) automatycznie czyści status AFK.
