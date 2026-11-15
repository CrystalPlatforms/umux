# PRD: umux ↔ Klepai Integration

## Overview

Integracja spięta umux i Klepai w jeden lokalny dev stack: Klepai zarządza modelami i czatem, umux terminalem i agentami w nim. To nie jest osobny produkt ani feature z brandingiem — to po prostu połączenie umux↔Klepai. Całość darmowa, w obu OSS.

## Problem Statement

Bez integracji umux i Klepai to dwie osobne aplikacje: agent działający w Klepai nie ma dostępu do terminala i nie może uruchamiać komend na maszynie, a konwersacje z Claude Code żyją w formatach, których umux nie widzi. Ręczne przełączanie okien i kopiowanie nie obsługuje agentów. Integracja realizuje wizję jednego spójnego stacku.

## Users

| Typ użytkownika | Opis | Wolumen |
|---|---|---|
| Developer (primary) | Solo dev na macOS/Windows/Linux, używa jednocześnie umux i Klepai, pracuje z Claude Code/Codex | 1 (PO) na start, skaluje się z adopcją obu OSS |
| Agent CLI | Claude Code lub Codex działający w Klepai w trybie Agent — potrzebuje terminala i file systemu jako narzędzi | per sesja |

## Goals & Success Criteria

- [ ] Po połączeniu w umuxie istnieje nieusuwalny ręcznie workspace „Klepai", znikający po rozłączeniu.
- [ ] Każdy chat Klepai w trybie Agent ma swój tab w workspace Klepai, nazwany nazwą konwersacji; nazwa tabu jest niezmienna z umuxa (zmiana nazwy konwersacji w Klepai zmienia nazwę tabu).
- [ ] „Run in umux" w Klepai wykonuje komendę w workspace Klepai, w tabie przypisanym do tego chatu, automatycznie w katalogu, w którym był chat.
- [ ] „Open session in umux" odtwarza konwersację Claude Code/Codex w workspace Klepai w umuxie; server models wykluczone.
- [ ] Konwersacje Klepai z Claude Code zapisywane lokalnie w formacie Claude Code (Codex natywnie już to robi).
- [ ] Agent w Klepai pracuje w swoim tabie w umuxie: uruchamia tam wszystkie swoje komendy i tam manewruje (cd, nawigacja po katalogach, praca z plikami), w tym background i localhost; user widzi output na żywo, agent dostaje wynik po zakończeniu komendy.
- [ ] Agent w Klepai steruje umuxem przez umux Live CLI, prezentowane w UI Klepai jako Tool Calling.
- [ ] umux CLI nie zawiera żadnych podkomend związanych z Klepai.
- [ ] Wszystkie testy zielone; HITL na maszynie PO na trzech platformach.

## User Stories

1. Jako developer chcę, żeby połączenie z Klepai tworzyło w umuxie workspace „Klepai", którego nie usunę przypadkiem, a który znika po rozłączeniu, żeby stan odzwierciedlał rzeczywiste połączenie.
2. Jako developer chcę, żeby każdy chat w trybie Agent miał swój tab w workspace Klepai z nazwą konwersacji, żeby od razu widzieć, który terminal należy do której rozmowy.
3. Jako developer chcę przycisk „Run in umux" przy okienku komend w Klepai, żeby komenda wykonała się w właściwym tabie i katalogu w umuxie.
4. Jako developer chcę „Open session in umux" obok Rename/Delete conversation, żeby wrócić do konwersacji Claude Code/Codex w terminalu z pełnym kontekstem.
5. Jako developer chcę, żeby konwersacje z Claude Code zapisywały się w formacie Claude Code, żeby umux i Claude Code czytały je natywnie.
6. Jako agent chcę własny tab w umuxie, w którym uruchamiam wszystkie swoje komendy i w którym manewruję (cd, nawigacja, praca z plikami), również background i localhost, i dostaję output po zakończeniu, żeby działać na maszynie usera widocznie i w jednym miejscu.
7. Jako agent chcę sterować umuxem przez Tool Calling w Klepai (pod spodem umux Live CLI), żeby zarządzać pane'ami, tabami i workspace'ami z rozmowy.

## Scope

### In scope

- Cykl życia workspace „Klepai" w umuxie (tworzenie po połączeniu, nieusuwalność ręczna, usunięcie po rozłączeniu).
- Mapowanie chatów w trybie Agent na taby w workspace Klepai; nazwa tabu = nazwa konwersacji, mirror z Klepai, niemodyfikowalna z umuxa.
- „Run in umux" w UI Klepai — wykonanie w przypisanym tabie i katalogu chatu.
- „Open session in umux" w UI Klepai (obok Rename/Delete conversation) — odtworzenie konwersacji w Claude Code/Codex.
- Zapis konwersacji Claude Code w natywnym formacie Claude Code.
- Klepai MCP: umux Live CLI wystawiony jako narzędzia MCP dla agentów Klepai (Tool Calling w UI).
- Zasada izolacji agenta: agent w trybie Agent uruchamia wszystkie swoje komendy wyłącznie w swoim tabie w umuxie i tam manewruje po file systemie (cd, nawigacja, praca z plikami); komendy agenta nie wyciekają do innych tabów ani workspace'ów.

