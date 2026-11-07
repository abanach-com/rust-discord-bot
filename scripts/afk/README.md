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
  - Jeśli w treści wiadomości wspomniano użytkowników posiadających status AFK:
    - Agregacja: co najwyżej **1 zbiorcza wiadomość** na zdarzenie (maks. 5 unikalnych użytkowników, treść <= 2000 znaków).
    - Bezpieczeństwo Unicode (UTF-8): budowanie wiadomości respektuje budżet 2000 bajtów i znaków z bezpiecznym obcinaniem wyłącznie po pełnych Unicode scalar values (brak paniki przy wielobajtowych emoji); powiadomienia, które nie mieszczą się w budżecie, nie zużywają per-target cooldownu.
    - Per-target cooldown: powiadomienie o tej samej osobie na danym kanale nie częściej niż raz na 60 sekund.
    - Rate limit wyjściowy: maksymalnie 2 odpowiedzi na 30 sekund na kanał oraz 3 na 30 sekund na autora wiadomości.
    - Zero ping-loopów: powiadomienia wysyłane z wyczyszczonym `allowed_mentions` (bez pingowania).

---

## ⚙️ Konfiguracja (`config.json`)
- `auto_clear_on_message` (`boolean`, domyślnie `true`) — Automatyczne zdejmowanie AFK po napisaniu wiadomości.
- `max_reason_length` (`integer`, min `10`, max `500`, domyślnie `256`) — Maksymalna długość powodu.
- `afk_prefix` (`string`, domyślnie `"[AFK] "`) — Prefiks dla powiadomień.

---

## 🛡️ Bezpieczeństwo i audyt
- Wszystkie operacje podlegają izolacji per-gildia (tenant isolation).
- Audytowane zdarzenia: `afk_set`, `afk_clear`, `afk_auto_clear`, `afk_notice_throttled`.
- Metryki Prometheus: `afk_notifications_suppressed_total` (rejestruje zduszone powiadomienia floodu).
- Usunięcie danych użytkownika w ramach RODO (`/privacy/delete`) automatycznie czyści status AFK.
- Eksport danych użytkownika (`/privacy/export`) uwzględnia aktywny rekord AFK (`reason`, `set_at`).