### Out of scope

- Jakiekolwiek podkomendy Klepai w umux CLI — umux CLI nie ma wspólnego z Klepai.
- Branding „Interlink" — nie ma nazwy produktu, to po prostu połączenie umux↔Klepai.
- Taby dla chatów spoza trybu Agent.
- Server models w „Open session in umux".
- Integracje z innymi CLI-ami niż Claude Code i Codex.
- Protokół/standard dla przyszłych produktów — wykrystalizuje się z implementacji, nie projektuje się z góry.
- Synchronizacja między maszynami.
- Fallback bez uruchomionych obu aplikacji — integracja wymaga, żeby obie żyły.

## System Components

- umux Live CLI — fundament po stronie umuxa: live CLI/socket, przez który Klepai (MCP) steruje umuxem.
- Klepai runtime — Tauri v2, Rust core; wystawia powierzchnię sterowania (chats, konwersacje, agenty) na żywo.
- Workspace „Klepai" w umuxie — specjalny typ workspace'u o ograniczonym cyklu życia, z tabami per chat Agent.
- Klepai MCP — server MCP w Klepai, narzędzia mapowane na komendy umux Live CLI; w UI prezentowane jako Tool Calling.
- Kanał międzyprocesowy — komunikacja umux↔Klepai; konkretny transport (socket vs CLI) otwarte pytanie poniżej.
- Warstwa zapisu sesji Claude Code — eksport/konwersja konwersacji do natywnego formatu Claude Code.

## Implementation Decisions

| Obszar | Decyzja | Uzasadnienie |
|---|---|---|
| Model produktowy | Cała integracja darmowa, w obu OSS | To strategia spójnego stacku, nie monetyzacja |
| Nazwa | Brak nazwy produktu — „połączenie umux↔Klepai" | Decyzja PO: bez brandingu Interlink |
| Zakres MVP | Wszystkie funkcje w jednym release | Decyzja PO: bez cięcia |
| Timing | Po umux v3.0.0 | Zależy od umux Live CLI w pełnej parności |
| Podejście | Integracja najpierw, protokół później | Unikanie nadprojektowania standardu bez drugiego konsumenta |
| Sterowanie z umuxa | Tylko agentowe Tool Calling z Klepai; umux CLI bez śladu Klepai | Decyzja PO |
| Taby | Tylko chaty w trybie Agent dostają taby | Decyzja PO |
| Stack | Bez zmian: Tauri v2, Rust core, TS frontend po obu stronach | Oba produkty już na tym samym fundamencie |
| Warunek działania | Obie apki żywe jednocześnie | Workspace znika po rozłączeniu |
| Język | UI i PRD po angielsku w repo, rozmowa po polsku | Zasada językowa z obu repo |

## Validation Strategy

- Testy jednostkowe i integracyjne po obu stronach (Rust + frontend), zgodnie z konwencją obu repo.
- Test cyklu życia workspace: tworzenie po połączeniu, próba ręcznego usunięcia, rozłączenie, zniknięcie.
- Test tabów: nowy chat Agent → nowy tab o nazwie konwersacji; rename w Klepai → rename tabu; próba rename z umuxa → niemożliwa.
- Test E2E: „Run in umux" → komenda w właściwym tabie i katalogu → output w pane.
- Test „Open session in umux" dla Claude Code i Codex; server models — brak przycisku.
- Test agenta: wszystkie komendy agenta lądują w jego tabie (izolacja od innych tabów), manewrowanie cd/nawigacja działa w ramach tabu, background komenda + localhost, asynchroniczny zwrot outputu.
- Test Tool Calling: agent steruje umuxem, w UI widoczne jako wywołania narzędzi.
- HITL: Adam, na własnej maszynie, wszystkie trzy platformy, z prawdziwymi sesjami Claude Code i Codex.

## Open Questions

- [ ] Transport międzyprocesowy umux↔Klepai (socket vs CLI-to-CLI) — do rozstrzygnięcia w planie implementacji.
- [ ] Semantyka rozłączenia: los konwersacji i plików przy killu Klepai w trakcie sesji.
- [ ] Miejsce PRD: repo umux (`plans/`), repo Klepai (`plans/`), czy wspólny plik w obu.
- [ ] Numer wersji/slot w roadmapie umux po v3.0.0.

## References

- Discovery summary: sesja Interlink (potwierdzona przez PO, zrewidowana przed blueprintem).
- umux master PRD: `plans/umux-prd.md` (repo umux).
- Klepai PRD: `PRD.md` (repo CRS-SHAIM).